//! The change staged in a repository's index. `read_staged` writes nothing, so a commit gate can call it on every
//! commit; `snapshot` also writes the index as a tree and a commit object, so the change can be reviewed like any
//! commit, and still moves no ref, index entry or file.

use crate::error::CritiqueError;
use crate::fields::Fields;
use crate::runtime::Runtime;
use model_gateway::sha256::sha256_hex;

/// A staged change: the commit it is staged on, and the hash that names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedDiff {
    /// HEAD when the change was read.
    pub head_commit: String,
    /// `staged_diff_hash` of HEAD's tree and the staged entries.
    pub sha256: String,
}

/// A staged change made into a commit a code stage can review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedSnapshot {
    pub staged: StagedDiff,
    /// A commit holding the index, with HEAD as its parent; no ref points at it.
    pub commit: String,
}

const SNAPSHOT_MESSAGE: &str = "critique: the staged change under review";

/// The hash that names one staged change: the tree of the commit it is staged on and every staged entry (mode, object
/// id and path). Restaging any file, or staging on another commit, changes it; neither git's diff settings nor the
/// working tree enter it, so a commit gate computes it the same way prepare did.
pub fn staged_diff_hash(head_tree: &str, staged_entries: &str) -> String {
    sha256_hex(format!("staged-diff v1\0{head_tree}\0{staged_entries}").as_bytes())
}

/// Reads the change staged in the index, on HEAD.
pub fn read_staged(runtime: &Runtime, root: &str) -> Result<StagedDiff, CritiqueError> {
    let head = runtime
        .git
        .resolve_commit(root, "HEAD")
        .map_err(|detail| CritiqueError::new("invalid-input", format!("staged needs a commit at HEAD to diff the index against: {detail}")))?;
    let tree = runtime.git.tree_of(root, &head).map_err(CritiqueError::unexpected)?;
    let entries = runtime.git.staged_entries(root).map_err(CritiqueError::unexpected)?;
    Ok(StagedDiff { head_commit: head, sha256: staged_diff_hash(&tree, &entries) })
}

/// Reads the staged change and writes it as a commit on HEAD.
pub fn snapshot(runtime: &Runtime, root: &str) -> Result<StagedSnapshot, CritiqueError> {
    let staged = read_staged(runtime, root)?;
    let tree = runtime.git.write_index_tree(root).map_err(CritiqueError::unexpected)?;
    let commit = runtime.git.commit_tree(root, &tree, &staged.head_commit, SNAPSHOT_MESSAGE).map_err(CritiqueError::unexpected)?;
    Ok(StagedSnapshot { staged, commit })
}

/// Whether a command's input asks for the staged change: the index, made into a commit on HEAD. It stands in for
/// `head`, and only a ticket review, which runs before its ticket is committed, takes it.
pub fn staged_input(fields: &Fields<'_>, stage: &str) -> Result<bool, StagedInputError> {
    if !fields.has("staged") || !fields.boolean("staged").map_err(StagedInputError::Shape)? {
        return Ok(false);
    }
    if stage != "ticket-review" {
        return Err(StagedInputError::Refused(CritiqueError::new("invalid-input", format!("only ticket-review takes staged; {stage} reviews commits"))));
    }
    if fields.has("head") {
        return Err(StagedInputError::Refused(CritiqueError::new("invalid-input", "staged reviews the index: leave out head")));
    }
    Ok(true)
}

/// Why `staged_input` failed: the input's shape, which the caller words as its own, or an error that is already worded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StagedInputError {
    Shape(crate::error::ShapeError),
    Refused(CritiqueError),
}
