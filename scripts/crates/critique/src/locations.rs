//! Where the runner's per-machine files live: in the owner's home, never in a
//! repository, resolved from the environment so a test can put the whole
//! machine in a temporary directory.

use crate::error::CritiqueError;
use std::collections::HashMap;

/// The family allowlist the model gateway reads too, so both resolve it the same way.
pub fn allowlist_path(env: &HashMap<String, String>) -> Result<String, CritiqueError> {
    model_gateway::locations::allowlist_path(env).map_err(|error| CritiqueError::unexpected(error.message))
}

/// One ledger per user, in agent-kit's data directory.
pub fn ledger_path(env: &HashMap<String, String>) -> Result<String, CritiqueError> {
    match env.get("HOME") {
        Some(home) => Ok(model_gateway::text::join(&[home, ".local", "share", "agent-kit", "critique", "ledger.jsonl"])),
        None => Err(CritiqueError::new("unexpected", "required environment variable is missing: HOME")),
    }
}

/// The run's manifest, which `prepare` writes and every later command reads.
pub fn manifest_path(run_dir: &str) -> String {
    crate::paths::join(&[run_dir, "manifest.json"])
}

/// `cold-read.json`, which the cold read writes beside the run and returns.
pub fn cold_read_path(run_dir: &str) -> String {
    crate::paths::join(&[run_dir, "cold-read.json"])
}
