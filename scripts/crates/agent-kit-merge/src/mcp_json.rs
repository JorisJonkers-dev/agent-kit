//! Merging a rendered MCP fragment into a JSON client config: Antigravity's
//! `mcp_config.json` and OpenCode's `opencode.json`.

use crate::json::{self, Object, Value};
use crate::{backup_once, finish, home, make_parent, repr_list, with_suffix, Env, Failure, Io};
use std::fs;
use std::path::{Path, PathBuf};

/// Antigravity: keys of a managed server that are the user's to set, through `agy mcp disable`.
pub const ANTIGRAVITY_USER_KEYS: &[&str] = &["disabled"];
/// OpenCode: keys of a managed server that are the user's to set.
pub const OPENCODE_USER_KEYS: &[&str] = &["enabled"];
pub const OPENCODE_SCHEMA: &str = "https://opencode.ai/config.json";

pub fn antigravity_default(env: Env) -> PathBuf {
    home(env).join(".gemini").join("config").join("mcp_config.json")
}

pub fn opencode_default(env: Env) -> PathBuf {
    let base = env("XDG_CONFIG_HOME").filter(|base| !base.is_empty()).map(PathBuf::from).unwrap_or_else(|| home(env).join(".config"));
    base.join("opencode").join("opencode.json")
}

/// The fragment's entry, carrying over what the user set on the one it replaces.
pub fn managed_entry(entry: &Value, current: Option<&Value>, user_keys: &[&str]) -> Result<Value, Failure> {
    let Value::Obj(wanted) = entry else {
        return Err(Failure("a server entry is not an object".to_string()));
    };
    let mut merged = wanted.clone();
    if let Some(Value::Obj(current)) = current {
        for key in user_keys {
            if let Some(value) = current.get(key) {
                merged.set(key, value.clone());
            }
        }
    }
    Ok(Value::Obj(merged))
}

/// `x or {}` where x must be an object when it is not falsy.
fn object_or_empty(value: Option<&Value>, what: &str) -> Result<Object, Failure> {
    match value {
        None => Ok(Object::default()),
        Some(Value::Obj(object)) => Ok(object.clone()),
        Some(other) if other.is_falsy() => Ok(Object::default()),
        Some(_) => Err(Failure(format!("{what} is not an object"))),
    }
}

fn document(text: &str) -> Result<Object, Failure> {
    match if text.trim().is_empty() { Value::Obj(Object::default()) } else { json::parse(text)? } {
        Value::Obj(object) => Ok(object),
        _ => Err(Failure("the config is not a JSON object".to_string())),
    }
}

/// The servers the fragment leaves in place followed by the managed ones, in the file's own
/// order, so a re-run writes byte-identical output.
pub fn merged_servers(current: &Object, wanted: &Object, managed: &Object, dropped: &[String]) -> Object {
    let mut merged = Object::default();
    for (name, entry) in current.entries() {
        if !wanted.contains(name) && !dropped.contains(name) {
            merged.set(name, entry.clone());
        }
    }
    for (name, entry) in managed.entries() {
        merged.set(name, entry.clone());
    }
    merged
}

/// What setup would be told if the file did not come back as written, or None.
pub fn verify(tool: &str, managed: &Object, back: &Object, dropped: &[String]) -> Option<String> {
    let missing: Vec<String> = managed.entries().iter().filter(|(name, entry)| back.get(name) != Some(entry)).map(|(name, _)| name.clone()).collect();
    let mut lingering: Vec<String> = dropped.iter().filter(|name| back.contains(name)).cloned().collect();
    lingering.sort();
    lingering.dedup();
    (!missing.is_empty() || !lingering.is_empty())
        .then(|| format!("{tool}: FAILED, not as written: {}, still present: {}", repr_list(&missing), repr_list(&lingering)))
}

fn managed_set(wanted: &Object, current: &Object, dropped: &[String], user_keys: &[&str]) -> Result<Object, Failure> {
    let mut managed = Object::default();
    for (name, entry) in wanted.entries() {
        if !dropped.contains(name) {
            managed.set(name, managed_entry(entry, current.get(name), user_keys)?);
        }
    }
    Ok(managed)
}

/// Merge a rendered `mcpServers` fragment into Antigravity's global MCP config.
pub fn antigravity(fragment: &Path, config: &Path, remove: &[String], io: &mut Io) -> Result<i32, Failure> {
    let fragment = document(&fs::read_to_string(fragment)?)?;
    let wanted = object_or_empty(fragment.get("mcpServers"), "mcpServers")?;
    let text = if config.is_file() { fs::read_to_string(config)? } else { String::new() };
    // A fresh agy install leaves the file behind empty.
    let mut doc = document(&text)?;
    let current = object_or_empty(doc.get("mcpServers"), "mcpServers")?;
    let managed = managed_set(&wanted, &current, remove, ANTIGRAVITY_USER_KEYS)?;
    doc.set("mcpServers", Value::Obj(merged_servers(&current, &wanted, &managed, remove)));

    backup_once(config)?;
    make_parent(config)?;
    fs::write(config, json::dumps(&Value::Obj(doc)) + "\n")?;

    let back = object_or_empty(document(&fs::read_to_string(config)?)?.get("mcpServers"), "mcpServers")?;
    let success = format!("antigravity-merge-mcp: {} managed servers in {}", managed.entries().len(), config.display());
    finish(verify("antigravity-merge-mcp", &managed, &back, remove), success, io)
}

fn opencode_servers(doc: &Object) -> Object {
    match doc.get("mcp").and_then(Value::as_object).and_then(|mcp| mcp.get("servers")) {
        Some(Value::Obj(servers)) => servers.clone(),
        _ => Object::default(),
    }
}

/// Merge a rendered OpenCode fragment into `opencode.json`.
pub fn opencode(fragment: &Path, config: &Path, remove: &[String], io: &mut Io) -> Result<i32, Failure> {
    let wanted = opencode_servers(&document(&fs::read_to_string(fragment)?)?);
    let text = if config.is_file() { fs::read_to_string(config)? } else { String::new() };
    let mut doc = match document(&text) {
        Ok(doc) => doc,
        Err(error) => {
            // opencode.json may hold comments when someone wrote it by hand; a merge
            // that rewrote it would drop them, so it stops instead.
            writeln!(io.err, "opencode-merge-mcp: FAILED, {} is not plain JSON ({}); nothing changed", config.display(), error.0)?;
            return Ok(1);
        }
    };
    let current = opencode_servers(&doc);
    let managed = managed_set(&wanted, &current, remove, OPENCODE_USER_KEYS)?;
    let merged = merged_servers(&current, &wanted, &managed, remove);
    let mut mcp = doc.get("mcp").and_then(Value::as_object).cloned().unwrap_or_default();
    mcp.set("servers", Value::Obj(merged));
    doc.set("mcp", Value::Obj(mcp));
    if !doc.contains("$schema") {
        doc.set("$schema", Value::Str(OPENCODE_SCHEMA.to_string()));
    }

    // The backup is the user's own config from before agent-kit, so it is taken only
    // from a file no merge has touched (none of the managed servers in it), and never
    // overwritten.
    if !wanted.keys().iter().any(|name| current.contains(name)) {
        backup_once(config)?;
    }
    make_parent(config)?;
    // Written beside it and renamed, so a starting OpenCode never reads half a file.
    let staged = with_suffix(config, ".agent-kit-tmp");
    fs::write(&staged, json::dumps(&Value::Obj(doc)) + "\n")?;
    fs::rename(&staged, config)?;

    let back = opencode_servers(&document(&fs::read_to_string(config)?)?);
    let success = format!("opencode-merge-mcp: {} managed servers in {}", managed.entries().len(), config.display());
    finish(verify("opencode-merge-mcp", &managed, &back, remove), success, io)
}
