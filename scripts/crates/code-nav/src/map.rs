//! Repository map: files and symbols ranked by how often they are referenced.
//!
//! Nothing is indexed: every call walks the files as they are now.
//!
//! **Reference.** A symbol's references are the whole-identifier occurrences
//! of its name in every scanned file, minus the declarations of that name
//! (each declaration spells the name once). `impl` blocks are not symbols
//! here: they name a type that is declared elsewhere.
//!
//! **Rank.** Symbols sort by references, highest first; ties go to the path
//! with fewer components, then the path, then the line. A file's score is the
//! sum of the references of the symbols it declares; files sort the same way.
//! Both lists are capped at `limit`, and `total_*` say how much was left out.
//!
//! The walk skips symlinks, secret files, VCS and build directories, files
//! over [`MAX_FILE_BYTES`], anything that is not UTF-8 text, and stops after
//! [`MAX_FILES`] files so a huge root cannot stall a call.

use crate::boundary::is_secret;
use crate::outline;
use crate::search::{MAX_FILE_BYTES, SKIPPED_DIRS};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Files read in one call.
pub const MAX_FILES: usize = 20_000;

/// A declared symbol and how often it is referenced.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub line: usize,
    pub kind: String,
    pub name: String,
    pub refs: usize,
}

/// A file and the references to the symbols it declares.
#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub symbols: usize,
    pub refs: usize,
}

/// The ranked, capped map and the totals before the cap.
#[derive(Debug, PartialEq)]
pub struct RepoMap {
    pub files: Vec<FileEntry>,
    pub symbols: Vec<Entry>,
    pub total_files: usize,
    pub total_symbols: usize,
    /// True when the walk stopped at the file cap.
    pub walk_capped: bool,
}

/// Map every file under the canonical `scope` paths, keeping `limit` entries
/// of each list.
pub fn map(scope: &[PathBuf], limit: usize) -> RepoMap {
    map_capped(scope, limit, MAX_FILES)
}

/// [`map`] with an explicit file cap.
pub fn map_capped(scope: &[PathBuf], limit: usize, max_files: usize) -> RepoMap {
    let mut texts: Vec<(PathBuf, String)> = Vec::new();
    for path in scope {
        visit(path, max_files, &mut texts);
    }
    let walk_capped = texts.len() >= max_files;
    let mut words: HashMap<&str, usize> = HashMap::new();
    let mut declared: HashMap<String, usize> = HashMap::new();
    let mut entries: Vec<Entry> = Vec::new();
    for (path, text) in &texts {
        for word in identifiers(text) {
            *words.entry(word).or_insert(0) += 1;
        }
        for s in outline::symbols(text).into_iter().filter(|s| s.kind != "impl") {
            *declared.entry(s.name.clone()).or_insert(0) += 1;
            entries.push(Entry { path: path.clone(), line: s.line, kind: s.kind, name: s.name, refs: 0 });
        }
    }
    for entry in &mut entries {
        let seen = words.get(entry.name.as_str()).copied().unwrap_or(0);
        entry.refs = seen.saturating_sub(declared[&entry.name]);
    }
    let mut per_file: HashMap<&Path, FileEntry> = HashMap::new();
    for entry in &entries {
        let file = per_file
            .entry(entry.path.as_path())
            .or_insert_with(|| FileEntry { path: entry.path.clone(), symbols: 0, refs: 0 });
        file.symbols += 1;
        file.refs += entry.refs;
    }
    let mut files: Vec<FileEntry> = per_file.into_values().collect();
    files.sort_by(|a, b| b.refs.cmp(&a.refs).then_with(|| path_order(&a.path, &b.path)));
    entries.sort_by(|a, b| {
        b.refs.cmp(&a.refs).then_with(|| path_order(&a.path, &b.path)).then(a.line.cmp(&b.line))
    });
    let (total_files, total_symbols) = (files.len(), entries.len());
    files.truncate(limit);
    entries.truncate(limit);
    RepoMap { files, symbols: entries, total_files, total_symbols, walk_capped }
}

/// Fewer components first, then the path.
fn path_order(a: &Path, b: &Path) -> std::cmp::Ordering {
    a.components().count().cmp(&b.components().count()).then_with(|| a.cmp(b))
}

/// Read every admissible text file under `path` into `texts`, up to `max_files`.
fn visit(path: &Path, max_files: usize, texts: &mut Vec<(PathBuf, String)>) {
    if texts.len() >= max_files {
        return;
    }
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
            if SKIPPED_DIRS.contains(&name.as_str()) || is_secret(Path::new(&name)) {
                continue;
            }
            visit(&child, max_files, texts);
        }
    } else if meta.is_file() && meta.len() <= MAX_FILE_BYTES {
        if let Ok(text) = fs::read_to_string(path) {
            if !text.contains('\0') {
                texts.push((path.to_path_buf(), text));
            }
        }
    }
}

/// Every identifier in `text`.
fn identifiers(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty())
}
