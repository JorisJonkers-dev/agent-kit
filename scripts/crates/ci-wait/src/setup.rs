//! The repository's setup file: the one machine-readable record of how agents
//! work in a repository, and the parts this tool reads from it.
//!
//! Only the tracker repo and the flaky jobs are checked here. The full schema
//! belongs to `scripts/repo_setup.py`, whose validator owns it, so extra keys
//! are ignored rather than refused.

use serde_yaml::{Mapping, Value};
use std::path::Path;

/// The setup file, relative to the repository root.
const SETUP_FILE: &str = "docs/agents/setup.yaml";

/// A job the repository has agreed to rerun once instead of failing the wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlakyJob {
    /// The check run's name, as GitHub lists it.
    pub name: String,
    /// Why the job is flaky, printed with the rerun.
    pub reason: String,
}

/// The parts of the setup file this tool reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setup {
    /// The tracker repo (`OWNER/NAME`) the repository's setup file names.
    pub tracker_repo: String,
    /// The jobs a failed run may rerun once.
    pub flaky_jobs: Vec<FlakyJob>,
}

/// The setup file of `repo_dir`.
///
/// A repository without a valid setup file is refused with a message that
/// starts with the file's relative name, so the caller can exit non-zero
/// without calling GitHub.
pub fn load(repo_dir: &Path) -> Result<Setup, String> {
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
    let tracker_repo = match tracker.get("repo").and_then(Value::as_str) {
        Some(repo) if crate::is_owner_name(repo) => repo.to_string(),
        _ => return Err(format!("{SETUP_FILE}: tracker.repo must be OWNER/NAME")),
    };
    let flaky_jobs = flaky(file)?;
    Ok(Setup {
        tracker_repo,
        flaky_jobs,
    })
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

/// The `flaky_jobs` list, each entry with the name and reason this tool needs
/// and the review the schema requires.
fn flaky(file: &Mapping) -> Result<Vec<FlakyJob>, String> {
    let entries = match file.get("flaky_jobs") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Sequence(entries)) => entries,
        Some(_) => return Err(format!("{SETUP_FILE}: flaky_jobs must be a list")),
    };
    let mut jobs = Vec::new();
    for entry in entries {
        let fields = entry.as_mapping().ok_or_else(|| {
            format!("{SETUP_FILE}: a flaky job must be a mapping with name, reason, reviewed_by and reviewed_on")
        })?;
        let name = string(fields, "name")?;
        let reason = string(fields, "reason")?;
        string(fields, "reviewed_by")?;
        string(fields, "reviewed_on")?;
        jobs.push(FlakyJob { name, reason });
    }
    Ok(jobs)
}

/// One non-empty string field of a flaky job entry.
fn string(fields: &Mapping, key: &str) -> Result<String, String> {
    match fields.get(key).and_then(Value::as_str) {
        Some(text) if !text.is_empty() => Ok(text.to_string()),
        _ => Err(format!("{SETUP_FILE}: a flaky job needs a non-empty {key}")),
    }
}

#[cfg(test)]
mod tests;
