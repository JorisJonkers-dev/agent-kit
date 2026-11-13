//! A change between two commits, as git reports it: the commits, the diff and what the risk rules read. Read from git
//! alone, with no model and no network: the diff from the merge base of `base` and `head` to `head`, and every path
//! it touches. A file routing's exclusion globs match, or whose first lines say a tool generated it (at the head, or
//! at the base for a deleted file), keeps one line in the diff instead of its hunks and adds nothing to the
//! changed-line count. A code packet and risk triage read a change the same way.

use crate::diff_filter::{filter_diff, LeftOutFile};
use crate::error::CritiqueError;
use crate::repository_packet::{exclusion_of, exclusion_reason, PacketLimits};
use crate::runtime::Runtime;
use crate::triage::{code_change, CodeChange};
use std::collections::HashMap;

/// A change between two commits.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadChange {
    /// Where the change left its base: the merge base of the base and the head.
    pub base_commit: String,
    pub head_commit: String,
    /// The diff, without the hunks of the files `left_out` names.
    pub diff: String,
    /// Every path is touched; only the lines of files not left out are counted.
    pub change: CodeChange,
    /// The generated, vendored and lock files whose hunks the diff leaves out.
    pub left_out: Vec<LeftOutFile>,
}

/// Reads the change from the merge base of `base` and `head` to `head`.
pub fn read_change(runtime: &Runtime, root: &str, base: &str, head: &str, limits: &PacketLimits) -> Result<ReadChange, CritiqueError> {
    let head_commit = commit(runtime, root, "head", head)?;
    let base_commit = runtime.git.merge_base(root, &commit(runtime, root, "base", base)?, &head_commit).map_err(CritiqueError::unexpected)?;
    let raw = runtime.git.diff_commits(root, &base_commit, &head_commit).map_err(CritiqueError::unexpected)?;
    let paths = runtime.git.changed_paths(root, &base_commit, &head_commit).map_err(CritiqueError::unexpected)?;
    let mut reasons: HashMap<String, String> = HashMap::new();
    for path in &paths {
        let text = if excluded(path, None, limits)?.is_none() { Some(text_of(runtime, root, &head_commit, &base_commit, path)?) } else { None };
        if let Some(exclusion) = excluded(path, text.as_deref(), limits)? {
            reasons.insert(path.clone(), exclusion_reason(&exclusion));
        }
    }
    let filtered = filter_diff(&raw, &|path| reasons.get(path).cloned());
    Ok(ReadChange { base_commit, change: code_change(&filtered.diff, &paths), diff: filtered.diff, head_commit, left_out: filtered.left_out })
}

fn excluded(path: &str, text: Option<&str>, limits: &PacketLimits) -> Result<Option<crate::repository_packet::Exclusion>, CritiqueError> {
    exclusion_of(path, text, limits).map_err(CritiqueError::unexpected)
}

/// The path's text at the head, or at the base when the change deleted it.
fn text_of(runtime: &Runtime, root: &str, head: &str, base: &str, path: &str) -> Result<String, CritiqueError> {
    runtime.git.file_at(root, head, path).or_else(|_| runtime.git.file_at(root, base, path)).map_err(CritiqueError::unexpected)
}

fn commit(runtime: &Runtime, root: &str, field: &str, revision: &str) -> Result<String, CritiqueError> {
    runtime.git.resolve_commit(root, revision).map_err(|detail| CritiqueError::new("invalid-input", format!("{field} {revision} names no commit: {detail}")))
}
