//! Lints a Gold Band workflow definition against the model and loop rules
//! every port shares (#371).
//!
//! Input is Gold Band's workflow JSON at version `"0.1"` (#350): either a
//! bare definition, or the `--workflow` authoring wrapper with the
//! definition nested under `workflow` and its worker bindings under
//! `modelBindings` (joined to a node by `executionSlotId`). The lint does
//! not re-validate the definition's shape — Gold Band's own validator does
//! that; it refuses only a definition that breaks the rules every port
//! shares, each reported as one problem line
//! `workflow-lint: <FILE>: <node id or "workflow">: <rule>`:
//!
//! * `banned-binding`: the node binds an agent id or model (any binding
//!   field — a worker's `provider`/`model`, an `ai-dynamic` strategy's
//!   fixed, dynamic, bootstrap, acceptance or available-agent bindings, or
//!   the wrapper's `agentId`/`modelId`) containing `codex`
//!   (case-insensitive). An OpenRouter or `opencode/` (Zen) binding is
//!   allowed (owner decision, 2026-11-08): the per-key LiteLLM budgets are
//!   the guard, and the model gateway keeps its own refusals;
//! * `opus-outside-judgment`: the node binds an agent id or model
//!   containing `opus` (case-insensitive) without being a judgment node. A
//!   workflow marks its judgment nodes by naming convention: the
//!   escalation node it hands off to once the repair cap
//!   (`control.max_attempts`) is reached ends with `-escalation`, and the
//!   final acceptance node ends with `-acceptance` with every edge leaving
//!   it targeting `$end`, and the planning node that states Workflow A's
//!   targeted prompt ends with `-planning` (owner decision, 2026-11-08) —
//!   those three, and only those, may bind Opus;
//! * `unbounded-back-edge`: the graph of node-to-node edges has a cycle (a
//!   back-edge) while `control.max_attempts` is absent;
//! * `unbounded-rounds`: an edge targets `$new-round` while
//!   `control.max_rounds` is absent.
//!
//! `workflow` is the subject a rule names when no single node owns it.
//!
//! Usage: `workflow-lint <FILE>...` lints every file and prefixes its
//! problems with the file path. Exit 0 clean, printing `workflow-lint: ok`;
//! 1 problems; 2 bad usage or an unreadable or invalid file.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use serde_json::Value;

/// Rule: a node binds an agent or model the ports never allow.
const BANNED_BINDING: &str = "banned-binding";
/// Rule: a node binds Opus without being a judgment node.
const OPUS_OUTSIDE_JUDGMENT: &str = "opus-outside-judgment";
/// Rule: a back-edge that `control.max_attempts` does not bound.
const UNBOUNDED_BACK_EDGE: &str = "unbounded-back-edge";
/// Rule: a `$new-round` edge that `control.max_rounds` does not bound.
const UNBOUNDED_ROUNDS: &str = "unbounded-rounds";
/// The subject for a problem no single node owns.
const WORKFLOW: &str = "workflow";
/// The reserved edge target that ends the workflow.
const END: &str = "$end";
/// The reserved edge target that opens a new round.
const NEW_ROUND: &str = "$new-round";
/// The agent id or model name no binding may contain (case-insensitive).
/// An OpenRouter or `opencode/` binding is allowed (owner decision,
/// 2026-11-08); the per-key LiteLLM budgets are the guard.
const BANNED: &str = "codex";
/// The agent id or model marker of an Opus binding (case-insensitive).
const OPUS: &str = "opus";

/// The string of one field of a JSON object, if it is a string.
fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// The array of one field of a JSON object, if it is an array.
fn array_field<'a>(value: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    value.get(key).and_then(Value::as_array)
}

/// Whether an agent id or model names a banned provider, case-insensitive.
fn banned_text(text: &str) -> bool {
    text.to_ascii_lowercase().contains(BANNED)
}

/// Whether an agent id or model marks an Opus binding, case-insensitive.
fn is_opus(text: &str) -> bool {
    text.to_ascii_lowercase().contains(OPUS)
}

/// The rule-1 verdict on a node's bindings once collected.
fn banned_binding(agents: &[&str], models: &[&str]) -> bool {
    agents.iter().copied().any(banned_text) || models.iter().any(|m| banned_text(m))
}

/// Whether any binding of the node is an Opus one.
fn opus_bound(agents: &[&str], models: &[&str]) -> bool {
    let agent = agents.iter().copied().any(is_opus);
    let model = models.iter().copied().any(is_opus);
    agent || model
}

/// Whether a node id marks a judgment node — the escalation node the repair
/// cap hands off to, or the final acceptance node — which may bind Opus: a
/// `-escalation` id always marks the escalation node, and a `-acceptance`
/// id marks the final acceptance node only while `mid` does not carry it.
fn is_judgment(id: &str, mid: &BTreeSet<&str>) -> bool {
    if id.ends_with("-escalation") || id.ends_with("-planning") {
        return true;
    }
    id.ends_with("-acceptance") && !mid.contains(id)
}

/// The `-acceptance` nodes that are not the final acceptance node: an edge
/// leaves them for something other than `$end`, so Opus may not be bound
/// to them.
fn mid_acceptance<'a>(nodes: &'a [Value], edges: &'a [Value]) -> BTreeSet<&'a str> {
    let mut out = BTreeSet::new();
    for node in nodes {
        let Some(id) = str_field(node, "id") else {
            continue;
        };
        if id.ends_with("-acceptance")
            && edges.iter().any(|edge| {
                str_field(edge, "from") == Some(id) && str_field(edge, "to") != Some(END)
            })
        {
            out.insert(id);
        }
    }
    out
}

/// Appends each field of `value` named in `keys` that is a string.
fn push_fields<'a>(out: &mut Vec<&'a str>, value: &'a Value, keys: &[&str]) {
    for key in keys {
        if let Some(found) = str_field(value, key) {
            out.push(found);
        }
    }
}

/// Collects every agent id and model one node binds, from any binding
/// field: a worker's own `provider`/`model` (an `ai-dynamic` node's legacy
/// plain ones included), its `agentStrategy`'s fixed or dynamic bindings,
/// and the authoring-wrapper bindings joined to it by `executionSlotId`.
fn node_strings<'a>(
    node: &'a Value,
    bindings: &'a [Value],
    agents: &mut Vec<&'a str>,
    models: &mut Vec<&'a str>,
) {
    push_fields(agents, node, &["provider"]);
    push_fields(models, node, &["model"]);
    if let Some(strategy) = node.get("agentStrategy") {
        match str_field(strategy, "mode") {
            Some("fixed") => {
                push_fields(agents, strategy, &["provider"]);
                push_fields(models, strategy, &["model"]);
            }
            Some("dynamic") => {
                push_fields(agents, strategy, &["bootstrapProvider"]);
                push_fields(models, strategy, &["bootstrapModel"]);
                push_fields(models, strategy, &["acceptanceModel"]);
                if let Some(available) = array_field(strategy, "availableAgents") {
                    for agent in available {
                        push_fields(agents, agent, &["provider"]);
                        push_fields(models, agent, &["model"]);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(slot) = str_field(node, "executionSlotId") {
        for binding in bindings {
            if str_field(binding, "executionSlotId") == Some(slot) {
                push_fields(agents, binding, &["agentId"]);
                push_fields(models, binding, &["modelId"]);
            }
        }
    }
}

/// The rules one node breaks; a node without an id cannot be named, so it
/// breaks none here (Gold Band's own validator rejects it). `mid` carries
/// the `-acceptance` nodes that are not final, which may not bind Opus.
fn node_problems<'a>(
    node: &'a Value,
    bindings: &'a [Value],
    mid: &BTreeSet<&str>,
) -> Vec<(&'a str, &'static str)> {
    let Some(id) = str_field(node, "id") else {
        return Vec::new();
    };
    let mut agents = Vec::new();
    let mut models = Vec::new();
    node_strings(node, bindings, &mut agents, &mut models);
    let mut problems = Vec::new();
    if banned_binding(&agents, &models) {
        problems.push((id, BANNED_BINDING));
    }
    if opus_bound(&agents, &models) && !is_judgment(id, mid) {
        problems.push((id, OPUS_OUTSIDE_JUDGMENT));
    }
    problems
}

/// The definition to lint: the root, or its `workflow` member when the file
/// is the authoring wrapper.
fn definition(root: &Value) -> Result<&Value, String> {
    if !root.is_object() {
        return Err("the file is not a JSON object".to_string());
    }
    match root.get("workflow") {
        Some(def) if def.is_object() => Ok(def),
        Some(_) => Err("workflow is not a JSON object".to_string()),
        None => Ok(root),
    }
}

/// The required `nodes`/`edges` array.
fn field_array<'a>(def: &'a Value, key: &str) -> Result<&'a [Value], String> {
    array_field(def, key)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{key} is not an array"))
}

/// The authoring wrapper's worker bindings, empty outside the wrapper.
fn bindings(root: &Value) -> &[Value] {
    root.get("modelBindings")
        .and_then(|m| m.get("bindings"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Whether `control` declares the limit as a positive whole number.
fn has_limit(def: &Value, key: &str) -> bool {
    def.get("control")
        .and_then(|c| c.get(key))
        .and_then(Value::as_u64)
        .is_some_and(|n| n > 0)
}

/// The directed graph of node-to-node edges: terminal targets (`$end`,
/// `$new-round`) and edges that do not name two nodes stay out of it.
fn adjacency<'a>(ids: &BTreeSet<&'a str>, edges: &'a [Value]) -> BTreeMap<&'a str, Vec<&'a str>> {
    let mut out: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for id in ids {
        out.insert(*id, Vec::new());
    }
    for edge in edges {
        if let (Some(from), Some(to)) = (str_field(edge, "from"), str_field(edge, "to")) {
            if ids.contains(from) && ids.contains(to) {
                out.entry(from).or_default().push(to);
            }
        }
    }
    out
}

/// The DFS states of the cycle search.
enum Color {
    OnStack,
    Done,
}

/// The node the search is inside when it closes a cycle, if it finds one.
fn goes_back<'a>(
    node: &'a str,
    adj: &BTreeMap<&'a str, Vec<&'a str>>,
    color: &mut BTreeMap<&'a str, Color>,
) -> Option<&'a str> {
    color.insert(node, Color::OnStack);
    for &next in adj.get(node).map_or(&[][..], Vec::as_slice) {
        match color.get(next) {
            None => {
                if let Some(found) = goes_back(next, adj, color) {
                    return Some(found);
                }
            }
            Some(Color::OnStack) => return Some(next),
            Some(Color::Done) => {}
        }
    }
    color.insert(node, Color::Done);
    None
}

/// One node of the graph's first cycle, found working through `ids` in
/// order; `None` when the graph of node-to-node edges has no cycle.
fn cycle<'a>(ids: &[&'a str], adj: &BTreeMap<&'a str, Vec<&'a str>>) -> Option<&'a str> {
    let mut color: BTreeMap<&str, Color> = BTreeMap::new();
    for id in ids {
        if !color.contains_key(id) {
            if let Some(found) = goes_back(id, adj, &mut color) {
                return Some(found);
            }
        }
    }
    None
}

/// The loop rules one workflow breaks.
fn loop_problems<'a>(
    def: &'a Value,
    nodes: &'a [Value],
    edges: &'a [Value],
) -> Vec<(&'a str, &'static str)> {
    let ids: Vec<&str> = nodes.iter().filter_map(|n| str_field(n, "id")).collect();
    let mut problems = Vec::new();
    if !has_limit(def, "max_attempts") {
        let known: BTreeSet<&str> = ids.iter().copied().collect();
        if let Some(node) = cycle(&ids, &adjacency(&known, edges)) {
            problems.push((node, UNBOUNDED_BACK_EDGE));
        }
    }
    if !has_limit(def, "max_rounds") {
        for edge in edges {
            if str_field(edge, "to") == Some(NEW_ROUND) {
                let from = str_field(edge, "from").unwrap_or(WORKFLOW);
                problems.push((from, UNBOUNDED_ROUNDS));
                break;
            }
        }
    }
    problems
}

/// Every rule problem in the parsed workflow, in a stable order (nodes in
/// document order, each `banned-binding` before `opus-outside-judgment`,
/// the loop rules last); `Err` when the file is not workflow-shaped JSON.
pub fn lint(root: &Value) -> Result<Vec<String>, String> {
    let def = definition(root)?;
    let nodes = field_array(def, "nodes")?;
    let edges = field_array(def, "edges")?;
    let bound = bindings(root);
    let mid = mid_acceptance(nodes, edges);
    let mut problems: Vec<(&str, &'static str)> = Vec::new();
    for node in nodes {
        problems.extend(node_problems(node, bound, &mid));
    }
    problems.extend(loop_problems(def, nodes, edges));
    Ok(problems
        .into_iter()
        .map(|(subject, rule)| format!("{subject}: {rule}"))
        .collect())
}

/// Lints one file and prints its problems; returns its exit code (0 clean,
/// 1 problems, 2 unreadable or invalid).
fn lint_file(path: &Path, shown: &str) -> i32 {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("workflow-lint: {shown}: cannot read file: {e}");
            return 2;
        }
    };
    let root: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("workflow-lint: {shown}: invalid JSON: {e}");
            return 2;
        }
    };
    let problems = match lint(&root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("workflow-lint: {shown}: invalid workflow: {e}");
            return 2;
        }
    };
    for problem in &problems {
        eprintln!("workflow-lint: {shown}: {problem}");
    }
    if problems.is_empty() {
        0
    } else {
        1
    }
}

/// Runs one command line and returns its exit code.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    if args.is_empty() {
        eprintln!("usage: workflow-lint <FILE>...");
        return 2;
    }
    let mut code = 0;
    for arg in &args {
        let shown = arg.to_string_lossy();
        code = code.max(lint_file(Path::new(arg), &shown));
    }
    if code == 0 {
        println!("workflow-lint: ok");
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of the #350 repair workflows, realistic and passing: a
    /// repair loop bounded by `max_attempts`, ending in an escalation node
    /// and a final acceptance node, with Opus bound only to those two.
    const REPAIR_LOOP: &str = r#"{
      "version": "0.1",
      "id": "structured-review-loop",
      "entry": "build",
      "control": { "max_attempts": 2 },
      "nodes": [
        { "type": "worker", "id": "build", "provider": "claude-acp",
          "model": "claude-sonnet-4-5", "profile": "pf-builtin-dev",
          "goal": "Implement the requirement.", "output": null,
          "success_condition": null, "permission_mode": "bypassPermissions",
          "auto_accept": true, "config_options": { "effort": "max" } },
        { "type": "worker", "id": "repair", "provider": "claude-acp",
          "model": "claude-sonnet-4-5", "profile": "pf-builtin-dev",
          "goal": "Fix the review findings." },
        { "type": "worker", "id": "review-escalation", "provider": "claude-acp",
          "model": "opus-4-1", "profile": "pf-escalation",
          "goal": "Take over the review the repair cap did not settle." },
        { "type": "worker", "id": "final-acceptance", "provider": "claude-acp",
          "model": "opus-4-1", "profile": "pf-critique",
          "goal": "Accept the finished work." }
      ],
      "edges": [
        { "from": "build", "to": "final-acceptance", "on": "success", "session": null },
        { "from": "build", "to": "repair", "on": "failure", "session": null },
        { "from": "repair", "to": "build", "on": "success", "session": null },
        { "from": "repair", "to": "review-escalation", "on": "failure", "session": null },
        { "from": "review-escalation", "to": "$end", "on": "success", "session": null },
        { "from": "final-acceptance", "to": "$end", "on": "success", "session": null }
      ]
    }"#;

    /// The shape of the #350 multi-round workflows, realistic and passing:
    /// a `$new-round` back to the entry, bounded by `max_rounds`.
    const ROUNDS: &str = r#"{
      "version": "0.1",
      "id": "research-rounds",
      "entry": "plan",
      "control": { "max_rounds": 3 },
      "nodes": [
        { "type": "worker", "id": "plan", "provider": "claude-acp",
          "model": "claude-sonnet-4-5", "profile": "pf-builtin-dev",
          "goal": "Plan the round and open the next one." },
        { "type": "worker", "id": "gather", "provider": "claude-acp",
          "model": "claude-sonnet-4-5", "profile": "pf-builtin-dev",
          "goal": "Gather this round's findings." }
      ],
      "edges": [
        { "from": "plan", "to": "gather", "on": "success", "session": null },
        { "from": "gather", "to": "$end", "on": "success", "session": null },
        { "from": "gather", "to": "$new-round", "on": "failure", "session": null,
          "new_round_entry": "$entry" }
      ]
    }"#;

    /// The authoring-wrapper shape (#363): the definition nested under
    /// `workflow`, its worker binding under `modelBindings`, joined to the
    /// node by `executionSlotId`.
    const WRAPPED: &str = r#"{
      "workflow": {
        "version": "0.1",
        "id": "authoring-wrapped",
        "entry": "dev-build",
        "control": { "max_attempts": 1 },
        "nodes": [
          { "type": "worker", "id": "dev-build", "provider": "claude-acp",
            "executionSlotId": "slot-dev-build", "profile": "pf-builtin-dev",
            "goal": "Implement it.", "output": null, "success_condition": null,
            "permission_mode": "bypassPermissions", "auto_accept": true,
            "config_options": { "effort": "max" } }
        ],
        "edges": [
          { "from": "dev-build", "to": "$end", "on": "success", "session": null }
        ]
      },
      "modelBindings": {
        "bindings": [
          { "executionSlotId": "slot-dev-build", "agentId": "claude-acp",
            "modelId": "claude-sonnet-4-5", "permissionModeId": "bypassPermissions",
            "autoAccept": true }
        ]
      }
    }"#;

    const NOT_OBJECT: &str = "the file is not a JSON object";
    const BAD_WORKFLOW: &str = "workflow is not a JSON object";
    const NO_NODES: &str = "nodes is not an array";
    const NO_EDGES: &str = "edges is not an array";

    const EMPTY: &str = "{}";
    const CAP: &str = r#"{"max_attempts":2}"#;
    const CAP_NULL: &str = r#"{"max_attempts":null}"#;
    const SLOT: &str = r#"{"id":"dev","executionSlotId":"s"}"#;
    const EDGE: &str = r#"{"from":"dev","to":"$end"}"#;
    const TWO: &str = concat!(
        r#"{"id":"a","provider":"acp","model":"m"},"#,
        r#"{"id":"b","provider":"acp","model":"m"}"#
    );
    const FORTH: &str = concat!(
        r#"{"from":"a","to":"b","on":"success","session":null},"#,
        r#"{"from":"b","to":"a","on":"failure","session":null}"#
    );

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    fn object(fields: &[(&str, Value)]) -> Value {
        let mut map = serde_json::Map::new();
        for (key, value) in fields {
            map.insert(key.to_string(), value.clone());
        }
        Value::Object(map)
    }

    fn array(text: &str) -> Value {
        parse(&format!("[{text}]"))
    }

    fn flow(control: &str, nodes: &str, edges: &str) -> Value {
        object(&[
            ("control", parse(control)),
            ("nodes", array(nodes)),
            ("edges", array(edges)),
        ])
    }

    fn simple(nodes: &str) -> Value {
        flow(EMPTY, nodes, EDGE)
    }

    fn wrapped(node: &str, bindings: &str) -> Value {
        let def = object(&[("nodes", array(node)), ("edges", array(EDGE))]);
        let list = object(&[("bindings", array(bindings))]);
        object(&[("workflow", def), ("modelBindings", list)])
    }

    fn worker(id: &str, provider: &str, model: &str) -> String {
        format!(r#"{{"id":"{id}","provider":"{provider}","model":"{model}"}}"#)
    }

    fn tmp(tag: &str, text: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("wf-lint-{tag}-{}", std::process::id()));
        fs::write(&path, text).unwrap();
        path
    }

    fn lint_nodes(nodes: &str) -> String {
        lint(&simple(nodes)).unwrap().join("\n")
    }

    fn lint_flow(control: &str, nodes: &str, edges: &str) -> String {
        lint(&flow(control, nodes, edges)).unwrap().join("\n")
    }

    fn lint_err(text: &str) -> String {
        lint(&parse(text)).unwrap_err()
    }

    #[test]
    fn the_passing_fixtures_pass() {
        for text in [REPAIR_LOOP, ROUNDS, WRAPPED] {
            assert_eq!(lint(&parse(text)), Ok(Vec::new()));
        }
    }

    #[test]
    fn the_passing_fixtures_pass_from_disk() {
        for (tag, text) in [
            ("repair", REPAIR_LOOP),
            ("rounds", ROUNDS),
            ("wrapped", WRAPPED),
        ] {
            assert_eq!(run(vec![tmp(tag, text).into_os_string()]), 0);
        }
    }

    #[test]
    fn codex_agent_fails_and_names_the_node() {
        let nodes = worker("a", "CodeX-CLI", "gpt-5");
        assert_eq!(lint_nodes(&nodes), "a: banned-binding");
    }

    #[test]
    fn openrouter_models_pass() {
        let nodes = worker("a", "acp", "openrouter/x");
        assert_eq!(lint_nodes(&nodes), "");
    }

    #[test]
    fn opencode_models_pass() {
        for model in ["opencode/zen", "OpenCode/Zen"] {
            let nodes = worker("a", "acp", model);
            assert_eq!(lint_nodes(&nodes), "");
        }
    }

    #[test]
    fn opus_model_outside_judgment_fails() {
        let nodes = worker("a", "acp", "opus-4");
        assert_eq!(lint_nodes(&nodes), "a: opus-outside-judgment");
    }

    #[test]
    fn opus_agent_outside_judgment_fails() {
        let nodes = worker("a", "opus-crew", "m");
        assert_eq!(lint_nodes(&nodes), "a: opus-outside-judgment");
    }

    #[test]
    fn judgment_nodes_may_bind_opus() {
        let nodes = concat!(
            r#"{"id":"x-escalation","model":"opus-4"},"#,
            r#"{"id":"z-planning","model":"opus-4"},"#,
            r#"{"id":"y-acceptance","model":"opus-4"}"#
        );
        assert_eq!(lint_nodes(nodes), "");
    }

    #[test]
    fn opus_on_a_node_that_only_looks_like_a_judgment_node_fails() {
        // mid-acceptance has a successor, so it is not the final acceptance node.
        let nodes = concat!(
            r#"{"id":"build","provider":"claude-acp","model":"claude-sonnet-4-5"},"#,
            r#"{"id":"mid-acceptance","provider":"claude-acp","model":"claude-opus-4-1"},"#,
            r#"{"id":"final-acceptance","provider":"claude-acp","model":"claude-opus-4-1"}"#
        );
        let edges = concat!(
            r#"{"from":"build","to":"mid-acceptance","on":"success"},"#,
            r#"{"from":"mid-acceptance","to":"final-acceptance","on":"success"},"#,
            r#"{"from":"final-acceptance","to":"$end","on":"success"}"#
        );
        assert_eq!(
            lint_flow(CAP, nodes, edges),
            "mid-acceptance: opus-outside-judgment"
        );
    }

    #[test]
    fn opus_on_a_final_acceptance_node_whose_edges_all_end_the_flow_passes() {
        // Every edge leaving final-acceptance targets `$end`, so it is the
        // final acceptance node and may bind Opus.
        let nodes = concat!(
            r#"{"id":"build","provider":"claude-acp","model":"claude-sonnet-4-5"},"#,
            r#"{"id":"final-acceptance","provider":"claude-acp","model":"claude-opus-4-1"}"#
        );
        let edges = concat!(
            r#"{"from":"build","to":"final-acceptance","on":"success"},"#,
            r#"{"from":"final-acceptance","to":"$end","on":"success"}"#
        );
        assert_eq!(lint_flow(CAP, nodes, edges), "");
    }

    #[test]
    fn one_node_can_break_two_rules() {
        let nodes = worker("a", "codex", "opus-4");
        assert_eq!(
            lint_nodes(&nodes),
            "a: banned-binding\na: opus-outside-judgment"
        );
    }

    #[test]
    fn a_wrapper_binding_fails_where_its_slot_joins_it() {
        let bindings = concat!(
            r#"{"executionSlotId":"s","agentId":"codex"},"#,
            r#"{"executionSlotId":"x","agentId":"codex"}"#
        );
        let w = wrapped(SLOT, bindings);
        assert_eq!(lint(&w), Ok(vec!["dev: banned-binding".to_string()]));
    }

    #[test]
    fn an_openrouter_wrapper_model_binding_passes() {
        let bindings = r#"{"executionSlotId":"s","modelId":"openrouter/x"}"#;
        let w = wrapped(SLOT, bindings);
        assert_eq!(lint(&w), Ok(Vec::new()));
    }

    #[test]
    fn ai_dynamic_codex_bindings_fail_while_openrouter_and_opencode_pass() {
        let nodes = concat!(
            r#"{"id":"f","agentStrategy":{"mode":"fixed","provider":"codex","model":"m"}},"#,
            r#"{"id":"d","agentStrategy":{"mode":"dynamic","#,
            r#""bootstrapProvider":"codex","bootstrapModel":"opencode/zen","#,
            r#""acceptanceModel":"opus","routingPrompt":"r","#,
            r#""availableAgents":[{"model":"openrouter/x"}]}},"#,
            r#"{"id":"p","agentStrategy":{"mode":"dynamic","#,
            r#""bootstrapModel":"opencode/zen","routingPrompt":"r","#,
            r#""availableAgents":[{"model":"openrouter/x"}]}}"#
        );
        assert_eq!(
            lint_nodes(nodes),
            "f: banned-binding\nd: banned-binding\nd: opus-outside-judgment"
        );
    }

    #[test]
    fn a_strategy_without_a_known_mode_is_gold_bands_problem() {
        // The lint checks only the three rules, so bindings inside a
        // strategy Gold Band's own validator rejects (no mode) are left to
        // it; a node without an id cannot be named, so it breaks none here.
        let nodes = concat!(
            r#"{"id":"o","agentStrategy":{"provider":"codex","model":"opus-4"}},"#,
            r#"{"id":"m","agentStrategy":{"mode":"dynamic","bootstrapModel":"m"}},"#,
            r#"{}"#
        );
        assert_eq!(lint_nodes(nodes), "");
    }

    #[test]
    fn a_back_edge_without_max_attempts_fails() {
        assert_eq!(lint_flow(EMPTY, TWO, FORTH), "a: unbounded-back-edge");
    }

    #[test]
    fn a_back_edge_with_max_attempts_passes() {
        assert_eq!(lint_flow(CAP, TWO, FORTH), "");
    }

    #[test]
    fn a_back_edge_with_a_null_max_attempts_is_unbounded() {
        assert_eq!(lint_flow(CAP_NULL, TWO, FORTH), "a: unbounded-back-edge");
    }

    #[test]
    fn a_self_loop_fails() {
        let node = r#"{"id":"a","provider":"acp","model":"m"}"#;
        let edge = r#"{"from":"a","to":"a"}"#;
        assert_eq!(lint_flow(EMPTY, node, edge), "a: unbounded-back-edge");
    }

    #[test]
    fn a_diamond_without_a_back_edge_passes() {
        let nodes = concat!(
            r#"{"id":"a","provider":"acp","model":"m"},"#,
            r#"{"id":"b","provider":"acp","model":"m"},"#,
            r#"{"id":"c","provider":"acp","model":"m"},"#,
            r#"{"id":"d","provider":"acp","model":"m"}"#
        );
        let edges = concat!(
            r#"{"from":"a","to":"b"},"#,
            r#"{"from":"a","to":"c"},"#,
            r#"{"from":"b","to":"d"},"#,
            r#"{"from":"c","to":"d"}"#
        );
        assert_eq!(lint_flow(EMPTY, nodes, edges), "");
    }

    #[test]
    fn a_new_round_without_max_rounds_fails() {
        let node = r#"{"id":"p","provider":"acp","model":"m"}"#;
        let edge = r#"{"from":"p","to":"$new-round"}"#;
        assert_eq!(lint_flow(EMPTY, node, edge), "p: unbounded-rounds");
    }

    #[test]
    fn a_new_round_edge_without_a_source_names_the_workflow() {
        let edge = r#"{"to":"$new-round"}"#;
        assert_eq!(lint_flow(EMPTY, TWO, edge), "workflow: unbounded-rounds");
    }

    #[test]
    fn both_loop_rules_fail_together() {
        let edges = format!(r#"{FORTH},{{"from":"a","to":"$new-round"}}"#);
        assert_eq!(
            lint_flow(EMPTY, TWO, &edges),
            "a: unbounded-back-edge\na: unbounded-rounds"
        );
    }

    #[test]
    fn shapes_the_lint_cannot_read_fail() {
        assert_eq!(lint_err("[]"), NOT_OBJECT);
        assert_eq!(lint_err(r#"{"workflow":1}"#), BAD_WORKFLOW);
        assert_eq!(lint_err("{}"), NO_NODES);
        assert_eq!(lint_err(r#"{"nodes":1}"#), NO_NODES);
        assert_eq!(lint_err(r#"{"nodes":[]}"#), NO_EDGES);
        assert_eq!(lint_err(r#"{"nodes":[],"edges":1}"#), NO_EDGES);
    }

    #[test]
    fn bad_usage_fails() {
        assert_eq!(run(std::iter::empty()), 2);
    }

    #[test]
    fn an_unreadable_file_fails() {
        let name = format!("wf-lint-absent-{}", std::process::id());
        let path = std::env::temp_dir().join(name);
        let _ = fs::remove_file(&path);
        assert_eq!(run(vec![path.into_os_string()]), 2);
    }

    #[test]
    fn an_invalid_json_file_fails() {
        assert_eq!(run(vec![tmp("j", "{{").into_os_string()]), 2);
    }

    #[test]
    fn an_invalid_workflow_file_fails() {
        assert_eq!(run(vec![tmp("wf", "[]").into_os_string()]), 2);
    }

    #[test]
    fn a_problem_file_exits_one() {
        let broken = simple(&worker("a", "codex", "m"));
        let text = serde_json::to_string(&broken).unwrap();
        assert_eq!(run(vec![tmp("b", &text).into_os_string()]), 1);
    }

    #[test]
    fn many_files_exit_the_worst_outcome() {
        let neat = tmp("m-clean", REPAIR_LOOP).into_os_string();
        let broken = simple(&worker("a", "codex", "m"));
        let text = serde_json::to_string(&broken).unwrap();
        let wrong = tmp("m-bad", &text).into_os_string();
        let shape = tmp("m-shape", "[]").into_os_string();
        assert_eq!(run(vec![neat.clone(), neat.clone()]), 0);
        assert_eq!(run(vec![neat, wrong.clone()]), 1);
        assert_eq!(run(vec![wrong, shape]), 2);
    }

    #[test]
    fn regression_a_non_numeric_limit_does_not_bound_a_back_edge() {
        assert_eq!(
            lint_flow(r#"{"max_attempts":false}"#, TWO, FORTH),
            "a: unbounded-back-edge"
        );
        assert_eq!(
            lint_flow(r#"{"max_attempts":0}"#, TWO, FORTH),
            "a: unbounded-back-edge"
        );
    }
}
