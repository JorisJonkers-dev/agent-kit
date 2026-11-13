//! A commit's tracked files a packet may send. The node runner's `application/tracked-tree.ts`.

use crate::error::CritiqueError;
use crate::repository_packet::{exclusion_of, is_generated, PacketLimits, TrackedFile};
use crate::runtime::Runtime;

/// A commit's tracked files a packet may send, and how many each exclusion left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackedTree {
    /// Every tracked path not excluded, sorted, with its text, or `None` for a binary file.
    pub tracked: Vec<TrackedFile>,
    /// How many paths each exclusion kind left out: generated, lock, vendored.
    pub excluded: Excluded,
}

/// The count of paths left out, per exclusion kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Excluded {
    pub generated: usize,
    pub lock: usize,
    pub vendored: usize,
}

impl Excluded {
    fn count(&mut self, kind: &str) {
        match kind {
            "generated" => self.generated += 1,
            "lock" => self.lock += 1,
            _ => self.vendored += 1,
        }
    }
}

/// Reads every tracked path at a commit from git's object store (so no `.git` and no worktree metadata), sorted. A
/// path routing's exclusion globs match is counted and never read; a file whose first lines say a tool generated it
/// is counted and left out; a file holding a NUL byte is binary, kept without its text.
pub fn read_tracked_tree(runtime: &Runtime, root: &str, commit: &str, limits: &PacketLimits) -> Result<TrackedTree, CritiqueError> {
    let mut excluded = Excluded::default();
    let mut paths = Vec::new();
    for path in runtime.git.tree_files(root, commit).map_err(CritiqueError::unexpected)? {
        match exclusion_of(&path, None, limits).map_err(CritiqueError::unexpected)? {
            Some(exclusion) => excluded.count(exclusion.kind),
            None => paths.push(path),
        }
    }
    let mut tracked = Vec::new();
    for path in paths {
        let text = runtime.git.file_at(root, commit, &path).map_err(CritiqueError::unexpected)?;
        if text.contains('\0') {
            tracked.push(TrackedFile { path, text: None });
        } else if is_generated(&text) {
            excluded.generated += 1;
        } else {
            tracked.push(TrackedFile { path, text: Some(text) });
        }
    }
    Ok(TrackedTree { excluded, tracked })
}
