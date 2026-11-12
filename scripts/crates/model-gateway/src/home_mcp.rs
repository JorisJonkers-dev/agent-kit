//! The MCP servers a client's home declares, read so a run can switch each of them off.

use crate::error::GatewayError;
use crate::json::Value;
use crate::ports::Files;
use crate::text::join;
use crate::toml::read_toml;
use std::collections::HashMap;

/// The MCP servers Vibe's home declares, by name, so a run can switch each of
/// them off: `~/.vibe/config.toml`. Vibe is the one client whose home the
/// gateway reads, and a home with no configuration declares none. A home the
/// gateway cannot read or parse is refused rather than read as declaring
/// nothing: a run that asked to switch the home's servers off would otherwise
/// leave every one of them live.
pub fn home_mcp_servers(files: &dyn Files, env: &HashMap<String, String>) -> Result<Vec<String>, GatewayError> {
    let Some(home) = env.get("HOME") else {
        return Err(GatewayError::new("unexpected", "required environment variable is missing: HOME"));
    };
    let path = join(&[home, ".vibe", "config.toml"]);
    if !files.exists(&path).map_err(GatewayError::from_fs)? {
        return Ok(Vec::new());
    }
    let unreadable = |words: String| GatewayError::new("home-config-unreadable", format!("{path}: cannot read the home's MCP servers: {words}"));
    let text = files.read_text(&path).map_err(|error| unreadable(error.message))?;
    let document = read_toml(&text).map_err(unreadable)?;
    Ok(document.as_object().and_then(|table| table.get("mcp_servers")).map(names_of).unwrap_or_default())
}

/// The `name` of each server in an `mcp_servers` value, leaving out one without a name.
fn names_of(servers: &Value) -> Vec<String> {
    let name = |server: &Value| server.as_object().and_then(|table| table.get("name")).and_then(Value::as_str).map(str::to_string);
    match servers {
        Value::Arr(servers) => servers.iter().filter_map(name).collect(),
        single => name(single).into_iter().collect(),
    }
}
