//! What the runner asks git: a path's remotes, its repository root and its
//! common directory. The words of a failure are the node adapter's.

use model_gateway::adapters::fs::fail;
use model_gateway::ports::GitRemote;
use model_gateway::text::js_trim;
use std::collections::HashMap;
use std::process::{Command, Stdio};

pub trait Git: Send + Sync {
    fn remotes(&self, cwd: &str) -> Result<Vec<GitRemote>, String>;
    fn common_dir(&self, cwd: &str) -> Result<String, String>;
    fn root(&self, cwd: &str) -> Result<String, String>;
    /// The commit a revision names; an `Err` when it names none.
    fn resolve_commit(&self, cwd: &str, revision: &str) -> Result<String, String>;
    /// The last commit that changed the path and its committer date (ISO 8601); `None` when no commit touched it.
    fn last_change(&self, cwd: &str, path: &str) -> Result<Option<LastChange>, String>;
    /// Every commit reachable from `revision` that changed the path, newest first: its id and committer date only, never an author or a message.
    fn file_commits(&self, cwd: &str, revision: &str, path: &str) -> Result<Vec<LastChange>, String>;
    /// One file's contents at a commit.
    fn file_at(&self, cwd: &str, commit: &str, path: &str) -> Result<String, String>;
    /// The regular files of a commit's tree, without symbolic links and submodules, as paths from the repository root.
    fn tree_files(&self, cwd: &str, commit: &str) -> Result<Vec<String>, String>;
    /// The files the working tree holds that git neither tracks nor ignores.
    fn untracked_paths(&self, cwd: &str) -> Result<Vec<String>, String>;
    /// The best common ancestor of two commits.
    fn merge_base(&self, cwd: &str, a: &str, b: &str) -> Result<String, String>;
    /// The unified diff from one commit to another, with no external diff driver or text conversion.
    fn diff_commits(&self, cwd: &str, base: &str, head: &str) -> Result<String, String>;
    /// Every path the diff from one commit to another adds, changes or deletes, a rename as both of its paths.
    fn changed_paths(&self, cwd: &str, base: &str, head: &str) -> Result<Vec<String>, String>;
    /// Every staged entry as `git ls-files --stage -z` prints it; reads the index and writes nothing.
    fn staged_entries(&self, cwd: &str) -> Result<String, String>;
    /// The id of the tree a revision's commit holds; an `Err` when the revision names no commit.
    fn tree_of(&self, cwd: &str, revision: &str) -> Result<String, String>;
    /// Writes the index's tree objects and returns the top tree's id.
    fn write_index_tree(&self, cwd: &str) -> Result<String, String>;
    /// Writes a commit object holding `tree` with `parent` as its parent and returns its id; no ref moves.
    fn commit_tree(&self, cwd: &str, tree: &str, parent: &str, message: &str) -> Result<String, String>;
    /// The commits `head` has that `base` does not, newest first, each with its tree and parents.
    fn commits_between(&self, cwd: &str, base: &str, head: &str) -> Result<Vec<CommitLink>, String>;
    /// A tree's entries in the form `ls-files --stage -z` prints the index: `<mode> <object> 0\t<path>\0`.
    fn tree_entries(&self, cwd: &str, revision: &str) -> Result<String, String>;
    /// The tree a merge of `theirs` into `ours` would record, and whether it holds conflicts. Writes objects, moves no ref.
    fn merge_tree(&self, cwd: &str, ours: &str, theirs: &str) -> Result<MergeTree, String>;
    /// The diff of the index (`staged`) or the working tree against HEAD.
    fn diff_against_head(&self, cwd: &str, staged: bool) -> Result<String, String>;
    /// The paths that differ between the index (`staged`) or the working tree and HEAD.
    fn paths_against_head(&self, cwd: &str, staged: bool) -> Result<Vec<String>, String>;
    /// The checked-out branch; empty on a detached HEAD.
    fn current_branch(&self, cwd: &str) -> Result<String, String>;
    /// What `git status` lists: each path with its two-letter status code, untracked files one by one.
    fn porcelain_status(&self, cwd: &str) -> Result<Vec<PorcelainEntry>, String>;
}

/// One path `git status` lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PorcelainEntry {
    pub code: String,
    pub path: String,
}

/// The entries of `git status --porcelain=v1 -z`: a rename or copy is followed by the path it came from, which is not
/// an entry of its own.
pub fn parse_porcelain(listing: &str) -> Vec<PorcelainEntry> {
    let fields: Vec<&str> = listing.split('\0').collect();
    let mut entries = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let field = fields[index];
        index += 1;
        if field.is_empty() {
            continue;
        }
        let code: String = field.chars().take(2).collect();
        entries.push(PorcelainEntry { path: field.chars().skip(3).collect(), code: code.clone() });
        if code.contains('R') || code.contains('C') {
            index += 1;
        }
    }
    entries
}

/// One commit a merge brings in: its id, its tree and its parents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitLink {
    pub commit: String,
    pub tree: String,
    pub parents: Vec<String>,
}

/// What `git merge-tree --write-tree` gave: the merged tree, and whether it holds conflict markers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeTree {
    pub tree: String,
    pub conflicts: bool,
}

/// `rev-list --format=%H %T %P` lines: one commit link per line that holds something.
pub fn commit_links(listing: &str) -> Vec<CommitLink> {
    listing
        .split('\n')
        .map(js_trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut words = line.split(' ');
            let commit = words.next().unwrap_or_default().to_string();
            let tree = words.next().unwrap_or_default().to_string();
            CommitLink { commit, parents: words.map(str::to_string).collect(), tree }
        })
        .collect()
}

/// `ls-tree -r -z` entries, `<mode> <type> <object>\t<path>`, as `ls-files --stage -z` prints index entries.
pub fn stage_entries(listing: &str) -> String {
    listing.split('\0').filter_map(stage_entry).collect()
}

fn stage_entry(entry: &str) -> Option<String> {
    let (header, path) = entry.split_once('\t')?;
    let fields: Vec<&str> = header.split(' ').collect();
    let [mode, kind, object] = fields[..] else { return None };
    let word = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    let digits = !mode.is_empty() && mode.bytes().all(|b| b.is_ascii_digit());
    (digits && word(kind) && word(object) && !path.is_empty()).then(|| format!("{mode} {object} 0\t{path}\0"))
}

/// What `diff` compares the given side of the change with: `--cached HEAD --` for the index, `HEAD --` for the work tree.
fn against_head(staged: bool) -> &'static [&'static str] {
    if staged {
        &["--cached", "HEAD", "--"]
    } else {
        &["HEAD", "--"]
    }
}

/// The last commit that changed one path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastChange {
    pub commit: String,
    pub date: String,
}

/// One entry of `git ls-tree -r -z`, `<mode> <type> <object>\t<path>`: the path of a blob that is no symbolic link (mode 120000).
pub fn blob_path(entry: &str) -> Option<&str> {
    let (header, path) = entry.split_once('\t')?;
    let word = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    let fields: Vec<&str> = header.split(' ').collect();
    let [mode, kind, object] = fields[..] else {
        return None;
    };
    let digits = !mode.is_empty() && mode.bytes().all(|b| b.is_ascii_digit());
    (digits && word(kind) && word(object) && !path.is_empty() && kind == "blob" && mode != "120000").then_some(path)
}

/// git in the environment it was given (PATH decides which git).
pub struct SystemGit {
    pub env: HashMap<String, String>,
}

impl SystemGit {
    /// Runs git in `cwd`; an exit code outside `accepted` fails with the words the node adapter's `GitCommandError` has.
    fn run(&self, cwd: &str, args: &[&str], accepted: &[i32]) -> Result<String, String> {
        self.run_coded(cwd, args, accepted).map(|(_, stdout)| stdout)
    }

    /// `run`, with the exit code it ended on.
    fn run_coded(&self, cwd: &str, args: &[&str], accepted: &[i32]) -> Result<(i32, String), String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(&self.env)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("spawn git {}", fail(&error, "spawn", None).code))?;
        let code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if accepted.contains(&code) {
            return Ok((code, stdout));
        }
        Err(failure_words(args, cwd, code, &stdout, &String::from_utf8_lossy(&output.stderr)))
    }
}

/// `git <args> failed in <cwd>: <detail>`: what git said on stderr, else on stdout, else its exit code.
pub fn failure_words(args: &[&str], cwd: &str, code: i32, stdout: &str, stderr: &str) -> String {
    let detail = [js_trim(stderr), js_trim(stdout)].into_iter().find(|text| !text.is_empty()).map_or_else(|| format!("exit code {code}"), str::to_string);
    format!("git {} failed in {cwd}: {detail}", args.join(" "))
}

/// `remote.<name>.url <url>`, as `/^remote\.(.+)\.url (.+)$/` reads a line: the name runs to the last `.url `.
pub fn parse_remote_line(line: &str) -> Option<GitRemote> {
    let rest = line.strip_prefix("remote.")?;
    let dot = |c: char| matches!(c, '\r' | '\u{2028}' | '\u{2029}');
    let (name, url) = rest.rmatch_indices(".url ").find_map(|(at, found)| {
        let (name, url) = (&rest[..at], &rest[at + found.len()..]);
        (!name.is_empty() && !url.is_empty() && !name.contains(dot) && !url.contains(dot)).then_some((name, url))
    })?;
    Some(GitRemote { name: name.to_string(), url: url.to_string() })
}

/// The paths a `-z` listing holds.
fn nul_paths(listing: &str) -> Vec<String> {
    listing.split('\0').filter(|path| !path.is_empty()).map(str::to_string).collect()
}

impl Git for SystemGit {
    fn remotes(&self, cwd: &str) -> Result<Vec<GitRemote>, String> {
        // `git config --get-regexp` exits 1 when nothing matches.
        let listing = self.run(cwd, &["config", "--get-regexp", r"^remote\..*\.url$"], &[0, 1])?;
        Ok(listing.split('\n').filter_map(|line| parse_remote_line(js_trim(line))).collect())
    }

    fn common_dir(&self, cwd: &str) -> Result<String, String> {
        self.run(cwd, &["rev-parse", "--path-format=absolute", "--git-common-dir"], &[0]).map(|out| js_trim(&out).to_string())
    }

    fn root(&self, cwd: &str) -> Result<String, String> {
        self.run(cwd, &["rev-parse", "--show-toplevel"], &[0]).map(|out| js_trim(&out).to_string())
    }

    fn resolve_commit(&self, cwd: &str, revision: &str) -> Result<String, String> {
        let named = format!("{revision}^{{commit}}");
        self.run(cwd, &["rev-parse", "--verify", "--quiet", "--end-of-options", &named], &[0]).map(|out| js_trim(&out).to_string())
    }

    fn last_change(&self, cwd: &str, path: &str) -> Result<Option<LastChange>, String> {
        let listing = self.run(cwd, &["--literal-pathspecs", "log", "-1", "--format=%H%x00%cI", "--", path], &[0])?;
        let mut fields = js_trim(&listing).split('\0');
        let commit = fields.next().unwrap_or_default();
        let date = fields.next().unwrap_or_default();
        Ok((!commit.is_empty()).then(|| LastChange { commit: commit.to_string(), date: date.to_string() }))
    }

    fn file_commits(&self, cwd: &str, revision: &str, path: &str) -> Result<Vec<LastChange>, String> {
        let listing = self.run(cwd, &["--literal-pathspecs", "log", "--format=%H%x00%cI", "--end-of-options", revision, "--", path], &[0])?;
        Ok(listing
            .split('\n')
            .map(js_trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                let mut fields = line.split('\0');
                LastChange { commit: fields.next().unwrap_or_default().to_string(), date: fields.next().unwrap_or_default().to_string() }
            })
            .collect())
    }

    fn file_at(&self, cwd: &str, commit: &str, path: &str) -> Result<String, String> {
        self.run(cwd, &["cat-file", "blob", &format!("{commit}:{path}")], &[0])
    }

    fn tree_files(&self, cwd: &str, commit: &str) -> Result<Vec<String>, String> {
        let listing = self.run(cwd, &["ls-tree", "-r", "-z", "--full-tree", commit], &[0])?;
        Ok(listing.split('\0').filter_map(blob_path).map(str::to_string).collect())
    }

    fn untracked_paths(&self, cwd: &str) -> Result<Vec<String>, String> {
        self.run(cwd, &["--no-optional-locks", "ls-files", "--others", "--exclude-standard", "-z"], &[0]).map(|out| nul_paths(&out))
    }

    fn merge_base(&self, cwd: &str, a: &str, b: &str) -> Result<String, String> {
        self.run(cwd, &["merge-base", "--end-of-options", a, b], &[0]).map(|out| js_trim(&out).to_string())
    }

    fn diff_commits(&self, cwd: &str, base: &str, head: &str) -> Result<String, String> {
        self.run(cwd, &["-c", "core.quotePath=false", "diff", "--no-ext-diff", "--no-textconv", "--no-color", base, head, "--"], &[0])
    }

    fn changed_paths(&self, cwd: &str, base: &str, head: &str) -> Result<Vec<String>, String> {
        self.run(cwd, &["diff", "--name-only", "-z", "--no-renames", base, head, "--"], &[0]).map(|out| nul_paths(&out))
    }

    fn staged_entries(&self, cwd: &str) -> Result<String, String> {
        self.run(cwd, &["ls-files", "--stage", "-z"], &[0])
    }

    fn tree_of(&self, cwd: &str, revision: &str) -> Result<String, String> {
        let named = format!("{revision}^{{tree}}");
        self.run(cwd, &["rev-parse", "--verify", "--quiet", "--end-of-options", &named], &[0]).map(|out| js_trim(&out).to_string())
    }

    fn write_index_tree(&self, cwd: &str) -> Result<String, String> {
        self.run(cwd, &["write-tree"], &[0]).map(|out| js_trim(&out).to_string())
    }

    fn commit_tree(&self, cwd: &str, tree: &str, parent: &str, message: &str) -> Result<String, String> {
        // The commit object is a snapshot nobody pushes, so it carries a fixed identity and no signature.
        let identity = ["-c", "user.name=agent-kit critique", "-c", "user.email=critique@agent-kit.invalid"];
        let mut args: Vec<&str> = identity.to_vec();
        args.extend(["commit-tree", "--no-gpg-sign", "-p", parent, "-m", message, tree]);
        self.run(cwd, &args, &[0]).map(|out| js_trim(&out).to_string())
    }

    fn commits_between(&self, cwd: &str, base: &str, head: &str) -> Result<Vec<CommitLink>, String> {
        let not_base = format!("^{base}");
        self.run(cwd, &["rev-list", "--no-commit-header", "--format=%H %T %P", "--end-of-options", head, &not_base], &[0]).map(|out| commit_links(&out))
    }

    fn tree_entries(&self, cwd: &str, revision: &str) -> Result<String, String> {
        self.run(cwd, &["ls-tree", "-r", "-z", "--full-tree", revision], &[0]).map(|out| stage_entries(&out))
    }

    fn merge_tree(&self, cwd: &str, ours: &str, theirs: &str) -> Result<MergeTree, String> {
        // Exit 1 is a merge with conflicts, its tree holding the conflict markers.
        let args = ["-c", "merge.conflictStyle=merge", "merge-tree", "--write-tree", "--no-messages", "--name-only", "--end-of-options", ours, theirs];
        let (code, out) = self.run_coded(cwd, &args, &[0, 1])?;
        Ok(MergeTree { conflicts: code == 1, tree: js_trim(out.split('\n').next().unwrap_or_default()).to_string() })
    }

    fn diff_against_head(&self, cwd: &str, staged: bool) -> Result<String, String> {
        let mut args = vec!["--no-optional-locks", "-c", "core.quotePath=false", "diff", "--no-ext-diff", "--no-textconv", "--no-color"];
        args.extend(against_head(staged));
        self.run(cwd, &args, &[0])
    }

    fn paths_against_head(&self, cwd: &str, staged: bool) -> Result<Vec<String>, String> {
        let mut args = vec!["--no-optional-locks", "diff", "--name-only", "-z", "--no-renames"];
        args.extend(against_head(staged));
        self.run(cwd, &args, &[0]).map(|out| nul_paths(&out))
    }

    fn current_branch(&self, cwd: &str) -> Result<String, String> {
        self.run(cwd, &["branch", "--show-current"], &[0]).map(|out| js_trim(&out).to_string())
    }

    fn porcelain_status(&self, cwd: &str) -> Result<Vec<PorcelainEntry>, String> {
        self.run(cwd, &["status", "--porcelain=v1", "-z", "--untracked-files=all"], &[0]).map(|out| parse_porcelain(&out))
    }
}
