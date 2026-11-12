//! Ranked, capped search of the live working tree.
//!
//! Nothing is indexed: every call reads the files as they are now.
//!
//! **Match.** A line matches when it contains the query, ignoring case.
//!
//! **Rank.** Each match scores 1, plus 4 when the query appears as a whole
//! identifier (not inside a longer one), plus 2 when the file's name contains
//! the query. Results sort by score, highest first; ties go to the path with
//! fewer components, then the path, then the line number.
//!
//! **Cap.** A call returns at most `limit` hits: [`DEFAULT_LIMIT`] unless the
//! caller asks for fewer or more, never above [`MAX_LIMIT`]. `total` counts
//! every match, so a capped result says how much it left out.
//!
//! The walk skips symlinks, secret files, VCS and build directories, files
//! over [`MAX_FILE_BYTES`] and anything that is not UTF-8 text.

use crate::boundary::{is_secret, is_withheld};
use std::fs;
use std::path::{Path, PathBuf};

/// The result cap when the caller names none.
pub const DEFAULT_LIMIT: usize = 20;
/// The largest cap a caller may ask for.
pub const MAX_LIMIT: usize = 200;
/// Files larger than this are not searched.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Longest line text returned, in characters.
const MAX_LINE_CHARS: usize = 200;
/// Directories never entered.
pub(crate) const SKIPPED_DIRS: [&str; 4] = [".git", "node_modules", "target", ".venv"];

/// One matching line.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub path: PathBuf,
    pub line: usize,
    pub text: String,
    pub score: u32,
}

/// The ranked, capped hits and how many matches there were in all.
#[derive(Debug, PartialEq)]
pub struct Results {
    pub hits: Vec<Hit>,
    pub total: usize,
}

impl Results {
    /// True when the cap left matches out.
    pub fn truncated(&self) -> bool {
        self.total > self.hits.len()
    }
}

/// The sort order: score descending, then path depth, path and line.
fn order(a: &Hit, b: &Hit) -> std::cmp::Ordering {
    b.score
        .cmp(&a.score)
        .then(a.path.components().count().cmp(&b.path.components().count()))
        .then(a.path.cmp(&b.path))
        .then(a.line.cmp(&b.line))
}

/// Search every file under the canonical `scope` paths (files or directories).
pub fn search(scope: &[PathBuf], query: &str, limit: usize) -> Results {
    let limit = limit.clamp(1, MAX_LIMIT);
    let needle = query.to_lowercase();
    let mut hits: Vec<Hit> = Vec::new();
    let mut total = 0;
    for path in scope {
        visit(path, &needle, limit, &mut hits, &mut total);
    }
    hits.sort_by(order);
    hits.truncate(limit);
    Results { hits, total }
}

/// Walk `path`; keep only the best `limit` hits so a common query stays small.
fn visit(path: &Path, needle: &str, limit: usize, hits: &mut Vec<Hit>, total: &mut usize) {
    let Ok(meta) = fs::symlink_metadata(path) else { return };
    if meta.file_type().is_symlink() {
        return;
    }
    if meta.is_dir() {
        let Ok(entries) = fs::read_dir(path) else { return };
        let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        children.sort();
        for child in children {
            let name = child.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if SKIPPED_DIRS.contains(&name.as_str()) || is_secret(Path::new(&name)) || is_withheld(Path::new(&name)) {
                continue;
            }
            visit(&child, needle, limit, hits, total);
        }
    } else if meta.is_file() && meta.len() <= MAX_FILE_BYTES {
        scan_file(path, needle, limit, hits, total);
    }
}

fn scan_file(path: &Path, needle: &str, limit: usize, hits: &mut Vec<Hit>, total: &mut usize) {
    let Ok(text) = fs::read_to_string(path) else { return };
    if text.contains('\0') {
        return;
    }
    let in_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase().contains(needle))
        .unwrap_or(false);
    for (i, line) in text.lines().enumerate() {
        let lower = line.to_lowercase();
        if !lower.contains(needle) {
            continue;
        }
        *total += 1;
        let score = 1 + if whole_identifier(&lower, needle) { 4 } else { 0 } + if in_name { 2 } else { 0 };
        hits.push(Hit { path: path.to_path_buf(), line: i + 1, text: clip(line.trim()), score });
        if hits.len() >= limit * 2 {
            hits.sort_by(order);
            hits.truncate(limit);
        }
    }
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// True when `needle` occurs in `line` with no identifier character on either side.
fn whole_identifier(line: &str, needle: &str) -> bool {
    line.match_indices(needle).any(|(at, m)| {
        let before = line[..at].chars().next_back().is_none_or(|c| !is_ident(c));
        let after = line[at + m.len()..].chars().next().is_none_or(|c| !is_ident(c));
        before && after
    })
}

fn clip(text: &str) -> String {
    text.chars().take(MAX_LINE_CHARS).collect()
}
