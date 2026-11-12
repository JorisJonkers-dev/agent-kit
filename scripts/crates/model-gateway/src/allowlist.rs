//! The family allowlist critique writes and the gateway reads: per repository,
//! the model families its content may go to and whether that content may be
//! code and its history.

use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::ports::Runtime;
use std::collections::BTreeMap;

/// One repository's entry in this machine's family allowlist. An entry recorded
/// before a permission existed reads as false.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllowlistEntry {
    pub families: Vec<String>,
    pub code: bool,
    pub history: bool,
}

/// The family allowlist, keyed by `<host>/<owner>/<repo>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyAllowlist {
    pub repositories: BTreeMap<String, AllowlistEntry>,
}

impl FamilyAllowlist {
    /// The allowlist of a machine that has written none.
    pub fn empty() -> FamilyAllowlist {
        FamilyAllowlist { repositories: BTreeMap::new() }
    }
}

/// Reads parsed JSON as the family allowlist, or says what is wrong with it.
pub fn parse_allowlist(raw: &Value) -> Result<FamilyAllowlist, GatewayError> {
    let Some(object) = raw.as_object() else {
        return Err(invalid("the allowlist must be a JSON object"));
    };
    if object.get("version") != Some(&Value::Num(1.0)) {
        return Err(invalid("version must be 1"));
    }
    let Some(repositories) = object.get("repositories").and_then(Value::as_object) else {
        return Err(invalid("repositories must be an object keyed by repository"));
    };
    let mut parsed = BTreeMap::new();
    for (key, value) in repositories.entries() {
        let Some(entry) = value.as_object() else {
            return Err(invalid(format!("repositories.{key} must be an object")));
        };
        parsed.insert(key.to_string(), parse_entry(key, entry)?);
    }
    Ok(FamilyAllowlist { repositories: parsed })
}

fn parse_entry(key: &str, raw: &Object) -> Result<AllowlistEntry, GatewayError> {
    let where_ = format!("repositories.{key}");
    let code = match raw.get("code") {
        None => false,
        Some(value) => require_boolean(value, &format!("{where_}.code"))?,
    };
    let families = require_strings(raw.get("families"), &format!("{where_}.families"))?;
    let history = match raw.get("history") {
        None => false,
        Some(value) => require_boolean(value, &format!("{where_}.history"))?,
    };
    Ok(AllowlistEntry { families, code, history })
}

fn require_strings(value: Option<&Value>, where_: &str) -> Result<Vec<String>, GatewayError> {
    let Some(Value::Arr(items)) = value else {
        return Err(invalid(format!("{where_} must be a list of non-empty strings")));
    };
    let mut strings = Vec::new();
    for item in items {
        match item {
            Value::Str(text) if !text.is_empty() => strings.push(text.clone()),
            _ => return Err(invalid(format!("{where_} must be a list of non-empty strings"))),
        }
    }
    Ok(strings)
}

fn require_boolean(value: &Value, where_: &str) -> Result<bool, GatewayError> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        _ => Err(invalid(format!("{where_} must be true or false"))),
    }
}

/// This machine's family allowlist, the one critique writes; a missing file is
/// an empty one.
pub fn load_allowlist(runtime: &Runtime) -> Result<FamilyAllowlist, GatewayError> {
    let path = crate::locations::allowlist_path(&runtime.env)?;
    match runtime.files.exists(&path) {
        Ok(false) => return Ok(FamilyAllowlist::empty()),
        Ok(true) => {}
        Err(error) => return Err(invalid(format!("{path}: cannot read the family allowlist: {}", error.message))),
    }
    let text = match runtime.files.read_text(&path) {
        Ok(text) => text,
        Err(error) => return Err(invalid(format!("{path}: cannot read the family allowlist: {}", error.message))),
    };
    let raw = json::parse(&text).map_err(|message| invalid(format!("{path}: not JSON: {message}")))?;
    parse_allowlist(&raw).map_err(|error| invalid(format!("{path}: {}", error.message)))
}

fn invalid(message: impl Into<String>) -> GatewayError {
    GatewayError::new("invalid-config", message)
}
