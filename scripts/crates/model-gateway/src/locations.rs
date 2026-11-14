//! Where the gateway's files live: this machine's override of the shipped
//! defaults, the family allowlist, the grants, Vibe's own configuration and the
//! gateway's state. Every path is resolved from the environment, so a test can
//! put the whole machine in a temporary directory.

use crate::error::GatewayError;
use crate::state_file::state_dir;
use crate::text::join;
use std::collections::HashMap;

/// `$XDG_CONFIG_HOME/agent-kit`, or `~/.config/agent-kit` when XDG_CONFIG_HOME
/// is unset or not an absolute path.
pub fn config_dir(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    if let Some(xdg) = env.get("XDG_CONFIG_HOME") {
        if xdg.starts_with('/') {
            return Ok(join(&[xdg, "agent-kit"]));
        }
    }
    Ok(join(&[require_home(env)?, ".config", "agent-kit"]))
}

/// This machine's override of the shipped defaults.
pub fn override_path(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    Ok(join(&[&config_dir(env)?, "model-gateway.json"]))
}

/// The family allowlist critique writes and the gateway reads.
pub fn allowlist_path(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    Ok(join(&[&config_dir(env)?, "critique-allowlist.json"]))
}

/// The grants the owner gave, in the machine configuration beside the override.
pub fn grants_path(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    Ok(join(&[&config_dir(env)?, "model-gateway-grants.json"]))
}

/// The configuration Vibe declares its own models in: `$VIBE_HOME/config.toml`,
/// or `~/.vibe/config.toml` when VIBE_HOME is unset or not an absolute path.
pub fn vibe_config_path(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    if let Some(home) = env.get("VIBE_HOME") {
        if home.starts_with('/') {
            return Ok(join(&[home, "config.toml"]));
        }
    }
    Ok(join(&[require_home(env)?, ".vibe", "config.toml"]))
}

/// Where the gateway keeps what the Vibe probe answered:
/// `$XDG_CACHE_HOME/model-gateway/vibe-available.json`, or
/// `~/.cache/model-gateway/vibe-available.json` when XDG_CACHE_HOME is unset or
/// not an absolute path.
pub fn vibe_available_cache_path(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    let root = match env.get("XDG_CACHE_HOME") {
        Some(xdg) if xdg.starts_with('/') => xdg.clone(),
        _ => join(&[require_home(env)?, ".cache"]),
    };
    Ok(join(&[&root, "model-gateway", "vibe-available.json"]))
}

/// The live LiteLLM model catalogue setup fetches: `$XDG_CACHE_HOME/agent-kit/litellm-catalogue.json`, or
/// `~/.cache/agent-kit/litellm-catalogue.json` when XDG_CACHE_HOME is unset or not an absolute path.
pub fn litellm_catalogue_path(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    let root = match env.get("XDG_CACHE_HOME") {
        Some(xdg) if xdg.starts_with('/') => xdg.clone(),
        _ => join(&[require_home(env)?, ".cache"]),
    };
    Ok(join(&[&root, "agent-kit", "litellm-catalogue.json"]))
}

/// The machine-wide ledger: one JSON line per run, whichever repository it ran in.
pub fn ledger_path(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    Ok(join(&[&state_dir(env)?, "ledger.jsonl"]))
}

/// The gateway's run directories for runs outside a repository: one directory per run under the state directory.
pub fn runs_dir(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    Ok(join(&[&state_dir(env)?, "runs"]))
}

/// This machine's routing override, applied client by client over the routing critique ships. It is resolved
/// the way critique resolves it, so the gateway reads the routing critique really runs.
pub fn routing_override_path(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    Ok(join(&[require_home(env)?, ".config", "agent-kit", "critique-routing.json"]))
}

/// Setup's recorded results, where critique's setup writes them: one result per check id.
pub fn setup_results_path(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    Ok(join(&[require_home(env)?, ".local", "share", "agent-kit", "critique", "machine-setup.json"]))
}

/// `env.require('HOME')`: the same error the state directory gives.
fn require_home(env: &HashMap<String, String>) -> Result<&str, GatewayError> {
    match env.get("HOME") {
        Some(home) => Ok(home),
        None => Err(GatewayError::new("unexpected", "required environment variable is missing: HOME")),
    }
}
