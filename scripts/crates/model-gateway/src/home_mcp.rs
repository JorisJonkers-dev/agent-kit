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

/// The names of every MCP server a client's home declares, for the matrix's escape probe: OpenCode's
/// `opencode.json`, Hermes' `config.yaml`, Codex's `config.toml` and Vibe's `config.toml`. A file that is not
/// there or does not parse declares none; this reads, it never refuses.
pub fn declared_servers(files: &dyn Files, env: &HashMap<String, String>) -> Vec<String> {
    let home = env.get("HOME").map_or("", String::as_str);
    let read = |path: String| files.read_text(&path).ok();
    let mut names: Vec<String> = Vec::new();
    let config = env.get("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()).cloned().unwrap_or_else(|| join(&[home, ".config"]));
    if let Some(Value::Obj(document)) = read(join(&[&config, "opencode", "opencode.json"])).and_then(|text| crate::json::parse(&text).ok()) {
        if let Some(Value::Obj(mcp)) = document.get("mcp") {
            let servers = match mcp.get("servers") {
                Some(Value::Obj(servers)) => servers,
                _ => mcp,
            };
            names.extend(servers.entries().into_iter().map(|(name, _)| name.to_string()));
        }
    }
    let hermes = env.get("HERMES_HOME").filter(|dir| !dir.is_empty()).cloned().unwrap_or_else(|| join(&[home, ".hermes"]));
    if let Some(text) = read(join(&[&hermes, "config.yaml"])) {
        names.extend(crate::run_home::yaml_children(&text, "mcp_servers"));
    }
    let codex = env.get("CODEX_HOME").filter(|dir| !dir.is_empty()).cloned().unwrap_or_else(|| join(&[home, ".codex"]));
    if let Some(Value::Obj(document)) = read(join(&[&codex, "config.toml"])).and_then(|text| read_toml(&text).ok()) {
        if let Some(Value::Obj(servers)) = document.get("mcp_servers") {
            names.extend(servers.entries().into_iter().map(|(name, _)| name.to_string()));
        }
    }
    names.extend(home_mcp_servers(files, env).unwrap_or_default());
    names.sort();
    names.dedup();
    names
}
