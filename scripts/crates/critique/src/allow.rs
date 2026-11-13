//! `allow`: records which model families this machine lets a repository send
//! content to, whether that content may be code, and whether it may be history.
//! `code` or `history` left out keeps the entry's current permission, which is
//! off for a new entry.

use crate::allowlist::{instant, Entry, ALLOWLIST_FAMILIES};
use crate::error::CritiqueError;
use crate::fields::Fields;
use crate::locations::allowlist_path;
use crate::repository::{read_allowlist, resolve, Resolution};
use crate::runtime::Runtime;
use model_gateway::json::{self, Value};

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let read = || -> Result<_, crate::error::ShapeError> {
        let fields = Fields::of(raw, "")?;
        let code = if fields.has("code") { Some(fields.boolean("code")?) } else { None };
        let families = fields.one_of_list("families", &ALLOWLIST_FAMILIES)?;
        let history = if fields.has("history") { Some(fields.boolean("history")?) } else { None };
        Ok((code, families, history, fields.string("repo")?))
    };
    let (code, families, history, repo) = read().map_err(|error| error.into_error("invalid-input", "allow input"))?;
    let repository = match resolve(runtime, &repo)? {
        Resolution::Refused(refusal) => return Ok(refusal),
        Resolution::Resolved(repository) => repository,
    };
    let path = allowlist_path(&runtime.env)?;
    let allowlist = read_allowlist(runtime)?;
    let current = allowlist.get(&repository.key);
    let code = code.or(current.map(|entry| entry.code)).unwrap_or(false);
    let history = history.or(current.map(|entry| entry.history)).unwrap_or(false);
    let entry = Entry { families: families.clone(), code, history, updated_at: instant(runtime.clock.now_ms()) };
    let next = allowlist.with_entry(&repository.key, entry);
    runtime.files.write_text(&path, &next.to_text()).map_err(CritiqueError::from_fs)?;
    Ok(json::object(vec![
        ("allowlist_path", Value::str(&path)),
        ("code", Value::Bool(code)),
        ("families", Value::Arr(families.iter().map(|family| Value::str(family)).collect())),
        ("history", Value::Bool(history)),
        ("repository", Value::str(&repository.key)),
        ("status", Value::str("allowed")),
    ]))
}
