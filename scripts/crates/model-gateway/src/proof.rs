//! What setup and critique left on this machine that a read-only proof is read
//! from: critique's shipped routing, this machine's routing override and setup's
//! recorded results. It reads files only: it runs no client and spends nothing,
//! so a run can be refused before it launches.

use crate::error::GatewayError;
use crate::locations;
use crate::ports::Runtime;
use crate::run::ProofFiles;

/// The proof files as they stand; each is `None` when it is not there.
pub fn read_proof_files(runtime: &Runtime) -> Result<ProofFiles, GatewayError> {
    Ok(ProofFiles {
        routing: Some(runtime.routing.clone()),
        routing_override: read_if_present(runtime, &locations::routing_override_path(&runtime.env)?)?,
        setup_results: read_if_present(runtime, &locations::setup_results_path(&runtime.env)?)?,
    })
}

/// The file's text, or `None` when it is not there.
fn read_if_present(runtime: &Runtime, path: &str) -> Result<Option<String>, GatewayError> {
    if !runtime.files.exists(path).map_err(GatewayError::from_fs)? {
        return Ok(None);
    }
    runtime.files.read_text(path).map(Some).map_err(GatewayError::from_fs)
}
