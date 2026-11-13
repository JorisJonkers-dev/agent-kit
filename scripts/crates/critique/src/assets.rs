//! The runner's shipped files (roles, prompts, lenses), read from the folder `assets_root` names.

use crate::error::CritiqueError;
use crate::runtime::Runtime;
use model_gateway::text::join;

/// The text of a shipped file, named by its path under the assets folder.
pub fn read_asset(runtime: &Runtime, relative: &str) -> Result<String, CritiqueError> {
    runtime.files.read_text(&join(&[&runtime.assets_root, relative])).map_err(CritiqueError::from_fs)
}
