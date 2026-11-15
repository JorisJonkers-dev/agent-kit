//! The `critique` rule group (#793): every artifact a template has a model
//! write is reviewed by critics of another model family before the run ends.
//!
//! * `critic-required`: every path from a **writer** node to `$end` crosses a
//!   **critique record node**;
//! * `critic-family`: no candidate of a **critic** node is of the family of the
//!   writer whose artifact it reviews.
//!
//! The vocabulary is derived from what the shipped templates already carry;
//! Gold Band's editor keeps no field for it and the lint adds none.
//!
//! * A **writer** is a `worker` whose `output.artifact` key names an artifact
//!   a model writes (the key, minus a leading `repair-<N>-`, ends in
//!   `-write`, `-draft`, `-summary`, `-update`, `-record`, `-implementation`
//!   or `-written`) and whose binding has a known model family. Gate and
//!   shim nodes (`stub-model`, `pinned-command`) have no family, so they never
//!   write; neither do the question rounds (`...-open-questions`) and the
//!   verdict nodes (`...-open-findings`).
//! * A **critique record node** is the runner chain's `gate-critique-<stage>-record`
//!   (what the stage's refute and record shims close) or workflow-b's
//!   `verdict-<N>` (the findings check's verdict). Edges of either outcome count.
//! * A run-start refusal ends a run without handing the artifact on: the work
//!   variants route a refused grant to a node with the id `grant-refused`, and
//!   paths through it do not count as reaching `$end`.
//! * A **critic** is one of: an `ai-dynamic` node (its candidates are the
//!   `availableAgents`); a worker bound to a review profile (anything but
//!   `pf-builtin-dev`; its candidate is its own binding); or the runner chain's
//!   `gate-critique-<stage>-fan-out` (its candidates are the clients of that
//!   stage in `skills/workstation/critique/routing.json`, slots `base` and
//!   `deep` for the personal instance and `work` and `deep` for the work one).
//! * The runner never routes a critic of the **orchestrator's** family: `prepare`
//!   takes the orchestrator's family and drops every client of it from the roster.
//!   The stage's orchestrator is the worker `<stage>-verdicts`, so the fan-out's
//!   candidates leave out that node's family. A template that binds its writer and
//!   its orchestrator to one family therefore never meets a same-family critic, and
//!   that is how a template excludes the writer's family from a roster (#796). A second
//!   critique round of a stage appends `-2` to every id of the chain
//!   (`gate-critique-spec-fan-out-2`, `gate-critique-spec-record-2`, orchestrator
//!   `spec-verdicts-2`) and is the same stage with the same roster.
//! * The writer a critic reviews is each **nearest** writer upstream of it: a
//!   writer with a path to the critic that crosses no other writer.
//!
//! The family of a binding is read from the same `routing.json` the critique
//! pipeline routes with, never from a table of its own: the binding's model
//! (provider prefix cut, `.` read as `-`) is matched, longest first and on a
//! whole `-` segment, against each client's `model` and `gateway.model`; when
//! no client names it, the agent id (`claude-work` is `claude`) names a client
//! kind whose clients all share one family (`claude` is `anthropic`). A model
//! neither finds has no family and is never compared.
//!
//! Each finding reads `<node id>: <rule>: <message>`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::Value;

use crate::Instance;

const ROUTING: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../skills/workstation/critique/routing.json"
));
const END_NODE: &str = "$end";
const REFUSAL_NODE: &str = "grant-refused";
const GENERIC_PROFILE: &str = "pf-builtin-dev";
const WRITER_TOKENS: [&str; 7] = [
    "write",
    "draft",
    "summary",
    "update",
    "record",
    "implementation",
    "written",
];

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
struct Routing {
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
    fn load(machine_override: Option<&Value>) -> Routing {
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
    fn family(&self, agent: &str, model: &str) -> Option<String> {
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

    /// The clients (key, family) a stage routes to on this instance.
    fn roster(&self, stage: &str, instance: Instance) -> Vec<(String, String)> {
        let slots = match instance {
            Instance::Work => ["work", "deep"],
            Instance::Personal => ["base", "deep"],
        };
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

/// The agent id and model a worker is bound to: its slot's wrapper binding,
/// else its own `provider` and `model`.
fn binding(node: &Value, bindings: &[Value]) -> (String, String) {
    let slot = text(node, "executionSlotId");
    for bound in bindings {
        if !slot.is_empty() && text(bound, "executionSlotId") == slot {
            return (
                text(bound, "agentId").to_string(),
                text(bound, "modelId").to_string(),
            );
        }
    }
    (
        text(node, "provider").to_string(),
        text(node, "model").to_string(),
    )
}

fn is_writer_key(key: &str) -> bool {
    let mut rest = key;
    if let Some((count, tail)) = key
        .strip_prefix("repair-")
        .and_then(|after| after.split_once('-'))
    {
        if !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit()) {
            rest = tail;
        }
    }
    WRITER_TOKENS.contains(&rest.rsplit('-').next().unwrap_or(""))
}

/// An id without the round number a second critique round of the same stage appends:
/// `gate-critique-spec-fan-out-2` is round 2 of `gate-critique-spec-fan-out`.
fn without_round(id: &str) -> (&str, &str) {
    match id.rsplit_once('-') {
        Some((head, round)) if !round.is_empty() && round.bytes().all(|b| b.is_ascii_digit()) => {
            (head, &id[head.len()..])
        }
        _ => (id, ""),
    }
}

fn is_record(id: &str) -> bool {
    let runner = {
        let base = without_round(id).0;
        base.starts_with("gate-critique-") && base.ends_with("-record")
    };
    let verdict = id
        .strip_prefix("verdict-")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    runner || verdict
}

struct Writer<'a> {
    id: &'a str,
    artifact: &'a str,
    family: String,
    binding: String,
}

fn writers<'a>(nodes: &'a [Value], bindings: &[Value], routing: &Routing) -> Vec<Writer<'a>> {
    let mut out = Vec::new();
    for node in nodes {
        let artifact = node.get("output").map_or("", |o| text(o, "artifact"));
        if text(node, "type") != "worker" || !is_writer_key(artifact) {
            continue;
        }
        let (agent, model) = binding(node, bindings);
        if let Some(family) = routing.family(&agent, &model) {
            out.push(Writer {
                id: text(node, "id"),
                artifact,
                family,
                binding: format!("{agent}/{model}"),
            });
        }
    }
    out
}

/// The shortest path from `start` to `$end` that crosses no record node and
/// no refusal.
fn unreviewed_path<'a>(start: &'a str, edges: &'a [Value]) -> Option<Vec<&'a str>> {
    let mut parent: BTreeMap<&str, &str> = BTreeMap::new();
    let mut seen: BTreeSet<&str> = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(current) = queue.pop_front() {
        for edge in edges.iter().filter(|e| text(e, "from") == current) {
            let to = text(edge, "to");
            if is_record(to) || to == REFUSAL_NODE || !seen.insert(to) {
                continue;
            }
            parent.insert(to, current);
            if to == END_NODE {
                let mut path = vec![to];
                let mut at = to;
                while let Some(&before) = parent.get(at) {
                    path.push(before);
                    at = before;
                }
                path.reverse();
                return Some(path);
            }
            queue.push_back(to);
        }
    }
    None
}

/// The writers nearest upstream of `critic`: reached walking edges backward,
/// not walking on through a writer.
fn nearest_writers<'a>(
    critic: &str,
    edges: &'a [Value],
    writer_ids: &BTreeSet<&str>,
) -> BTreeSet<&'a str> {
    let mut found = BTreeSet::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut queue = VecDeque::from([critic]);
    while let Some(current) = queue.pop_front() {
        for edge in edges.iter().filter(|e| text(e, "to") == current) {
            let from = text(edge, "from");
            if from == critic || !seen.insert(from) {
                continue;
            }
            if writer_ids.contains(from) {
                found.insert(from);
            } else {
                queue.push_back(from);
            }
        }
    }
    found
}

/// The family of a stage's orchestrator, the worker `<stage>-verdicts`: the one family the
/// runner leaves out of that stage's roster.
fn orchestrator_family(
    stage: &str,
    round: &str,
    nodes: &[Value],
    bindings: &[Value],
    routing: &Routing,
) -> Option<String> {
    let id = format!("{stage}-verdicts{round}");
    let node = nodes.iter().find(|n| text(n, "id") == id)?;
    let (agent, model) = binding(node, bindings);
    routing.family(&agent, &model)
}

/// A critic's candidates as (label, family).
fn candidates(
    node: &Value,
    nodes: &[Value],
    bindings: &[Value],
    routing: &Routing,
    instance: Instance,
) -> Vec<(String, String)> {
    let id = text(node, "id");
    let labelled = |agent: &str, model: &str| {
        routing
            .family(agent, model)
            .map(|family| (format!("{agent}/{model}"), family))
    };
    if text(node, "type") == "ai-dynamic" {
        let agents = node["agentStrategy"]["availableAgents"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        return agents
            .iter()
            .filter_map(|a| labelled(text(a, "provider"), text(a, "model")))
            .collect();
    }
    let (base, round) = without_round(id);
    if let Some(stage) = base
        .strip_prefix("gate-critique-")
        .and_then(|rest| rest.strip_suffix("-fan-out"))
    {
        let orchestrator = orchestrator_family(stage, round, nodes, bindings, routing);
        return routing
            .roster(stage, instance)
            .into_iter()
            .filter(|(_, family)| Some(family) != orchestrator.as_ref())
            .map(|(client, family)| (format!("routing client {client}"), family))
            .collect();
    }
    let profile = text(node, "profile");
    if !profile.is_empty() && profile != GENERIC_PROFILE {
        let (agent, model) = binding(node, bindings);
        return labelled(&agent, &model).into_iter().collect();
    }
    Vec::new()
}

/// Every critique finding of one workflow, as `<node>: <rule>: <message>`.
pub fn critique(
    nodes: &[Value],
    edges: &[Value],
    bindings: &[Value],
    instance: Instance,
    machine_override: Option<&Value>,
) -> Vec<String> {
    let routing = Routing::load(machine_override);
    let writers = writers(nodes, bindings, &routing);
    let mut out = Vec::new();
    for writer in &writers {
        if let Some(path) = unreviewed_path(writer.id, edges) {
            out.push(format!(
                "{}: critic-required: writes {} but the path {} reaches $end without crossing a critique \
                 record node (gate-critique-<stage>-record or verdict-<N>)",
                writer.id,
                writer.artifact,
                path.join(" -> ")
            ));
        }
    }
    let writer_ids: BTreeSet<&str> = writers.iter().map(|w| w.id).collect();
    for node in nodes {
        let id = text(node, "id");
        let found = candidates(node, nodes, bindings, &routing, instance);
        let upstream = nearest_writers(id, edges, &writer_ids);
        for writer in writers.iter().filter(|w| upstream.contains(w.id)) {
            for (label, family) in found.iter().filter(|(_, f)| *f == writer.family) {
                out.push(format!(
                    "{id}: critic-family: candidate {label} is of family {family}, the family of writer \
                     {} ({}), whose {} it reviews",
                    writer.id, writer.binding, writer.artifact
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
