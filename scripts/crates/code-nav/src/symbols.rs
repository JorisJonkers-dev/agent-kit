//! Cross-repository symbol lookup: where a name is declared, across every
//! repository under the allowed roots.
//!
//! Nothing is indexed: the walk is the map's and each file is read once, one
//! text in memory at a time. A declaration is what [`crate::outline`] finds.
//!
//! **Match.** A symbol matches when its name contains the query, ignoring case.
//!
//! **Rank.** An exact match (same case) first, then one that differs only in
//! case, then a substring match; ties go to the path with fewer components,
//! then the path, then the line. At most `limit` symbols are returned and
//! `total` counts every match.
//!
//! Files in a `worktrees` directory are skipped (see [`crate::repos`]).
//!
//! Each symbol names the repository it is in (see [`crate::repos`]), or none
//! for a file that sits outside any repository.

use crate::map::{read_text, visit, MAX_FILES};
use crate::outline;
use crate::repos::{in_worktrees, RepoFinder};
use std::path::{Path, PathBuf};

/// One declaration that matches.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The directory name of the repository holding the file, if any.
    pub repo: Option<String>,
    pub path: PathBuf,
    pub line: usize,
    pub kind: String,
    pub name: String,
    /// 0 for an exact match, 1 for a case-insensitive one, 2 for a substring.
    pub class: u8,
}

/// The ranked, capped matches and the total before the cap.
#[derive(Debug, PartialEq)]
pub struct Symbols {
    pub found: Vec<Found>,
    pub total: usize,
    /// True when the walk stopped at the file cap.
    pub walk_capped: bool,
}

fn order(a: &Found, b: &Found) -> std::cmp::Ordering {
    a.class
        .cmp(&b.class)
        .then(a.path.components().count().cmp(&b.path.components().count()))
        .then_with(|| a.path.cmp(&b.path))
        .then(a.line.cmp(&b.line))
}

/// Find the symbols named like `query` under the canonical `scope` paths;
/// `roots` are the allowlist, which decides each file's repository.
pub fn find(roots: &[PathBuf], scope: &[PathBuf], query: &str, limit: usize) -> Symbols {
    find_with(roots, scope, query, limit, MAX_FILES, &mut read_text)
}

/// [`find`] with an explicit file cap and reader.
pub(crate) fn find_with(
    roots: &[PathBuf],
    scope: &[PathBuf],
    query: &str,
    limit: usize,
    max_files: usize,
    read: &mut dyn FnMut(&Path) -> Option<String>,
) -> Symbols {
    let needle = query.to_lowercase();
    let mut paths: Vec<PathBuf> = Vec::new();
    for path in scope {
        visit(path, max_files + 1, &mut paths);
    }
    let walk_capped = paths.len() > max_files;
    paths.truncate(max_files);
    let mut finder = RepoFinder::new(roots);
    let mut found: Vec<Found> = Vec::new();
    let mut total = 0;
    for path in paths.iter().filter(|p| !in_worktrees(roots, p)) {
        let Some(text) = read(path) else { continue };
        let mut repo: Option<Option<String>> = None;
        for s in outline::symbols(&text).into_iter().filter(|s| s.kind != "impl") {
            let class = if s.name == query {
                0
            } else if s.name.to_lowercase() == needle {
                1
            } else if s.name.to_lowercase().contains(&needle) {
                2
            } else {
                continue;
            };
            total += 1;
            let repo = repo
                .get_or_insert_with(|| {
                    path.parent()
                        .and_then(|d| finder.repo_of(d))
                        .and_then(|r| r.file_name().map(|n| n.to_string_lossy().into_owned()))
                })
                .clone();
            found.push(Found { repo, path: path.clone(), line: s.line, kind: s.kind, name: s.name, class });
            if found.len() >= limit * 2 {
                found.sort_by(order);
                found.truncate(limit);
            }
        }
    }
    found.sort_by(order);
    found.truncate(limit);
    Symbols { found, total, walk_capped }
}
