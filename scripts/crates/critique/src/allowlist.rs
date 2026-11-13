//! This machine's family allowlist: per repository, the model families its
//! content may go to, and whether that content may be code and its history.

use crate::error::ShapeError;
use crate::fields::Fields;
use model_gateway::json::{self, Object, Value};
use model_gateway::timestamps;

/// The families an allowlist entry may name: every family a client serves.
pub const ALLOWLIST_FAMILIES: [&str; 6] = ["mistral", "zhipu", "deepseek", "google", "anthropic", "openai"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub families: Vec<String>,
    /// Whether the repository may send code, not only documents. An entry recorded before the permission existed reads as false.
    pub code: bool,
    /// Whether the repository may send its history. An entry recorded before the permission existed reads as false.
    pub history: bool,
    pub updated_at: String,
}

/// The allowlist, keyed by `<host>/<owner>/<repo>`, in the order JavaScript enumerates its keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Allowlist {
    pub repositories: Vec<(String, Entry)>,
}

impl Allowlist {
    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.repositories.iter().find(|(name, _)| name == key).map(|(_, entry)| entry)
    }

    /// The allowlist with the key's entry set: in place when the key is there, else last.
    pub fn with_entry(&self, key: &str, entry: Entry) -> Allowlist {
        let mut repositories = self.repositories.clone();
        match repositories.iter_mut().find(|(name, _)| name == key) {
            Some(slot) => slot.1 = entry,
            None => repositories.push((key.to_string(), entry)),
        }
        Allowlist { repositories }
    }

    /// The file's text: `JSON.stringify(allowlist, null, 2)` and a newline.
    pub fn to_text(&self) -> String {
        let mut repositories = Object::new();
        for (key, entry) in &self.repositories {
            let families = Value::Arr(entry.families.iter().map(|family| Value::str(family)).collect());
            repositories.set(
                key,
                json::object(vec![
                    ("code", Value::Bool(entry.code)),
                    ("families", families),
                    ("history", Value::Bool(entry.history)),
                    ("updated_at", Value::str(&entry.updated_at)),
                ]),
            );
        }
        let file = json::object(vec![("version", Value::Num(1.0)), ("repositories", Value::Obj(repositories))]);
        format!("{}\n", json::pretty(&file))
    }
}

/// Reads the allowlist file; an entry naming a family no client serves does not parse.
pub fn parse_allowlist(text: &str) -> Result<Allowlist, ShapeError> {
    let value = json::parse(text).map_err(ShapeError)?;
    let file = Fields::of(&value, "")?;
    let repositories = file.entries("repositories", |entry, _| {
        Ok(Entry {
            code: if entry.has("code") { entry.boolean("code")? } else { false },
            families: entry.one_of_list("families", &ALLOWLIST_FAMILIES)?,
            history: if entry.has("history") { entry.boolean("history")? } else { false },
            updated_at: entry.string("updated_at")?,
        })
    })?;
    file.literal("version", 1.0)?;
    Ok(Allowlist { repositories })
}

/// An entry's `updated_at`: the instant, as `Date.prototype.toISOString` words it.
pub fn instant(now_ms: i64) -> String {
    timestamps::to_iso(now_ms)
}
