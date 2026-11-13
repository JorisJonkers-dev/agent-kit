//! What a gate reads from a shell command line: the words of each simple command, and the commits, pushes, merges and
//! PR creations the line runs, with the directory each runs in. It reads the text only; nothing is expanded or run.
//! The node runner's `domain/shell-command.ts`, kept to its reading of the text: a quote left open runs to the end of
//! the line, and a backslash with nothing after it adds an empty word.

use model_gateway::text::{js_trim, resolve};

/// The words of each simple command in a shell command line, as far as a gate needs them: quotes are removed, command
/// substitutions (`$(...)` and backticks) and here-document bodies are skipped, redirections and their targets are
/// dropped, and `&&`, `||`, `;`, `|`, `&`, parentheses and new lines end a command.
pub fn simple_commands(line: &str) -> Vec<Vec<String>> {
    let text: Vec<char> = line.chars().collect();
    let mut scanner = Scanner { text: &text, i: 0, done: Vec::new(), words: Vec::new(), word: None, skip_next_word: false, heredocs: Vec::new() };
    while scanner.i < text.len() {
        scanner.step();
    }
    scanner.end_command();
    scanner.done
}

/// A here-document opened on the current line: its delimiter, and whether `<<-` strips leading tabs.
#[derive(Clone)]
struct Heredoc {
    delimiter: String,
    strip_tabs: bool,
}

struct Scanner<'a> {
    text: &'a [char],
    i: usize,
    done: Vec<Vec<String>>,
    words: Vec<String>,
    word: Option<String>,
    skip_next_word: bool,
    /// Here-documents whose bodies start after the current line.
    heredocs: Vec<Heredoc>,
}

/// Whether `text` holds `needle` at `at`.
fn starts_with(text: &[char], at: usize, needle: &str) -> bool {
    let needle: Vec<char> = needle.chars().collect();
    text.get(at..at + needle.len()) == Some(&needle[..])
}

/// `set.includes(text.charAt(at))`: past the end `charAt` is the empty string, which every string includes.
fn includes_char_at(set: &str, text: &[char], at: usize) -> bool {
    text.get(at).is_none_or(|c| set.contains(*c))
}

/// `text.charAt(at)` as text: empty past the end.
fn char_at(text: &[char], at: usize) -> String {
    text.get(at).map(char::to_string).unwrap_or_default()
}

/// `text.indexOf(c, from)`.
fn index_of(text: &[char], c: char, from: usize) -> Option<usize> {
    text.get(from..).and_then(|rest| rest.iter().position(|other| *other == c)).map(|at| at + from)
}

impl Scanner<'_> {
    fn step(&mut self) {
        let c = self.text[self.i];
        if c == '\n' {
            self.end_command();
            self.i += 1;
            let heredocs = std::mem::take(&mut self.heredocs);
            self.i = skip_heredoc_bodies(self.text, self.i, &heredocs);
        } else if c == ' ' || c == '\t' {
            self.end_word();
            self.i += 1;
        } else if ";&|()".contains(c) {
            self.end_command();
            self.i += 1;
        } else if c == '<' || c == '>' {
            self.redirection();
        } else if c == '#' && self.word.is_none() {
            while self.i < self.text.len() && self.text[self.i] != '\n' {
                self.i += 1;
            }
        } else if c == '\'' {
            let stop = index_of(self.text, '\'', self.i + 1).unwrap_or(self.text.len());
            let quoted: String = self.text[self.i + 1..stop].iter().collect();
            self.append(&quoted);
            self.i = stop + 1;
        } else if c == '"' {
            self.i = self.double_quoted(self.i + 1);
        } else if c == '\\' {
            if self.text.get(self.i + 1) != Some(&'\n') {
                self.append(&char_at(self.text, self.i + 1));
            }
            self.i += 2;
        } else if c == '`' || starts_with(self.text, self.i, "$(") {
            self.append("");
            self.i = skip_substitution(self.text, self.i);
        } else {
            self.append(&c.to_string());
            self.i += 1;
        }
    }

    /// Reads a double-quoted string from just after its opening quote; returns the index after its closing one.
    fn double_quoted(&mut self, start: usize) -> usize {
        let mut i = start;
        self.append("");
        while i < self.text.len() {
            let c = self.text[i];
            if c == '"' {
                return i + 1;
            }
            if c == '\\' && includes_char_at("\"\\$`\n", self.text, i + 1) {
                if self.text.get(i + 1) != Some(&'\n') {
                    self.append(&char_at(self.text, i + 1));
                }
                i += 2;
            } else if c == '`' || starts_with(self.text, i, "$(") {
                i = skip_substitution(self.text, i);
            } else {
                self.append(&c.to_string());
                i += 1;
            }
        }
        i
    }

    /// A redirection: `<<` opens a here-document; any other drops its target word. A file descriptor before it is
    /// dropped too.
    fn redirection(&mut self) {
        if self.word.as_deref().is_some_and(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_digit())) {
            self.word = None;
        }
        self.end_word();
        if starts_with(self.text, self.i, "<<") && !starts_with(self.text, self.i, "<<<") {
            self.i += 2;
            let strip_tabs = self.text.get(self.i) == Some(&'-');
            if strip_tabs {
                self.i += 1;
            }
            let (delimiter, end) = read_delimiter(self.text, self.i);
            self.heredocs.push(Heredoc { delimiter, strip_tabs });
            self.i = end;
            return;
        }
        while self.i < self.text.len() && "<>&|".contains(self.text[self.i]) {
            self.i += 1;
        }
        // `2>&1` names a descriptor, not a file; either way the word is not an argument.
        self.skip_next_word = true;
    }

    fn append(&mut self, text: &str) {
        self.word.get_or_insert_with(String::new).push_str(text);
    }

    fn end_word(&mut self) {
        let Some(word) = self.word.take() else { return };
        if self.skip_next_word {
            self.skip_next_word = false;
        } else {
            self.words.push(word);
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        self.skip_next_word = false;
        let words = std::mem::take(&mut self.words);
        if !words.is_empty() {
            self.done.push(words);
        }
    }
}

/// The delimiter word of a here-document, quotes removed, and where it ends.
fn read_delimiter(text: &[char], start: usize) -> (String, usize) {
    let mut i = start;
    while matches!(text.get(i), Some(' ' | '\t')) {
        i += 1;
    }
    let mut delimiter = String::new();
    while i < text.len() && !" \t\n;&|()<>".contains(text[i]) {
        if !matches!(text[i], '\'' | '"' | '\\') {
            delimiter.push(text[i]);
        }
        i += 1;
    }
    (delimiter, i)
}

/// Skips the bodies of the here-documents opened on the line before `start`, each up to its delimiter line.
fn skip_heredoc_bodies(text: &[char], start: usize, heredocs: &[Heredoc]) -> usize {
    let mut i = start;
    for heredoc in heredocs {
        while i < text.len() {
            let stop = index_of(text, '\n', i).unwrap_or(text.len());
            let line: String = text[i..stop].iter().collect();
            i = stop + 1;
            let line = if heredoc.strip_tabs { line.trim_start_matches('\t') } else { line.as_str() };
            if js_trim(line) == heredoc.delimiter {
                break;
            }
        }
    }
    i.min(text.len())
}

/// Skips a command substitution from its `$(` or backtick; returns the index after its end. Quotes, nested
/// substitutions and here-documents inside it are skipped whole, so an apostrophe or a parenthesis in a commit message
/// written through `$(cat <<'EOF' ... EOF)` ends nothing.
fn skip_substitution(text: &[char], start: usize) -> usize {
    if text[start] == '`' {
        let mut i = start + 1;
        while i < text.len() && text[i] != '`' {
            i += if text[i] == '\\' { 2 } else { 1 };
        }
        return i + 1;
    }
    let mut i = start + 2;
    let mut depth = 1;
    let mut heredocs: Vec<Heredoc> = Vec::new();
    while i < text.len() && depth > 0 {
        let c = text[i];
        if c == '\n' {
            i = skip_heredoc_bodies(text, i + 1, &std::mem::take(&mut heredocs));
        } else if starts_with(text, i, "<<") && !starts_with(text, i, "<<<") {
            let strip_tabs = text.get(i + 2) == Some(&'-');
            let (delimiter, end) = read_delimiter(text, i + if strip_tabs { 3 } else { 2 });
            heredocs.push(Heredoc { delimiter, strip_tabs });
            i = end;
        } else if c == '\'' {
            i = index_of(text, '\'', i + 1).map_or(text.len(), |end| end + 1);
        } else if c == '"' {
            i += 1;
            while i < text.len() && text[i] != '"' {
                if text[i] == '\\' {
                    i += 2;
                } else if starts_with(text, i, "$(") || text[i] == '`' {
                    i = skip_substitution(text, i);
                } else {
                    i += 1;
                }
            }
            i += 1;
        } else if c == '\\' {
            i += 2;
        } else if c == '`' || starts_with(text, i, "$(") {
            i = skip_substitution(text, i);
        } else {
            if c == '(' {
                depth += 1;
            }
            if c == ')' {
                depth -= 1;
            }
            i += 1;
        }
    }
    i
}

/// A pushed ref: where it comes from (`None` for the current branch's commit) and the branch it updates (`None` for
/// the current branch).
#[derive(Clone, Debug, PartialEq)]
pub struct PushTarget {
    pub source: Option<String>,
    pub branch: Option<String>,
}

/// How much more than the index a commit may record: nothing, tracked files beyond the index (`-a`, paths,
/// `--include`, a `git add` earlier in the line), or untracked files too (an earlier `git add`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Widened {
    None,
    Tracked,
    Untracked,
}

impl Widened {
    pub fn name(self) -> &'static str {
        match self {
            Widened::None => "none",
            Widened::Tracked => "tracked",
            Widened::Untracked => "untracked",
        }
    }
}

/// The branches a push updates.
#[derive(Clone, Debug, PartialEq)]
pub enum PushTargets {
    /// `--all`, `--branches` or `--mirror`.
    All,
    Some(Vec<PushTarget>),
}

/// One command a gate stands before, and the directory it runs in (`None` when the command line leaves that unclear).
#[derive(Clone, Debug, PartialEq)]
pub enum GatedInvocation {
    Commit { cwd: Option<String>, widened: Widened },
    Push { cwd: Option<String>, targets: PushTargets, head_moves: bool },
    /// A `git merge` or `git pull` that may record a merge commit without any `git commit`: the revisions it merges,
    /// and whether git may fast-forward instead (one revision and no `--no-ff`).
    Merge { cwd: Option<String>, revisions: Vec<String>, fast_forward: bool, head_moves: bool },
    /// A `gh pr create`, with the branch `--head` names (`None` for the current branch).
    PrCreate { cwd: Option<String>, head: Option<String>, head_moves: bool },
}

impl GatedInvocation {
    pub fn cwd(&self) -> Option<&str> {
        match self {
            GatedInvocation::Commit { cwd, .. } | GatedInvocation::Push { cwd, .. } | GatedInvocation::Merge { cwd, .. } | GatedInvocation::PrCreate { cwd, .. } => cwd.as_deref(),
        }
    }
}

/// Git commands that may change the index, so a later commit in the same line records something else.
const INDEX_CHANGING: [&str; 19] = [
    "add", "am", "apply", "checkout", "cherry-pick", "commit", "merge", "mv", "pull", "read-tree", "rebase", "reset", "restore", "revert", "rm", "stage", "stash", "switch", "update-index",
];
/// Git commands that may add untracked files to the index.
const ADDING: [&str; 2] = ["add", "stage"];
/// Git commands that may move HEAD, so a later push or PR sends a commit the gate cannot see yet.
const HEAD_MOVING: [&str; 10] = ["am", "checkout", "cherry-pick", "commit", "merge", "pull", "rebase", "reset", "revert", "switch"];
/// Words that run the command after them.
const WRAPPERS: [&str; 6] = ["builtin", "command", "env", "exec", "nohup", "time"];
/// Git's global options that take the next word as their value.
const GIT_VALUED: [&str; 8] = ["-C", "-c", "--git-dir", "--work-tree", "--namespace", "--super-prefix", "--config-env", "--exec-path"];

/// What a `git merge` or `git pull` comes to.
enum MergeOf {
    /// It records no merge commit (it rebases, fast-forwards only, squashes or stops before committing).
    None,
    /// `git merge --continue`, which commits the merge staged in the index as `git commit` does.
    Continue,
    Merges { revisions: Vec<String>, fast_forward: bool },
}

/// The commits, pushes, merges and PR creations a command line runs, in order. `cd` and `git -C` move the directory a
/// later command runs in; a `cd` to a place the text alone does not name (`cd -`, a variable) leaves it unclear.
pub fn gated_invocations(line: &str, cwd: &str, home: Option<&str>) -> Vec<GatedInvocation> {
    let mut found = Vec::new();
    let mut dir = Some(cwd.to_string());
    let mut index_changed = Widened::None;
    let mut head_moved = false;
    for words in simple_commands(line) {
        let command = unwrap(&words);
        let Some((program, args)) = command.split_first() else { continue };
        if program == "cd" {
            dir = change_directory(dir.as_deref(), args, home);
        } else if program == "git" {
            let (subcommand, args, cwd) = git_subcommand(args, dir.as_deref(), home);
            let merge = match subcommand {
                Some(name @ ("merge" | "pull")) => merge_of(name, args),
                _ => MergeOf::None,
            };
            let commit = subcommand == Some("commit") && !args.iter().any(|arg| arg == "--dry-run");
            match merge {
                MergeOf::Continue => found.push(GatedInvocation::Commit { cwd, widened: index_changed }),
                _ if commit => found.push(GatedInvocation::Commit { cwd, widened: index_changed.max(commit_widening(args)) }),
                MergeOf::Merges { revisions, fast_forward } => found.push(GatedInvocation::Merge { cwd, fast_forward, head_moves: head_moved, revisions }),
                MergeOf::None if subcommand == Some("push") && !args.iter().any(|arg| arg == "--dry-run" || arg == "-n") => {
                    if let Some(targets) = push_targets(args) {
                        found.push(GatedInvocation::Push { cwd, head_moves: head_moved, targets });
                    }
                }
                MergeOf::None => {}
            }
            if let Some(name) = subcommand {
                if INDEX_CHANGING.contains(&name) {
                    index_changed = index_changed.max(if ADDING.contains(&name) { Widened::Untracked } else { Widened::Tracked });
                }
                if HEAD_MOVING.contains(&name) {
                    head_moved = true;
                }
            }
        } else if program == "gh" && is_pr_create(args) {
            found.push(GatedInvocation::PrCreate { cwd: dir.clone(), head: option_value(args, &["--head", "-H"]), head_moves: head_moved });
        }
    }
    found
}

/// Whether a word assigns a variable: `NAME=...`.
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else { return false };
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Drops leading variable assignments and wrappers such as `env` or `command`, with their options.
fn unwrap(words: &[String]) -> &[String] {
    let mut i = 0;
    loop {
        let Some(word) = words.get(i) else { return &[] };
        if is_assignment(word) {
            i += 1;
        } else if WRAPPERS.contains(&word.as_str()) {
            i += 1;
            while words.get(i).is_some_and(|word| word.starts_with('-')) {
                i += 1;
            }
        } else {
            return &words[i..];
        }
    }
}

fn change_directory(dir: Option<&str>, args: &[String], home: Option<&str>) -> Option<String> {
    let dir = dir?;
    match args.first() {
        None => home.map(str::to_string),
        Some(target) => within(Some(dir), target, home),
    }
}

/// `path` from `dir`, `~` expanded; `None` when the text alone does not say where it is.
fn within(dir: Option<&str>, path: &str, home: Option<&str>) -> Option<String> {
    let dir = dir?;
    if path == "-" || path.contains('$') {
        return None;
    }
    if path == "~" || path.starts_with("~/") {
        return home.map(|home| resolve(home, path.get(2..).unwrap_or_default()));
    }
    // An absolute path is taken as written, unnormalised, as `isAbsolute(path) ? path : resolve(dir, path)` takes it.
    Some(if path.starts_with('/') { path.to_string() } else { resolve(dir, path) })
}

/// The git subcommand after git's global options, its arguments, and the directory `-C` leaves it in.
fn git_subcommand<'a>(args: &'a [String], dir: Option<&str>, home: Option<&str>) -> (Option<&'a str>, &'a [String], Option<String>) {
    let mut cwd = dir.map(str::to_string);
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') {
        let option = args[i].as_str();
        if option == "-C" {
            cwd = within(cwd.as_deref(), args.get(i + 1).map_or("", String::as_str), home);
        }
        i += if GIT_VALUED.contains(&option) { 2 } else { 1 };
    }
    (args.get(i).map(String::as_str), args.get(i + 1..).unwrap_or_default(), cwd)
}

/// `git commit` options whose value is the next word, or the rest of a cluster such as `-mmessage`.
const COMMIT_VALUED_SHORT: [char; 6] = ['m', 'F', 'C', 'c', 't', 'S'];
const COMMIT_VALUED_LONG: [&str; 12] = [
    "--message", "--file", "--reuse-message", "--reedit-message", "--fixup", "--squash", "--author", "--date", "--template", "--cleanup", "--trailer", "--pathspec-from-file",
];
/// `git commit` options that commit files beyond what the index holds.
const COMMIT_WIDENING_SHORT: [char; 4] = ['a', 'i', 'o', 'p'];
const COMMIT_WIDENING_LONG: [&str; 6] = ["--all", "--include", "--only", "--patch", "--interactive", "--pathspec-from-file"];

/// Whether a commit's own options or paths make it record more than the index holds.
fn commit_widening(args: &[String]) -> Widened {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            return if i + 1 < args.len() { Widened::Tracked } else { Widened::None };
        }
        if arg.starts_with("--") {
            let name = arg.split('=').next().unwrap_or(arg);
            if COMMIT_WIDENING_LONG.contains(&name) {
                return Widened::Tracked;
            }
            if COMMIT_VALUED_LONG.contains(&arg) {
                i += 1;
            }
        } else if arg.starts_with('-') && arg.chars().count() > 1 {
            let flags: Vec<char> = arg.chars().collect();
            for (j, flag) in flags.iter().enumerate().skip(1) {
                if COMMIT_WIDENING_SHORT.contains(flag) {
                    return Widened::Tracked;
                }
                if COMMIT_VALUED_SHORT.contains(flag) {
                    // `-S` takes its key id attached or not at all.
                    if j == flags.len() - 1 && *flag != 'S' {
                        i += 1;
                    }
                    break;
                }
            }
        } else {
            // A path: the commit records that file as it is on disk.
            return Widened::Tracked;
        }
        i += 1;
    }
    Widened::None
}

const PUSH_VALUED: [&str; 5] = ["--repo", "-o", "--push-option", "--receive-pack", "--exec"];

/// The branches a push updates; `None` for a push of tags only or a deletion, which sends no branch's commits.
fn push_targets(args: &[String]) -> Option<PushTargets> {
    let mut positional: Vec<&str> = Vec::new();
    let mut tags = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            positional.extend(args[i + 1..].iter().map(String::as_str));
            break;
        }
        if matches!(arg, "--all" | "--branches" | "--mirror") {
            return Some(PushTargets::All);
        }
        if matches!(arg, "--delete" | "-d") {
            return None;
        }
        if arg == "--tags" {
            tags = true;
        }
        if PUSH_VALUED.contains(&arg) {
            i += 1;
        } else if !arg.starts_with('-') {
            positional.push(arg);
        }
        i += 1;
    }
    let refspecs = positional.get(1..).unwrap_or_default();
    if refspecs.is_empty() {
        return if tags { None } else { Some(PushTargets::Some(vec![PushTarget { branch: None, source: None }])) };
    }
    let targets: Vec<PushTarget> = refspecs.iter().filter_map(|refspec| push_target(refspec)).collect();
    (!targets.is_empty()).then_some(PushTargets::Some(targets))
}

/// What one refspec pushes; `None` for a tag or an empty source.
fn push_target(refspec: &str) -> Option<PushTarget> {
    let refspec = refspec.strip_prefix('+').unwrap_or(refspec);
    let mut parts = refspec.split(':');
    let raw_source = parts.next().unwrap_or_default();
    let raw_destination = parts.next();
    if raw_source.is_empty() || raw_source.starts_with("refs/tags/") || raw_destination.unwrap_or_default().starts_with("refs/tags/") {
        return None;
    }
    let source = (!matches!(raw_source, "HEAD" | "@")).then(|| raw_source.to_string());
    let destination = raw_destination.map(str::to_string).or_else(|| source.clone());
    Some(PushTarget { branch: destination.map(|branch| branch.strip_prefix("refs/heads/").map_or(branch.clone(), str::to_string)), source })
}

/// `git merge` options whose value is the next word, or the rest of a cluster such as `-mmessage`.
const MERGE_VALUED_SHORT: [char; 4] = ['m', 'F', 's', 'X'];
const MERGE_VALUED_LONG: [&str; 6] = ["--message", "--file", "--strategy", "--strategy-option", "--into-name", "--cleanup"];
/// `git pull` options whose value is the next word.
const PULL_VALUED_SHORT: [char; 4] = ['s', 'X', 'j', 'o'];
const PULL_VALUED_LONG: [&str; 11] = [
    "--strategy", "--strategy-option", "--depth", "--deepen", "--shallow-since", "--shallow-exclude", "--upload-pack", "--server-option", "--jobs", "--negotiation-tip", "--refmap",
];
/// Options after which the command records no merge commit: it fast-forwards only, squashes, stops before committing,
/// or ends a merge without one.
const NO_MERGE_COMMIT: [&str; 5] = ["--ff-only", "--squash", "--no-commit", "--abort", "--quit"];

/// What a `git merge` or `git pull` merges, when it may record a merge commit.
fn merge_of(subcommand: &str, args: &[String]) -> MergeOf {
    let merge = subcommand == "merge";
    let (valued_short, valued_long): (&[char], &[&str]) = if merge { (&MERGE_VALUED_SHORT, &MERGE_VALUED_LONG) } else { (&PULL_VALUED_SHORT, &PULL_VALUED_LONG) };
    let mut positional: Vec<String> = Vec::new();
    let mut commits = true;
    let mut fast_forward = true;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let name = arg.split('=').next().unwrap_or(arg);
        if arg == "--" {
            positional.extend(args[i + 1..].iter().cloned());
            break;
        }
        if merge && arg == "--continue" {
            return MergeOf::Continue;
        }
        if NO_MERGE_COMMIT.contains(&arg) {
            commits = false;
        } else if arg == "--commit" || arg == "--no-squash" {
            commits = true;
        } else if arg == "--no-ff" {
            fast_forward = false;
        } else if arg == "--ff" {
            fast_forward = true;
        } else if !merge && (arg == "-r" || name == "--rebase") {
            commits = arg == "--rebase=false";
        } else if !merge && arg == "--no-rebase" {
            commits = true;
        } else if arg.starts_with("--") {
            if valued_long.contains(&arg) {
                i += 1;
            }
        } else if arg.starts_with('-') && arg.chars().count() > 1 {
            // A valued flag last in its cluster takes the next word; earlier, the rest of the cluster is its value.
            let flags: Vec<char> = arg.chars().collect();
            let j = (1..flags.len()).find(|j| valued_short.contains(&flags[*j])).unwrap_or(flags.len());
            if j == flags.len() - 1 {
                i += 1;
            }
        } else {
            positional.push(arg.to_string());
        }
        i += 1;
    }
    if !commits {
        return MergeOf::None;
    }
    let revisions = if merge { positional } else { pulled_branches(&positional) };
    let merged = if revisions.is_empty() { vec!["@{upstream}".to_string()] } else { revisions };
    MergeOf::Merges { fast_forward: fast_forward && merged.len() == 1, revisions: merged }
}

/// The remote-tracking branch of each branch `git pull <remote> <refspec>...` fetches; none when it names no refspec.
fn pulled_branches(positional: &[String]) -> Vec<String> {
    let Some((remote, refspecs)) = positional.split_first() else { return Vec::new() };
    refspecs
        .iter()
        .map(|refspec| {
            let source = refspec.strip_prefix('+').unwrap_or(refspec).split(':').next().unwrap_or_default();
            format!("{remote}/{}", source.strip_prefix("refs/heads/").unwrap_or(source))
        })
        .collect()
}

/// Whether `gh` arguments are `pr create`, after any flag.
fn is_pr_create(args: &[String]) -> bool {
    let words: Vec<&str> = args.iter().map(String::as_str).filter(|arg| !arg.starts_with('-')).collect();
    words.first() == Some(&"pr") && words.get(1) == Some(&"create")
}

/// The value of the first of `names`, as `--name value` or `--name=value`.
fn option_value(args: &[String], names: &[&str]) -> Option<String> {
    for (i, arg) in args.iter().enumerate() {
        if names.contains(&arg.as_str()) {
            return args.get(i + 1).cloned();
        }
        if let Some(name) = names.iter().find(|name| arg.starts_with(&format!("{name}="))) {
            return Some(arg[name.len() + 1..].to_string());
        }
    }
    None
}
