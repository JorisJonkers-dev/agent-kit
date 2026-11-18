//! The model family of a binding, read from the one routing file the critique
//! pipeline routes with (`skills/workstation/critique/routing.json`), never from
//! a table of a crate's own (#783). `workflow-lint` (the `critic-family` rule) and
//! `gold-band-run-start` (the family allowlist) both resolve through it.
//!
//! The binding's model (provider prefix cut, `.` read as `-`) is matched, longest
//! first and on a whole `-` segment, against each client's `model` and
//! `gateway.model`; when no client names it, the agent id (`claude-work` is
//! `claude`) names a client kind whose clients all share one family (`claude` is
//! `anthropic`). A model neither finds has no family.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

pub mod tier;

const ROUTING: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../skills/workstation/critique/routing.json"
));

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// A model id as the routing file spells it: provider prefix cut, `.` as `-`.
fn normalise(model: &str) -> String {
    let name = model.rsplit('/').next().unwrap_or("");
    name.to_ascii_lowercase().replace('.', "-")
}

/// One client of `routing.json`: kind, family and the model names it answers to.
struct Client {
    kind: String,
    family: String,
    names: Vec<String>,
}

/// The clients and stages of `routing.json`.
pub struct Routing {
    clients: BTreeMap<String, Client>,
    stages: Value,
}

/// A JSON merge patch (RFC 7386), as the critique runner applies a routing override:
/// an object merges key by key, `null` removes a key, anything else replaces.
pub fn merge_patch(target: &mut Value, patch: &Value) {
    let Some(patch) = patch.as_object() else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(serde_json::Map::new());
    }
    let map = target.as_object_mut().expect("made an object above");
    for (key, value) in patch {
        if value.is_null() {
            map.remove(key);
        } else {
            merge_patch(map.entry(key.clone()).or_insert(Value::Null), value);
        }
    }
}

impl Routing {
    /// `routing.json` as shipped, with this machine's override applied over it when one is given.
    pub fn load(machine_override: Option<&Value>) -> Routing {
        let mut root: Value = serde_json::from_str(ROUTING).expect("routing.json is JSON");
        if let Some(patch) = machine_override {
            merge_patch(&mut root, patch);
        }
        let mut clients = BTreeMap::new();
        let shipped = serde_json::Map::new();
        for (key, client) in root["clients"].as_object().unwrap_or(&shipped) {
            let names = [text(client, "model"), text(&client["gateway"], "model")]
                .into_iter()
                .filter(|n| !n.is_empty())
                .map(normalise)
                .collect();
            clients.insert(
                key.clone(),
                Client {
                    kind: text(client, "kind").to_string(),
                    family: text(client, "family").to_string(),
                    names,
                },
            );
        }
        Routing {
            clients,
            stages: root["stages"].clone(),
        }
    }

    /// The family of an agent id and model id, if the routing file can tell.
    pub fn family(&self, agent: &str, model: &str) -> Option<String> {
        let model = normalise(model);
        let named = self
            .clients
            .values()
            .flat_map(|c| c.names.iter().map(move |n| (n, &c.family)))
            .filter(|(name, _)| {
                model == **name
                    || model
                        .strip_prefix(name.as_str())
                        .is_some_and(|rest| rest.starts_with('-'))
            })
            .max_by_key(|(name, _)| name.len());
        if let Some((_, family)) = named {
            return Some(family.clone());
        }
        let kind = agent.split(['-', '+']).next().unwrap_or("");
        let families: BTreeSet<&String> = self
            .clients
            .values()
            .filter(|c| c.kind == kind)
            .map(|c| &c.family)
            .collect();
        if families.len() == 1 {
            families.into_iter().next().cloned()
        } else {
            None
        }
    }

    /// The clients (key, family) a stage routes to in the given slots.
    pub fn roster(&self, stage: &str, slots: [&str; 2]) -> Vec<(String, String)> {
        let mut roster = Vec::new();
        for slot in slots {
            let entries = self.stages[stage][slot]
                .as_array()
                .map_or(&[][..], Vec::as_slice);
            roster.extend(entries.iter().filter_map(|entry| {
                let key = text(entry, "client");
                self.clients
                    .get(key)
                    .map(|client| (key.to_string(), client.family.clone()))
            }));
        }
        roster
    }
}

#[cfg(test)]
mod tests;
