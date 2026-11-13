//! Where the runner's per-machine files live: in the owner's home, never in a
//! repository, resolved from the environment so a test can put the whole
//! machine in a temporary directory.

use crate::error::CritiqueError;
use std::collections::HashMap;

/// The family allowlist the model gateway reads too, so both resolve it the same way.
pub fn allowlist_path(env: &HashMap<String, String>) -> Result<String, CritiqueError> {
    model_gateway::locations::allowlist_path(env).map_err(|error| CritiqueError::new(&error.code, error.message))
}
