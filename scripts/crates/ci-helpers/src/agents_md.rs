//! Check that each repository's AGENTS.md names only things that exist.
//!
//! AGENTS.md is the file a coding agent reads first, so a path or a symbol it
//! names that has since moved is worse than no pointer at all. This lint reads
//! each repository root it is given and reports:
//!
//! * a missing or empty `AGENTS.md` (whitespace-only counts as empty);
//! * a markdown link whose target does not exist, resolved against the root,
//!   after a `#fragment` or `?query` is stripped;
//! * an inline code span that looks like a repository path and does not exist;
//! * a `<path>::<symbol>` span whose path is gone, or whose symbol no longer
//!   appears as a whole word in that file (or, for a directory, in any file
//!   under it).
//!
//! Fenced code blocks are skipped, and so are links with a scheme and pure
//! `#anchor` links. Problems go to stderr, one per line, as
//! `<REPO_ROOT>/AGENTS.md:<line>: <message>`. Exit 0 when every repository is
//! clean, 1 when a problem was found, 2 when a REPO_ROOT is not a directory.

use crate::Io;
use std::fs;
use std::path::{Path, PathBuf};

const AGENTS_MD: &str = "AGENTS.md";
const EXTENSIONS: [&str; 19] = [
    "md", "py", "rs", "ts", "tsx", "js", "mjs", "json", "yaml", "yml", "toml", "sh", "nix", "kt",
    "kts", "go", "lock", "txt", "cfg",
];
/// Characters that make a span a pattern, a shell fragment or a placeholder.
const NOT_A_PATH: &str = "*?<>${}|~";

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The (1-based line number, text) pairs outside fenced code blocks.
fn unfenced_lines(text: &str) -> Vec<(usize, &str)> {
    let mut kept = Vec::new();
    let mut fence = String::new();
    for (index, line) in text.lines().enumerate() {
        let opener = fence_of(line);
        if !fence.is_empty() {
            if opener
                .as_ref()
                .is_some_and(|run| run.starts_with(&fence[..1]) && run.len() >= fence.len())
            {
                fence.clear();
            }
        } else if let Some(run) = opener {
            fence = run;
        } else {
            kept.push((index + 1, line));
        }
    }
    kept
}

/// `^\s{0,3}(`{3,}|~{3,})`: the run of fence characters a line opens with.
fn fence_of(line: &str) -> Option<String> {
    let rest = line.trim_start_matches(char::is_whitespace);
    if line.chars().count() - rest.chars().count() > 3 {
        return None;
    }
    let mark = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run: String = rest.chars().take_while(|c| *c == mark).collect();
    (run.len() >= 3).then_some(run)
}

/// Strip the `:<digits>` and `()` a span may end with, the trailing slashes and a leading `./`.
fn clean(span: &str) -> &str {
    let mut text = span.strip_suffix("()").unwrap_or(span);
    if let Some((head, digits)) = text.rsplit_once(':') {
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            text = head;
        }
    }
    let text = text.trim_end_matches('/');
    text.strip_prefix("./").unwrap_or(text)
}

fn looks_like_path(span: &str) -> bool {
    if span.is_empty()
        || span.chars().any(char::is_whitespace)
        || span.chars().any(|c| NOT_A_PATH.contains(c))
    {
        return false;
    }
    if span.starts_with(['-', '/', '~']) || span.contains("://") {
        return false;
    }
    let candidate = clean(span);
    if span.ends_with('/') || candidate.contains('/') {
        return true;
    }
    candidate
        .rsplit_once('.')
        .is_some_and(|(stem, extension)| !stem.is_empty() && EXTENSIONS.contains(&extension))
}

fn resolve(root: &Path, span: &str) -> Option<PathBuf> {
    Some(root.join(clean(span))).filter(|path| path.exists())
}

fn files_under(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, found);
        } else if path.is_file() {
            found.push(path);
        }
    }
}

/// Whether `symbol` appears as a whole word in a file, or under a directory.
fn mentions(path: &Path, symbol: &str) -> bool {
    let mut files = Vec::new();
    if path.is_file() {
        files.push(path.to_path_buf());
    } else {
        files_under(path, &mut files);
    }
    files.iter().any(|file| {
        has_word(
            &String::from_utf8_lossy(&fs::read(file).unwrap_or_default()),
            symbol,
        )
    })
}

fn has_word(text: &str, symbol: &str) -> bool {
    text.match_indices(symbol).any(|(at, found)| {
        let before = text[..at].chars().next_back().is_some_and(is_word);
        let after = text[at + found.len()..].chars().next().is_some_and(is_word);
        !before && !after
    })
}

fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn has_scheme(target: &str) -> bool {
    let mut chars = target.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && target.find(':').is_some_and(|colon| {
            target[1..colon]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        })
}

fn link_problem(root: &Path, target: &str) -> Option<String> {
    if target.is_empty() || target.starts_with('#') || has_scheme(target) {
        return None;
    }
    let path = target
        .split('#')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("");
    if path.is_empty() {
        return None;
    }
    let decoded = unquote(path);
    (!root.join(decoded.trim_start_matches('/')).exists()).then(|| format!("dead path: {path}"))
}

/// `^(.+?)::(\w+(?:(?:\.|::)\w+)*)(?:\(\))?$`: the path and the symbol.
fn symbol_reference(span: &str) -> Option<(&str, &str)> {
    (1..span.len())
        .filter(|at| span.is_char_boundary(*at) && span[*at..].starts_with("::"))
        .find_map(|at| {
            let symbol = span[at + 2..].strip_suffix("()").unwrap_or(&span[at + 2..]);
            symbol_names(symbol).map(|_| (&span[..at], symbol))
        })
}

/// The names of `a.b::c`, or None when it is not that shape.
fn symbol_names(symbol: &str) -> Option<Vec<&str>> {
    let names: Vec<&str> = symbol
        .split("::")
        .flat_map(|part| part.split('.'))
        .collect();
    names
        .iter()
        .all(|name| !name.is_empty() && name.chars().all(is_word))
        .then_some(names)
}

fn span_problem(root: &Path, span: &str) -> Option<String> {
    if let Some((path, symbol)) = symbol_reference(span) {
        if !looks_like_path(path) {
            return None;
        }
        let Some(resolved) = resolve(root, path) else {
            return Some(format!("dead path: {path}"));
        };
        let names = symbol_names(symbol).unwrap_or_default();
        return (!names.iter().all(|name| mentions(&resolved, name)))
            .then(|| format!("dead symbol: {span}"));
    }
    if !looks_like_path(span) {
        return None;
    }
    resolve(root, span)
        .is_none()
        .then(|| format!("dead path: {}", clean(span)))
}

/// `\[[^\]]*\]\(<?([^)\s>]+)>?(?:\s+"[^"]*")?\)` at `i`: the end and the target.
fn link_at(chars: &[char], i: usize) -> Option<(usize, String)> {
    let close = i + 1 + chars[i + 1..].iter().position(|c| *c == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let angled = chars.get(close + 2) == Some(&'<');
    let variants: &[bool] = if angled { &[true, false] } else { &[false] };
    variants.iter().find_map(|skip| {
        let start = close + 2 + usize::from(*skip);
        let len = chars[start..]
            .iter()
            .take_while(|c| !(**c == ')' || c.is_whitespace() || **c == '>'))
            .count();
        let mut end = start + len;
        end += usize::from(len > 0 && chars.get(end) == Some(&'>'));
        let blanks = chars[end..]
            .iter()
            .take_while(|c| c.is_whitespace())
            .count();
        if blanks > 0 && chars.get(end + blanks) == Some(&'"') {
            let quoted = chars[end + blanks + 1..].iter().position(|c| *c == '"');
            end = quoted.map_or(end, |q| end + blanks + 1 + q + 1);
        }
        (len > 0 && chars.get(end) == Some(&')'))
            .then(|| (end + 1, chars[start..start + len].iter().collect()))
    })
}

/// `` `([^`\n]+)` `` at `i`: the end and the span.
fn span_at(chars: &[char], i: usize) -> Option<(usize, String)> {
    let len = chars[i + 1..].iter().position(|c| *c == '`')?;
    (len > 0).then(|| (i + len + 2, chars[i + 1..i + 1 + len].iter().collect()))
}

/// The problems one line causes, in the order they appear on it.
fn line_problems(root: &Path, line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut found: Vec<(usize, String)> = Vec::new();
    for (opener, find) in [
        (
            '[',
            link_at as fn(&[char], usize) -> Option<(usize, String)>,
        ),
        ('`', span_at),
    ] {
        let mut i = 0;
        while i < chars.len() {
            match (chars[i] == opener).then(|| find(&chars, i)).flatten() {
                Some((end, text)) => {
                    let message = if opener == '[' {
                        link_problem(root, &text)
                    } else {
                        span_problem(root, &text)
                    };
                    found.extend(message.map(|message| (i, message)));
                    i = end;
                }
                None => i += 1,
            }
        }
    }
    found.sort_by_key(|entry| entry.0);
    found.into_iter().map(|(_, message)| message).collect()
}

/// The problem lines for one repository, each prefixed with its AGENTS.md path.
pub fn check_repo(root: &Path) -> Vec<String> {
    let label = root.join(AGENTS_MD).display().to_string();
    let path = root.join(AGENTS_MD);
    if !path.is_file() {
        return vec![format!("{label}:1: {AGENTS_MD} is missing")];
    }
    let Ok(text) = fs::read_to_string(&path) else {
        return vec![format!("{label}:1: {AGENTS_MD} is not valid utf-8")];
    };
    if text.trim().is_empty() {
        return vec![format!("{label}:1: {AGENTS_MD} is empty")];
    }
    let mut problems = Vec::new();
    for (number, line) in unfenced_lines(&text) {
        let mut seen: Vec<String> = Vec::new();
        for message in line_problems(root, line) {
            if !seen.contains(&message) {
                problems.push(format!("{label}:{number}: {message}"));
                seen.push(message);
            }
        }
    }
    problems
}

pub fn run(args: &[String], io: &mut Io) -> i32 {
    let mut roots: Vec<PathBuf> = Vec::new();
    for arg in args {
        if arg.starts_with('-') && arg != "--require" {
            io.complain(&format!("agents-md: unknown option {arg}"));
            return 2;
        }
        if arg != "--require" {
            roots.push(PathBuf::from(arg));
        }
    }
    if roots.is_empty() {
        roots.push(PathBuf::from("."));
    }
    if let Some(root) = roots.iter().find(|root| !root.is_dir()) {
        io.complain(&format!("{}: not a directory", root.display()));
        return 2;
    }
    let problems: Vec<String> = roots.iter().flat_map(|root| check_repo(root)).collect();
    for line in &problems {
        io.complain(line);
    }
    if !problems.is_empty() {
        return 1;
    }
    io.say(&format!("agents-md: {} repositories checked", roots.len()));
    0
}
