//! The refusal of a real run whose headless proof is missing, skipped or failed.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::record::{self, Outcome, Record};
use crate::sha256;

/// A workflow file as the refusal sees it: its id and the hash of its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workflow {
    /// The workflow id, from the authoring wrapper or the top level.
    pub id: String,
    /// The SHA-256 of the workflow file's bytes.
    pub sha256: String,
}

/// Reads a workflow file and hashes its bytes.
pub fn load_workflow(path: &Path) -> Result<Workflow, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot read the workflow {}: {}", path.display(), error))?;
    let digest = sha256::hex(&bytes);
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("the workflow {} is not UTF-8 text", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("the workflow {} is not JSON: {}", path.display(), error))?;
    let id =
        workflow_id(&value).ok_or_else(|| format!("the workflow {} has no id", path.display()))?;
    Ok(Workflow { id, sha256: digest })
}

/// Refuses a real run of `workflow` unless `dir` holds a passing proof of it.
pub fn check(dir: &Path, workflow: &Path, installed_commit: Option<&str>) -> Result<(), String> {
    let workflow = load_workflow(workflow).map_err(|message| format!("refused: {}", message))?;
    let record = record::read_record(dir, &workflow.id)
        .map_err(|message| format!("refused: {}", message))?;
    let record = match record {
        Some(record) => record,
        None => {
            return Err(format!(
                "refused: no headless proof is recorded for workflow `{}`; run `gold-band-proof prove` first",
                workflow.id
            ))
        }
    };
    match record.outcome {
        Outcome::Skipped => {
            return Err(format!(
                "refused: the headless proof of workflow `{}` was skipped: {}",
                workflow.id,
                record
                    .skip_reason
                    .as_deref()
                    .unwrap_or("no reason recorded")
            ))
        }
        Outcome::Failed => {
            return Err(format!(
                "refused: the headless proof of workflow `{}` failed: {}",
                workflow.id,
                failure_detail(&record)
            ))
        }
        Outcome::Passed => {}
    }
    if record.workflow_id != workflow.id {
        return Err(format!(
            "refused: the proof record found for workflow `{}` belongs to workflow `{}`",
            workflow.id, record.workflow_id
        ));
    }
    if record.workflow_sha256 != workflow.sha256 {
        return Err(format!(
            "refused: workflow `{}` changed since its headless proof was recorded",
            workflow.id
        ));
    }
    if let Some(commit) = installed_commit {
        if record.gold_band_commit.as_deref() != Some(commit) {
            return Err(format!(
                "refused: workflow `{}` was proved on Gold Band {}, but {} is installed; prove it again",
                workflow.id,
                record.gold_band_commit.as_deref().unwrap_or("an unknown build"),
                commit
            ));
        }
    }
    if record.scenarios.is_empty() || record.scenarios.iter().any(|scenario| !scenario.passed()) {
        return Err(format!(
            "refused: the proof record of workflow `{}` is not a passing proof",
            workflow.id
        ));
    }
    Ok(())
}

/// The id of a workflow document, from the authoring wrapper or the top level.
fn workflow_id(value: &Value) -> Option<String> {
    let wrapped = value
        .get("workflow")
        .and_then(|wrapper| wrapper.get("id"))
        .and_then(|id| id.as_str());
    let top_level = value.get("id").and_then(|id| id.as_str());
    wrapped.or(top_level).map(|id| id.to_string())
}

/// The first scenario that did not pass, described for the refusal message.
fn failure_detail(record: &Record) -> String {
    match record.scenarios.iter().find(|scenario| !scenario.passed()) {
        Some(scenario) => format!(
            "scenario `{}` expected exit {} but got {}",
            scenario.name,
            scenario.expected_exit,
            exit_text(scenario.exit_code)
        ),
        None => "no scenario recorded a failing exit".to_string(),
    }
}

/// An exit code as the refusal message spells it, `none` when there is none.
fn exit_text(code: Option<i32>) -> String {
    match code {
        Some(code) => code.to_string(),
        None => "none".to_string(),
    }
}
