//! Where the runner's per-machine files live: in the owner's home, never in a
//! repository, resolved from the environment so a test can put the whole
//! machine in a temporary directory.

use crate::error::CritiqueError;
use std::collections::HashMap;

/// The family allowlist the model gateway reads too, so both resolve it the same way.
pub fn allowlist_path(env: &HashMap<String, String>) -> Result<String, CritiqueError> {
    model_gateway::locations::allowlist_path(env).map_err(|error| CritiqueError::unexpected(error.message))
}

/// A path under the owner's home; a missing `HOME` is the error the node runner's `env.require` throws.
fn in_home(env: &HashMap<String, String>, parts: &[&str]) -> Result<String, CritiqueError> {
    match env.get("HOME") {
        Some(home) => Ok(model_gateway::text::join(&[&[home.as_str()], parts].concat())),
        None => Err(CritiqueError::new("unexpected", "required environment variable is missing: HOME")),
    }
}

/// One ledger per user, in agent-kit's data directory.
pub fn ledger_path(env: &HashMap<String, String>) -> Result<String, CritiqueError> {
    in_home(env, &[".local", "share", "agent-kit", "critique", "ledger.jsonl"])
}

/// This machine's critique configuration, which holds the per-machine kill switch.
pub fn critique_config_path(env: &HashMap<String, String>) -> Result<String, CritiqueError> {
    in_home(env, &[".config", "agent-kit", "critique.json"])
}

/// Where the gates and the session-start hook log what they could not decide.
pub fn gate_log_path(env: &HashMap<String, String>) -> Result<String, CritiqueError> {
    in_home(env, &[".local", "share", "agent-kit", "critique", "gate.log.jsonl"])
}

/// The results `/flow setup` recorded for this machine.
pub fn setup_results_path(env: &HashMap<String, String>) -> Result<String, CritiqueError> {
    in_home(env, &[".local", "share", "agent-kit", "critique", "machine-setup.json"])
}

/// The run's manifest, which `prepare` writes and every later command reads.
pub fn manifest_path(run_dir: &str) -> String {
    crate::paths::join(&[run_dir, "manifest.json"])
}

/// `cold-read.json`, which the cold read writes beside the run and returns.
pub fn cold_read_path(run_dir: &str) -> String {
    crate::paths::join(&[run_dir, "cold-read.json"])
}

/// Written by record, so its presence means the run's verdicts are recorded.
pub fn report_path(run_dir: &str) -> String {
    crate::paths::join(&[run_dir, "report.md"])
}

/// `findings.json`, which fan-out writes into the run and record reads back.
pub fn findings_path(run_dir: &str) -> String {
    crate::paths::join(&[run_dir, "findings.json"])
}

/// `refute.json`, which refute writes beside the run and record reads.
pub fn refute_path(run_dir: &str) -> String {
    crate::paths::join(&[run_dir, "refute.json"])
}
