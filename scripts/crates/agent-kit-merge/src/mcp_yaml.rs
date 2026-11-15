//! Merging a rendered `mcp_servers:` fragment into the local Hermes config
//! (`~/.hermes/config.yaml`), preserving every other key.

use crate::{copy_keeping, finish, home, make_parent, with_suffix, Env, Failure, Io};
use serde_yaml::{Mapping, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub fn default_config(env: Env) -> PathBuf {
    match env("HERMES_CONFIG").map(|value| value.trim().to_string()).filter(|value| !value.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => home(env).join(".hermes").join("config.yaml"),
    }
}

fn key(name: &str) -> Value {
    Value::String(name.to_string())
}

/// The config at `path` as a mapping; a missing, empty or non-mapping file reads as empty.
pub fn load_config(path: &Path) -> Result<Mapping, Failure> {
    if !path.is_file() {
        return Ok(Mapping::new());
    }
    let text = fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(Mapping::new());
    }
    match serde_yaml::from_str::<Value>(&text).map_err(|e| Failure(e.to_string()))? {
        Value::Mapping(mapping) => Ok(mapping),
        _ => Ok(Mapping::new()),
    }
}

fn servers_of(config: &Mapping) -> Option<&Mapping> {
    config.get(key("mcp_servers")).and_then(Value::as_mapping)
}

/// A copy of `target` with the fragment's `mcp_servers` merged in.
pub fn merge_fragment(fragment: &Mapping, target: &Mapping) -> Result<Mapping, Failure> {
    let mut result = target.clone();
    let mut servers = match result.get(key("mcp_servers")) {
        None => Mapping::new(),
        Some(Value::Mapping(servers)) => servers.clone(),
        Some(_) => return Err(Failure("existing config has a non-mapping 'mcp_servers' key".to_string())),
    };
    if let Some(wanted) = servers_of(fragment) {
        for (name, entry) in wanted {
            servers.insert(name.clone(), entry.clone());
        }
    }
    result.insert(key("mcp_servers"), Value::Mapping(servers));
    Ok(result)
}

/// A copy of `target` with the named `mcp_servers` removed.
pub fn remove_names(names: &[String], target: &Mapping) -> Mapping {
    let mut result = target.clone();
    if let Some(Value::Mapping(servers)) = result.get_mut(key("mcp_servers")) {
        for name in names {
            servers.shift_remove(key(name));
        }
    }
    result
}

fn present(config: &Mapping) -> Vec<String> {
    servers_of(config).map(|servers| servers.keys().filter_map(|k| k.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// What setup would be told if the config did not come back as written, or None.
pub fn verify(remove: &[String], deployed: &[String], reread: &[String]) -> Option<String> {
    if !remove.is_empty() {
        let mut still: Vec<String> = remove.iter().filter(|name| reread.contains(name)).cloned().collect();
        still.sort();
        still.dedup();
        return (!still.is_empty()).then(|| format!("verification failed; still present after remove: {}", crate::repr_list(&still)));
    }
    let mut missing: Vec<String> = deployed.iter().filter(|name| !reread.contains(name)).cloned().collect();
    missing.sort();
    (!missing.is_empty()).then(|| format!("verification failed; missing after merge: {}", crate::repr_list(&missing)))
}

/// `hermes-merge-mcp.py <fragment.yaml> [--config PATH] [--remove NAME ...]`.
pub fn hermes(fragment_path: &Path, config_path: &Path, remove: &[String], io: &mut Io) -> Result<i32, Failure> {
    if !fragment_path.is_file() {
        writeln!(io.err, "fragment not found: {}", fragment_path.display())?;
        return Ok(1);
    }
    let fragment = serde_yaml::from_str::<Value>(&fs::read_to_string(fragment_path)?).map_err(|e| Failure(e.to_string()))?;
    let Some(fragment) = fragment.as_mapping().filter(|m| m.contains_key(key("mcp_servers"))) else {
        writeln!(io.err, "fragment must be a mapping with an 'mcp_servers' key")?;
        return Ok(1);
    };

    let deployed = present(fragment);
    make_parent(config_path)?;
    let target = load_config(config_path)?;
    let merged = if remove.is_empty() { merge_fragment(fragment, &target)? } else { remove_names(remove, &target) };

    // Back up the previous config so a bad edit is reversible.
    if config_path.is_file() {
        copy_keeping(config_path, &with_suffix(config_path, ".bak"))?;
    }
    let changed = merged != target;
    fs::write(config_path, serde_yaml::to_string(&merged).unwrap_or_default())?;

    // Verify the VALUE: read back and confirm every fragment server is present.
    let reread = present(&load_config(config_path)?);
    let success = if remove.is_empty() {
        let verb = if changed { "merged" } else { "already present; no change" };
        format!("hermes: {verb} {} MCP server(s) into {}", deployed.len(), config_path.display())
    } else {
        let mut names = remove.to_vec();
        names.sort();
        let verb = if changed { "removed" } else { "already absent" };
        format!("hermes: {verb} MCP server(s) {} from {}", crate::repr_list(&names), config_path.display())
    };
    finish(verify(remove, &deployed, &reread), success, io)
}
