//! The repository's setup file: the one machine-readable record of how agents
//! work in a repository, and the label vocabulary a ticket may use.
//!
//! Only the parts this tool uses are checked here. The full schema belongs to
//! `scripts/repo_setup.py`, whose validator owns it, so extra keys are
//! ignored rather than refused. The tracker repo and the version come from
//! `ticket_export::setup::tracker_repo`, which refuses a file this tool could
//! not publish to.

use serde_yaml::Value;
use std::path::Path;

/// The setup file, relative to the repository root.
const SETUP_FILE: &str = "docs/agents/setup.yaml";

/// The label vocabulary the repository's setup file lists.
///
/// A ticket may carry only these labels, so the list must be non-empty and
/// every name a non-empty string. A repository without a readable setup file
/// is refused with a message that starts with the file's relative name.
pub fn labels(repo_dir: &Path) -> Result<Vec<String>, String> {
    let text = read(repo_dir)?;
    let data: Value =
        serde_yaml::from_str(&text).map_err(|e| format!("{SETUP_FILE} is not valid YAML: {e}"))?;
    let file = data
        .as_mapping()
        .ok_or_else(|| format!("{SETUP_FILE} must be a mapping"))?;
    let list = file
        .get("labels")
        .and_then(Value::as_sequence)
        .ok_or_else(|| format!("{SETUP_FILE}: labels must be a non-empty list of names"))?;
    let mut names = Vec::new();
    for item in list {
        match item.as_str() {
            Some(name) if !name.trim().is_empty() => names.push(name.to_string()),
            _ => {
                return Err(format!(
                    "{SETUP_FILE}: labels must be a non-empty list of names"
                ))
            }
        }
    }
    if names.is_empty() {
        return Err(format!(
            "{SETUP_FILE}: labels must be a non-empty list of names"
        ));
    }
    Ok(names)
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

#[cfg(test)]
mod tests;
