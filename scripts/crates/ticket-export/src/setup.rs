//! The repository's setup file: the one machine-readable record of how agents
//! work in a repository, and the tracker repo the export reads from.
//!
//! Only the parts this tool uses are checked here. The full schema belongs to
//! `scripts/repo_setup.py`, whose validator owns it, so extra keys are
//! ignored rather than refused.

use serde_yaml::Value;
use std::path::Path;

/// The setup file, relative to the repository root.
const SETUP_FILE: &str = "docs/agents/setup.yaml";

/// The tracker repo (`OWNER/NAME`) the repository's setup file names.
///
/// A repository without a valid setup file is refused with a message that
/// starts with the file's relative name, so the caller can exit non-zero
/// without calling GitHub.
pub fn tracker_repo(repo_dir: &Path) -> Result<String, String> {
    let text = read(repo_dir)?;
    let data: Value =
        serde_yaml::from_str(&text).map_err(|e| format!("{SETUP_FILE} is not valid YAML: {e}"))?;
    let file = data
        .as_mapping()
        .ok_or_else(|| format!("{SETUP_FILE} must be a mapping"))?;
    match file.get("version").and_then(Value::as_u64) {
        Some(1) => {}
        _ => return Err(format!("{SETUP_FILE}: version must be 1")),
    }
    let tracker = file
        .get("tracker")
        .and_then(Value::as_mapping)
        .ok_or_else(|| format!("{SETUP_FILE}: tracker must be a mapping with kind and repo"))?;
    match tracker.get("kind").and_then(Value::as_str) {
        Some("github") => {}
        _ => return Err(format!("{SETUP_FILE}: tracker.kind must be github")),
    }
    match tracker.get("repo").and_then(Value::as_str) {
        Some(repo) if is_owner_name(repo) => Ok(repo.to_string()),
        _ => Err(format!("{SETUP_FILE}: tracker.repo must be OWNER/NAME")),
    }
}

/// The setup file's text, or the refusal that says why it cannot be read.
fn read(repo_dir: &Path) -> Result<String, String> {
    let path = repo_dir.join(SETUP_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "{SETUP_FILE} is missing in {}; run repo_setup.py write",
                repo_dir.display()
            ))
        }
        Err(e) => return Err(format!("{SETUP_FILE} is not readable: {e}")),
    };
    String::from_utf8(bytes).map_err(|e| format!("{SETUP_FILE} is not UTF-8: {e}"))
}

/// Whether `repo` is `OWNER/NAME`: exactly one slash, each part letters,
/// digits, `_`, `.` or `-`, and neither part only dots (so no `./` or `../`).
fn is_owner_name(repo: &str) -> bool {
    let mut parts = repo.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) => is_name_part(owner) && is_name_part(name),
        _ => false,
    }
}

fn is_name_part(part: &str) -> bool {
    !part.is_empty()
        && !part.chars().all(|c| c == '.')
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

#[cfg(test)]
mod tests;
