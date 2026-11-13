//! Where a draft's refactoring plan lives. The node runner's `application/plan-file.ts`.

use crate::error::CritiqueError;
use crate::paths::join;
use model_gateway::text::resolve;

/// The plan's file name: the same in every place a plan lives.
pub const PLAN_FILE: &str = "refactoring-plan.md";

/// Where a draft's refactoring plan lives, by the repository's tracker convention (ADR-0005). A repository that
/// tracks specs as local markdown keeps it under `.scratch/<draft>/`, beside the specs. Any other keeps it beside the
/// flow state in the git common directory, which no commit holds.
pub fn plan_file_path(root: &str, common_dir: &str, tracker: &str, draft: &str) -> Result<String, CritiqueError> {
    plan_file_path_or_null(root, common_dir, tracker, draft).ok_or_else(|| empty_segment(draft))
}

fn empty_segment(draft: &str) -> CritiqueError {
    CritiqueError::new("invalid-input", format!("the draft name {draft} has an empty or dot segment, so it names no folder"))
}

/// `plan_file_path`, or `None` for a draft name that names no folder.
pub fn plan_file_path_or_null(root: &str, common_dir: &str, tracker: &str, draft: &str) -> Option<String> {
    if draft.split('/').any(|part| part == ".." || part == "." || part.is_empty()) {
        return None;
    }
    Some(if tracker == "local-markdown" {
        join(&[root, ".scratch", draft, PLAN_FILE])
    } else {
        join(&[common_dir, "critique", "plans", &draft.replace('/', "~"), PLAN_FILE])
    })
}

/// A plan path the caller names, made absolute against the repository's root. It must lie in the working tree or in
/// the git common directory, the only two places a plan lives.
pub fn named_plan_path(root: &str, common_dir: &str, path: &str) -> Result<String, CritiqueError> {
    let absolute = if path.starts_with('/') { path.to_string() } else { resolve(root, path) };
    let normal = resolve(root, &absolute);
    if !inside(root, &normal) && !inside(common_dir, &normal) {
        return Err(CritiqueError::new("outside-repository", format!("{path} is outside the repository and its git directory, where a plan lives")));
    }
    Ok(absolute)
}

/// Whether the absolute, normalised `path` is `base` or lies beneath it.
fn inside(base: &str, path: &str) -> bool {
    let base = resolve(base, ".");
    path == base || path.starts_with(&format!("{}/", base.trim_end_matches('/')))
}

/// The draft a flow key names: `draft:<name>`. `None` for any other key.
pub fn draft_of(flow: Option<&str>) -> Option<&str> {
    flow.and_then(|flow| flow.strip_prefix("draft:"))
}

/// `path.relative(from, to)` for two absolute paths.
pub fn relative(from: &str, to: &str) -> String {
    let (from, to) = (resolve(from, "."), resolve(to, "."));
    let (a, b): (Vec<&str>, Vec<&str>) = (from.split('/').filter(|part| !part.is_empty()).collect(), to.split('/').filter(|part| !part.is_empty()).collect());
    let shared = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    std::iter::repeat_n("..", a.len() - shared).chain(b[shared..].iter().copied()).collect::<Vec<_>>().join("/")
}
