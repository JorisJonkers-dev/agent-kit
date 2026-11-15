//! The `shape` rule group (#780): the checks Gold Band's workflow editor runs
//! before it saves a workflow, ported so a shipped template cannot open in the
//! app with errors the headless runtime never reports.
//!
//! Ported from Gold Band v0.19.1, commit `987fc3e298b15d725a7f0e9b609b0f77dba63a74`:
//!
//! * `web/src/components/WorkflowEditor.tsx`, `validateWorkflowForSave`
//!   (lines 3395-3680), `successConditionPath`, `parseExpressionPath`,
//!   `parseJsonPath`, `looksLikeJsonSchema`, `schemaContainsPath`,
//!   `nodeSupportsFailureOutcome` (:264) and `deriveWorkflowEntryCandidateIds`
//!   (:2904);
//! * `web/src/components/workflowGraph.ts`: `isBackwardEdge` (:39) and
//!   `workflowSuccessTopologyOrder` (:404).
//!
//! Not ported, because each needs the running app's live bindings or stores:
//! the provider, model, permission-mode and config-option availability checks
//! (the app's agent doctor), the role-visibility check against the profile
//! catalog, the duplicate workflow id check against the template store, and
//! the `ai-dynamic` node rules (agent strategy, allowed roles and workflows,
//! limits). Node ids are compared after de-duplication where the app keeps
//! duplicates (a duplicate id is reported as `duplicate-node-id` anyway).
//!
//! The entry rule is the *patched* one: setup applies
//! `scripts/gold-band/patches/0001-entry-rule-failure-targets.patch` to the
//! build copy (docs/GOLD-BAND.md, "The build patch"). A backward non-success
//! edge is ignored only when its target reaches its source over success edges;
//! any other failure edge makes its target a non-entry.
//!
//! Each finding reads `<node id or "workflow">: <rule>: <message>`; the
//! messages are the editor's own English text.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::Value;

const END_NODE: &str = "$end";
const ENTRY_NODE: &str = "$entry";
const NEW_ROUND_NODE: &str = "$new-round";
const WORKFLOW: &str = "workflow";

/// One finding: subject, rule, message.
type Finding = (String, &'static str, String);

/// The string of a field, `""` when it is missing or not a string.
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// JavaScript truthiness of a JSON value (`Boolean(x)`).
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(_) => true,
    }
}

/// `nodeSupportsFailureOutcome`: a worker with a manual check or validation.
fn supports_failure(node: Option<&Value>) -> bool {
    node.is_some_and(|n| {
        text(n, "type") == "worker"
            && (truthy(n.get("manual_check"))
                || truthy(n.get("output"))
                || truthy(n.get("success_condition")))
    })
}

/// `workflowSuccessTopologyOrder`: Kahn's order over success edges, nodes
/// that sit on a success cycle appended last.
fn topology_order(ids: &[String], edges: &[Value]) -> BTreeMap<String, usize> {
    let set: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    let mut adjacency: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
    for id in ids {
        adjacency.insert(id, Vec::new());
        indegree.insert(id, 0);
    }
    for edge in edges {
        let (from, to) = (text(edge, "from"), text(edge, "to"));
        if text(edge, "on") == "success" && set.contains(from) && set.contains(to) {
            adjacency.entry(from).or_default().push(to);
            *indegree.entry(to).or_default() += 1;
        }
    }
    let mut queued: BTreeSet<&str> = BTreeSet::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    for id in ids {
        if indegree[id.as_str()] == 0 && queued.insert(id) {
            queue.push_back(id);
        }
    }
    let mut ordered: Vec<&str> = Vec::new();
    while let Some(id) = queue.pop_front() {
        ordered.push(id);
        for &next in &adjacency[id] {
            let left = indegree.entry(next).or_default();
            *left = left.saturating_sub(1);
            if *left == 0 && queued.insert(next) {
                queue.push_back(next);
            }
        }
    }
    for id in ids {
        if !queued.contains(id.as_str()) {
            ordered.push(id);
        }
    }
    ordered
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id.to_string(), index))
        .collect()
}

/// Whether `goal` is reachable from `start` over success edges.
fn reaches_over_success(edges: &[Value], start: &str, goal: &str) -> bool {
    let mut seen: BTreeSet<&str> = BTreeSet::from([start]);
    let mut stack = vec![start];
    while let Some(current) = stack.pop() {
        if current == goal {
            return true;
        }
        for edge in edges {
            let next = text(edge, "to");
            if text(edge, "on") == "success" && text(edge, "from") == current && seen.insert(next) {
                stack.push(next);
            }
        }
    }
    false
}

/// `deriveWorkflowEntryCandidateIds`: nodes with no incoming edge, ignoring a
/// non-success edge that goes backward in the success topology and closes a loop.
fn entry_candidates(nodes: &[Value], edges: &[Value]) -> Vec<String> {
    let ids: Vec<String> = nodes
        .iter()
        .map(|n| text(n, "id").to_string())
        .filter(|id| !id.is_empty())
        .collect();
    let mut unique: Vec<String> = Vec::new();
    for id in &ids {
        if !unique.contains(id) {
            unique.push(id.clone());
        }
    }
    let order = topology_order(&unique, edges);
    let mut incoming: BTreeSet<&str> = BTreeSet::new();
    for edge in edges {
        let (from, to) = (text(edge, "from"), text(edge, "to"));
        if !order.contains_key(from) || !order.contains_key(to) {
            continue;
        }
        // `isBackwardEdge`: the target sorts before the source. Patched
        // (scripts/gold-band/patches/0001): such an edge is a retry loop only
        // when its target reaches its source over success edges.
        if text(edge, "on") != "success"
            && order[to] < order[from]
            && reaches_over_success(edges, to, from)
        {
            continue;
        }
        incoming.insert(to);
    }
    ids.into_iter()
        .filter(|id| !incoming.contains(id.as_str()))
        .collect()
}

/// One segment of a success-condition path.
#[derive(Debug, PartialEq)]
enum Seg {
    Key(String),
    Index(usize),
}

/// `parseJsonPath`; `Err` carries the editor's reason.
fn parse_json_path(path: &str) -> Result<Vec<Seg>, &'static str> {
    let mut value = path.trim();
    if let Some(rest) = value.strip_prefix("$.") {
        value = rest;
    } else if value == "$" {
        return Err("root path is not supported");
    } else if let Some(rest) = value.strip_prefix('$') {
        value = rest;
    }
    if value.is_empty() {
        return Err("empty path");
    }
    let chars: Vec<char> = value.chars().collect();
    let mut segments: Vec<Seg> = Vec::new();
    let mut key = String::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '.' => {
                if key.is_empty() {
                    if !matches!(segments.last(), Some(Seg::Index(_))) {
                        return Err("empty segment");
                    }
                } else {
                    segments.push(Seg::Key(std::mem::take(&mut key)));
                }
                index += 1;
            }
            '[' => {
                if !key.is_empty() {
                    segments.push(Seg::Key(std::mem::take(&mut key)));
                }
                let close = chars[index + 1..]
                    .iter()
                    .position(|c| *c == ']')
                    .map(|p| p + index + 1)
                    .ok_or("unclosed index")?;
                let raw: String = chars[index + 1..close].iter().collect();
                if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
                    return Err("invalid index");
                }
                segments.push(Seg::Index(raw.parse().unwrap_or(usize::MAX)));
                index = close + 1;
                if index < chars.len() && chars[index] != '.' && chars[index] != '[' {
                    return Err("invalid separator");
                }
            }
            c => {
                key.push(c);
                index += 1;
            }
        }
    }
    if !key.is_empty() {
        segments.push(Seg::Key(key));
    }
    // The editor re-checks for no segments here; a non-empty path cannot get
    // this far without one (it fails on a leading `.` or `[` first).
    Ok(segments)
}

/// `successConditionPath`: the path a success condition reads.
fn success_path(condition: &Value) -> Result<Vec<Seg>, &'static str> {
    let Some(object) = condition.as_object() else {
        return Err("condition is not an object");
    };
    if let Some(expression) = object.get("expression") {
        let expression = expression.as_str().unwrap_or("");
        let operator = [">=", "<=", "!=", "==", ">", "<"]
            .into_iter()
            .find(|op| expression.contains(op))
            .ok_or("unsupported expression")?;
        let left = expression.split(operator).next().unwrap_or("").trim();
        if !left.starts_with('$') {
            return Err("left side must start with $");
        }
        return parse_json_path(left);
    }
    match object.get("path") {
        None | Some(Value::Null) => parse_json_path(""),
        Some(Value::String(path)) => parse_json_path(path),
        Some(_) => Err("path is not a string"),
    }
}

/// `looksLikeJsonSchema`: the legacy JSON Schema form of an output shape.
fn looks_like_json_schema(schema: &Value) -> bool {
    schema.as_object().is_some_and(|o| {
        [
            "type",
            "properties",
            "required",
            "additionalProperties",
            "items",
        ]
        .iter()
        .any(|k| o.contains_key(*k))
    })
}

/// `schemaContainsPath`: the path walks the simplified output shape.
fn schema_contains_path(schema: &Value, path: &[Seg]) -> bool {
    let mut cursor = schema;
    for segment in path {
        match segment {
            Seg::Key(key) => match cursor.as_object().and_then(|o| o.get(key)) {
                Some(next) => cursor = next,
                None => return false,
            },
            Seg::Index(index) => {
                let Some(items) = cursor.as_array() else {
                    return false;
                };
                // `cursor[index] ?? cursor[0]`
                match items.get(*index).filter(|v| !v.is_null()).or(items.first()) {
                    Some(next) => cursor = next,
                    None => return false,
                }
            }
        }
    }
    true
}

/// `normalizeControlLimit` (`Math.trunc(Number(x))`, 0 when not finite).
fn control_limit(value: &Value) -> i64 {
    let number = match value {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) if s.trim().is_empty() => 0.0,
        Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => 0.0,
    };
    if number.is_finite() {
        number.trunc() as i64
    } else {
        0
    }
}

struct Findings(Vec<Finding>);

impl Findings {
    fn add(&mut self, subject: &str, rule: &'static str, message: String) {
        let subject = if subject.is_empty() {
            WORKFLOW
        } else {
            subject
        };
        self.0.push((subject.to_string(), rule, message));
    }
}

fn label(node: &Value) -> String {
    let id = text(node, "id");
    if id.is_empty() { "Unnamed node" } else { id }.to_string()
}

/// The workflow-level checks: id, nodes, entry, `$end`, control limits.
fn workflow_findings(def: &Value, nodes: &[Value], edges: &[Value], out: &mut Findings) {
    if text(def, "id").trim().is_empty() {
        out.add(
            WORKFLOW,
            "workflow-id-required",
            "Workflow ID is required.".into(),
        );
    }
    if nodes.is_empty() {
        out.add(
            WORKFLOW,
            "nodes-required",
            "Workflow requires at least one node.".into(),
        );
    } else {
        let entries = entry_candidates(nodes, edges);
        if entries.is_empty() {
            out.add(
                WORKFLOW,
                "entry-missing",
                "Workflow must have exactly one entry node with no incoming edge.".into(),
            );
        } else if entries.len() > 1 {
            // One finding per surplus entry, so each is named; the message
            // is the editor's, listing every candidate.
            for extra in &entries[1..] {
                out.add(
                    extra,
                    "entry-multiple",
                    format!(
                        "Workflow has multiple entry nodes: {}. Connect the graph so there is exactly one entry.",
                        entries.join(", ")
                    ),
                );
            }
        }
    }
    if !edges.iter().any(|e| text(e, "to") == END_NODE) {
        out.add(
            WORKFLOW,
            "end-required",
            "Workflow must include an end node.".into(),
        );
    }
    let control = def.get("control");
    for (key, rule, name) in [
        ("max_attempts", "max-attempts-positive", "Max attempts"),
        ("max_rounds", "max-rounds-positive", "Max rounds"),
    ] {
        let limit = control.and_then(|c| c.get(key)).filter(|v| !v.is_null());
        if limit.is_some_and(|v| control_limit(v) <= 0) {
            out.add(WORKFLOW, rule, format!("{name} must be greater than 0."));
        }
    }
}

/// The worker-node checks: slot, role, and the result-mode rules.
fn worker_findings(node: &Value, slot_counts: &BTreeMap<String, usize>, out: &mut Findings) {
    let (id, name) = (text(node, "id"), label(node));
    let slot = text(node, "executionSlotId").trim();
    if slot.is_empty() {
        out.add(
            id,
            "slot-required",
            format!("{name} node is missing an execution slot."),
        );
    } else if slot_counts.get(slot).copied().unwrap_or(0) > 1 {
        out.add(
            id,
            "slot-duplicate",
            format!("{name} node has a duplicate execution slot."),
        );
    }
    if text(node, "profile").trim().is_empty() {
        out.add(
            id,
            "profile-required",
            format!("{name} node is not associated with a role."),
        );
    }
    let validation = truthy(node.get("output")) || truthy(node.get("success_condition"));
    if validation && truthy(node.get("manual_check")) {
        out.add(
            id,
            "manual-check-exclusive",
            format!("{name} cannot enable AI output validation and manual check at the same time."),
        );
    }
    if !validation {
        return;
    }
    let output = node.get("output");
    if output
        .and_then(|o| o.get("artifact"))
        .and_then(Value::as_str)
        .is_none_or(|a| a.trim().is_empty())
    {
        out.add(
            id,
            "output-artifact-required",
            format!("{name} output artifact Key is required."),
        );
    }
    let condition = node.get("success_condition").filter(|c| truthy(Some(c)));
    let mut path = None;
    match condition.map(success_path) {
        None => out.add(
            id,
            "success-expression-required",
            format!("{name} requires a success expression when output validation is enabled."),
        ),
        Some(Ok(found)) => path = Some(found),
        Some(Err(_)) => out.add(
            id,
            "success-expression-invalid",
            format!("{name} has an invalid success expression."),
        ),
    }
    let schema = output
        .and_then(|o| o.get("schema"))
        .filter(|s| truthy(Some(s)));
    if let Some(schema) = schema {
        if looks_like_json_schema(schema) {
            out.add(
                id,
                "legacy-schema",
                format!("{name} still uses legacy JSON Schema. Use the simplified output shape instead."),
            );
        } else if path
            .as_deref()
            .is_some_and(|p| !schema_contains_path(schema, p))
        {
            out.add(
                id,
                "schema-missing-path",
                format!(
                    "{name} success expression path is not declared in the JSON output constraint."
                ),
            );
        }
    }
}

/// The per-node checks: id, outgoing edge, then a worker's own rules.
fn node_findings(nodes: &[Value], edges: &[Value], out: &mut Findings) {
    let mut id_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for node in nodes {
        *id_counts.entry(text(node, "id")).or_default() += 1;
    }
    let mut slot_counts: BTreeMap<String, usize> = BTreeMap::new();
    for node in nodes {
        let slot = text(node, "executionSlotId");
        if text(node, "type") == "worker" && !slot.trim().is_empty() {
            *slot_counts.entry(slot.to_string()).or_default() += 1;
        }
    }
    for node in nodes {
        let (id, name) = (text(node, "id"), label(node));
        if id.trim().is_empty() {
            out.add(
                id,
                "node-id-required",
                format!("{name} node ID is required."),
            );
        }
        if [END_NODE, ENTRY_NODE, NEW_ROUND_NODE].contains(&id) {
            out.add(
                id,
                "reserved-node-id",
                format!("{name} uses a reserved node ID."),
            );
        }
        if id_counts[id] > 1 {
            out.add(
                id,
                "duplicate-node-id",
                format!("{name} node ID is duplicated."),
            );
        }
        if !edges
            .iter()
            .any(|e| text(e, "from") == id && !id.trim().is_empty())
        {
            out.add(
                id,
                "dangling-node",
                format!("{name} has no outgoing edge and cannot continue or end the workflow."),
            );
        }
        if text(node, "type") != "ai-dynamic" {
            worker_findings(node, &slot_counts, out);
        }
    }
}

/// The edge-outcome checks of one edge (the editor's `if / else if` chain).
fn outcome_findings(
    index: usize,
    edge: &Value,
    by_id: &BTreeMap<&str, &Value>,
    counts: &BTreeMap<(String, String), usize>,
    reported: &mut BTreeSet<(String, String)>,
    out: &mut Findings,
) {
    let (from, to, on) = (text(edge, "from"), text(edge, "to"), text(edge, "on"));
    if on != "success" && on != "failure" {
        out.add(
            from,
            "edge-outcome-invalid",
            format!("Edge {} has an invalid type.", index + 1),
        );
    } else if on == "failure" && !supports_failure(by_id.get(from).copied()) {
        out.add(
            from,
            "failure-edge-unsupported",
            format!("{from} can only have a failure branch when AI output validation or manual check is enabled."),
        );
    } else if on == "success" && to == NEW_ROUND_NODE {
        out.add(
            from,
            "success-new-round",
            format!("{from} success edge cannot target new round."),
        );
    } else if !from.trim().is_empty() {
        let key = (from.to_string(), on.to_string());
        let count = counts[&key];
        if count > 1 && reported.insert(key) {
            out.add(
                from,
                "duplicate-edge-outcome",
                format!("{from} has {count} {on} edges; each outcome type can only have one edge."),
            );
        }
    }
}

/// The endpoint checks of one edge: source, target, new-round start, terminals.
fn endpoint_findings(index: usize, edge: &Value, ids: &BTreeSet<&str>, out: &mut Findings) {
    let (from, to) = (text(edge, "from"), text(edge, "to"));
    let n = index + 1;
    if from.trim().is_empty() {
        out.add(
            from,
            "edge-source-required",
            format!("Edge {n} is missing a source node."),
        );
    } else if !ids.contains(from) {
        out.add(
            from,
            "edge-source-missing",
            format!("Edge source node {from} does not exist."),
        );
    }
    if to.trim().is_empty() {
        out.add(
            from,
            "edge-target-required",
            format!("Edge {n} is missing a target node."),
        );
    } else if to != END_NODE && to != NEW_ROUND_NODE && !ids.contains(to) {
        out.add(
            from,
            "edge-target-missing",
            format!("Edge target node {to} does not exist."),
        );
    }
    if to == NEW_ROUND_NODE {
        let entry = text(edge, "new_round_entry").trim();
        if entry.is_empty() {
            out.add(
                from,
                "new-round-entry-required",
                format!("{from} edge targeting new round must choose the next round start."),
            );
        } else if entry != ENTRY_NODE && !ids.contains(entry) {
            out.add(
                from,
                "new-round-entry-missing",
                format!("{from} edge targeting new round selected missing start node {entry}."),
            );
        }
    }
    if from == END_NODE || from == NEW_ROUND_NODE {
        out.add(
            from,
            "terminal-edge-source",
            format!("End node {from} cannot be an edge source."),
        );
    }
    if text(edge, "session") == "continue" && (to == END_NODE || to == NEW_ROUND_NODE) {
        out.add(
            from,
            "continue-terminal-target",
            format!("Edge {n} continue session cannot target an end node."),
        );
    }
}

/// Every shape finding of one workflow definition, as
/// `<subject>: <rule>: <message>` lines in a stable order (workflow-level,
/// then nodes in document order, then edges in document order).
pub fn shape(def: &Value, nodes: &[Value], edges: &[Value]) -> Vec<String> {
    let mut out = Findings(Vec::new());
    workflow_findings(def, nodes, edges, &mut out);
    node_findings(nodes, edges, &mut out);
    let ids: BTreeSet<&str> = nodes
        .iter()
        .map(|n| text(n, "id"))
        .filter(|id| !id.is_empty())
        .collect();
    let by_id: BTreeMap<&str, &Value> = nodes.iter().map(|n| (text(n, "id"), n)).collect();
    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for edge in edges {
        let (from, on) = (text(edge, "from"), text(edge, "on"));
        if !from.trim().is_empty() && (on == "success" || on == "failure") {
            *counts
                .entry((from.to_string(), on.to_string()))
                .or_default() += 1;
        }
    }
    let mut reported = BTreeSet::new();
    for (index, edge) in edges.iter().enumerate() {
        endpoint_findings(index, edge, &ids, &mut out);
        outcome_findings(index, edge, &by_id, &counts, &mut reported, &mut out);
    }
    out.0
        .into_iter()
        .map(|(subject, rule, message)| format!("{subject}: {rule}: {message}"))
        .collect()
}

#[cfg(test)]
mod tests;
