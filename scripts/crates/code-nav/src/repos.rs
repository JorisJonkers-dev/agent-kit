//! Related repositories: which repository depends on which, from a manifest
//! generated on the spot from the workspace layout and the build files.
//!
//! Nothing is stored: every call walks the allowed roots as they are now.
//!
//! **Repository.** The nearest directory at or above a file that holds a `.git`
//! entry; when a root has none, each directory directly under the root. A file
//! that sits directly in a root with no `.git` belongs to no repository.
//!
//! **Manifest.** For each repository, the package names its build files provide
//! (plus the repository's own directory name) and the names they require; see
//! [`crate::build_files`] for the formats read. Only the build files are read,
//! one at a time.
//!
//! **Edge.** Repository A depends on repository B when A requires a name B
//! provides and A is not B. A name only A provides is internal; a name nobody
//! provides is external and is not listed.
//!
//! The walk is the map's: it stays inside the allowed roots, skips symlinks,
//! secret and withheld (`nedap`) locations and binary files, and stops after
//! [`MAX_FILES`] files.

use crate::build_files::{is_build_file, normalise, parse};
use crate::map::{read_text, visit, MAX_FILES};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// One repository and the names its build files declare.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoEntry {
    pub name: String,
    pub path: PathBuf,
    pub provides: BTreeSet<String>,
    pub requires: BTreeSet<String>,
}

/// Every repository under the roots, sorted by path.
#[derive(Debug, PartialEq)]
pub struct Manifest {
    pub repos: Vec<RepoEntry>,
    /// True when the walk stopped at the file cap.
    pub walk_capped: bool,
}

/// One related repository and a name that links it.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub repo: String,
    pub via: String,
}

/// The repositories one repository depends on and the ones that depend on it.
#[derive(Debug, PartialEq)]
pub struct Related {
    pub depends_on: Vec<Link>,
    pub depended_on_by: Vec<Link>,
}

/// Finds the repository a directory belongs to, remembering each answer.
pub(crate) struct RepoFinder<'a> {
    roots: &'a [PathBuf],
    cache: HashMap<PathBuf, Option<PathBuf>>,
}

impl<'a> RepoFinder<'a> {
    pub(crate) fn new(roots: &'a [PathBuf]) -> Self {
        RepoFinder { roots, cache: HashMap::new() }
    }

    /// The repository directory holding `dir`, or `None`.
    pub(crate) fn repo_of(&mut self, dir: &Path) -> Option<PathBuf> {
        if let Some(known) = self.cache.get(dir) {
            return known.clone();
        }
        let root = self.roots.iter().find(|r| dir.starts_with(r))?;
        let found = if has_git(dir) {
            Some(dir.to_path_buf())
        } else if dir == root {
            None
        } else {
            let parent = dir.parent().unwrap_or(dir);
            if parent == root && !has_git(root) {
                Some(dir.to_path_buf())
            } else {
                let parent = parent.to_path_buf();
                self.repo_of(&parent)
            }
        };
        self.cache.insert(dir.to_path_buf(), found.clone());
        found
    }
}

fn has_git(dir: &Path) -> bool {
    dir.join(".git").symlink_metadata().is_ok()
}

/// The manifest of every repository under the canonical `roots`.
pub fn manifest(roots: &[PathBuf]) -> Manifest {
    manifest_with(roots, MAX_FILES, &mut read_text)
}

/// [`manifest`] with an explicit file cap and reader; only build files reach the reader.
pub(crate) fn manifest_with(roots: &[PathBuf], max_files: usize, read: &mut dyn FnMut(&Path) -> Option<String>) -> Manifest {
    let mut paths: Vec<PathBuf> = Vec::new();
    for root in roots {
        visit(root, max_files + 1, &mut paths);
    }
    let walk_capped = paths.len() > max_files;
    paths.truncate(max_files);
    let mut finder = RepoFinder::new(roots);
    let mut repos: BTreeMap<PathBuf, RepoEntry> = BTreeMap::new();
    for path in &paths {
        let Some(dir) = path.parent().and_then(|d| finder.repo_of(d)) else { continue };
        let repo = repos.entry(dir.clone()).or_insert_with(|| {
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            RepoEntry { provides: BTreeSet::from([normalise(&name)]), name, path: dir.clone(), requires: BTreeSet::new() }
        });
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !is_build_file(&file_name) {
            continue;
        }
        if let Some(text) = read(path) {
            let info = parse(&file_name, &text);
            repo.provides.extend(info.provides);
            repo.requires.extend(info.requires);
        }
    }
    Manifest { repos: repos.into_values().collect(), walk_capped }
}

impl Manifest {
    /// The repository with this directory name (any case), or this path.
    pub fn find(&self, name_or_path: &str) -> Option<&RepoEntry> {
        let wanted = name_or_path.trim_end_matches('/');
        self.repos
            .iter()
            .find(|r| r.path == Path::new(wanted))
            .or_else(|| self.repos.iter().find(|r| r.name.eq_ignore_ascii_case(wanted)))
    }

    /// What `repo` depends on and what depends on it, each sorted by name.
    pub fn related(&self, repo: &RepoEntry) -> Related {
        let mut depends_on = Vec::new();
        let mut depended_on_by = Vec::new();
        for other in self.repos.iter().filter(|o| o.path != repo.path) {
            if let Some(via) = repo.requires.iter().find(|n| other.provides.contains(*n)) {
                depends_on.push(Link { repo: other.name.clone(), via: via.clone() });
            }
            if let Some(via) = other.requires.iter().find(|n| repo.provides.contains(*n)) {
                depended_on_by.push(Link { repo: other.name.clone(), via: via.clone() });
            }
        }
        depends_on.sort_by(|a, b| a.repo.cmp(&b.repo));
        depended_on_by.sort_by(|a, b| a.repo.cmp(&b.repo));
        Related { depends_on, depended_on_by }
    }
}
