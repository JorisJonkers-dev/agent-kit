//! Lints a Gold Band workflow definition against the model, memory and loop
//! rules every port shares (#446, replacing #371).
//!
//! Input is Gold Band's workflow JSON at version `"0.1"` (#350): either a
//! bare definition, or the `--workflow` authoring wrapper with the
//! definition nested under `workflow` and its worker bindings under
//! `modelBindings` (joined to a node by `executionSlotId`). The lint does
//! not re-validate the definition's shape; Gold Band's own validator does
//! that. It refuses only a definition that breaks one of these rules, each
//! reported as one line `<node id or "workflow">: <rule>: <message>`:
//!
//! * `banned-binding`: in the **personal** instance (`gold-band-personal`)
//!   only, a node binds an agent id or model containing `codex`
//!   (case-insensitive). The work instance (`gold-band`) may bind Codex
//!   (ADR-0011; its grant check is not this lint's concern). OpenRouter and
//!   `opencode/` bindings are allowed (owner decision, 2026-11-08);
//! * `opus-outside-judgment`: a node binds an agent id or model containing
//!   `opus` (case-insensitive) and its id does not end in `-escalation`,
//!   `-acceptance` or `-planning`;
//! * `memory-write-outside-judgment`: a node binds a memory-writing agent
//!   and its id does not end in `-acceptance` or `-escalation` (ADR-0012:
//!   every other node reads memory only);
//! * `loop-not-unrolled`: the node-to-node edges contain a cycle, or an
//!   edge targets `$new-round`. A repair or review loop must be unrolled
//!   into fixed nodes (`review-1`, `repair-1`, `review-2`, ...), so the
//!   graph is a finite DAG whose cost is known before it runs.
//!
//! # How a binding declares a memory write
//!
//! There is no memory field in Gold Band's workflow JSON, so the lint
//! defines the smallest declaration that fits it: an **agent id** (a
//! worker's `provider`, a strategy's provider fields, or a wrapper
//! binding's `agentId`) that contains the marker `memory-write`
//! (case-insensitive), for example `claude-acp+memory-write`. The agent id
//! is what the gateway grants memory access to, so the marker travels with
//! the binding and cannot drift from the node. A binding without the marker
//! reads memory only.
//!
//! Usage: `workflow-lint --instance gold-band|gold-band-personal <FILE>...`
//! (the flag is required; `--instance=NAME` also works) lints every file and
//! prefixes its
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
/// Rule: a node binds a memory-writing agent outside -acceptance/-escalation.
const MEMORY_WRITE_OUTSIDE_JUDGMENT: &str = "memory-write-outside-judgment";
/// Rule: a loop that is not unrolled into fixed nodes.
const LOOP_NOT_UNROLLED: &str = "loop-not-unrolled";
/// The subject for a problem no single node owns.
const WORKFLOW: &str = "workflow";
/// The reserved edge target that opens a new round.
const NEW_ROUND: &str = "$new-round";
/// The agent id or model name no binding may contain (case-insensitive).
const BANNED: &str = "codex";
/// The agent id or model marker of an Opus binding (case-insensitive).
const OPUS: &str = "opus";
/// The agent id marker of a binding that writes memory (case-insensitive).
pub const MEMORY_WRITE: &str = "memory-write";
/// Node id suffixes that may bind Opus.
const OPUS_SUFFIXES: [&str; 3] = ["-escalation", "-acceptance", "-planning"];
/// Node id suffixes that may write memory.
const MEMORY_SUFFIXES: [&str; 2] = ["-acceptance", "-escalation"];

/// The Gold Band instance a workflow belongs to (ADR-0011).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instance {
    /// `gold-band`, the work instance: Codex is allowed.
    Work,
    /// `gold-band-personal`: Codex is banned.
    Personal,
}

impl Instance {
    /// The instance a `--instance` value names.
    pub fn parse(name: &str) -> Option<Instance> {
        match name {
            "gold-band" => Some(Instance::Work),
            "gold-band-personal" => Some(Instance::Personal),
            _ => None,
        }
    }
}

/// One problem: the subject, the rule and a message that says what to change.
type Problem<'a> = (&'a str, &'static str, String);

/// The string of one field of a JSON object, if it is a string.
fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// The array of one field of a JSON object, if it is an array.
fn array_field<'a>(value: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    value.get(key).and_then(Value::as_array)
}

/// Whether `text` contains `needle`, case-insensitive (`needle` lowercase).
fn mentions(text: &str, needle: &str) -> bool {
    text.to_ascii_lowercase().contains(needle)
}

/// Whether any of `texts` mentions `needle`.
fn any_mentions(texts: &[&str], needle: &str) -> bool {
    texts.iter().any(|t| mentions(t, needle))
}

/// Whether a node id ends in one of `suffixes`.
fn ends_with_any(id: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|s| id.ends_with(s))
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
/// field: a worker's own `provider`/`model`, its `agentStrategy`'s fixed or
/// dynamic bindings, and the authoring-wrapper bindings joined to it by
/// `executionSlotId`.
fn node_strings<'a>(
    node: &'a Value,
    bindings: &'a [Value],
    agents: &mut Vec<&'a str>,
    models: &mut Vec<&'a str>,
) {
    push_fields(agents, node, &["provider"]);
    push_fields(models, node, &["model"]);
    if let Some(strategy) = node.get("agentStrategy") {
        // Every binding field is read whatever the mode says, so a strategy
        // with a missing or unknown mode cannot hide a binding.
        push_fields(agents, strategy, &["provider", "bootstrapProvider"]);
        push_fields(
            models,
            strategy,
            &["model", "bootstrapModel", "acceptanceModel"],
        );
        if let Some(available) = array_field(strategy, "availableAgents") {
            for agent in available {
                push_fields(agents, agent, &["provider"]);
                push_fields(models, agent, &["model"]);
            }
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
/// breaks none here (Gold Band's own validator rejects it).
fn node_problems<'a>(
    node: &'a Value,
    bindings: &'a [Value],
    instance: Instance,
) -> Vec<Problem<'a>> {
    let Some(id) = str_field(node, "id") else {
        return Vec::new();
    };
    let mut agents = Vec::new();
    let mut models = Vec::new();
    node_strings(node, bindings, &mut agents, &mut models);
    let mut problems = Vec::new();
    if instance == Instance::Personal
        && (any_mentions(&agents, BANNED) || any_mentions(&models, BANNED))
    {
        problems.push((
            id,
            BANNED_BINDING,
            format!("binds a `{BANNED}` agent or model, which the personal instance may not use (ADR-0011)"),
        ));
    }
    let opus = any_mentions(&agents, OPUS) || any_mentions(&models, OPUS);
    if opus && !ends_with_any(id, &OPUS_SUFFIXES) {
        problems.push((
            id,
            OPUS_OUTSIDE_JUDGMENT,
            "binds Opus, which only a node whose id ends in -escalation, -acceptance \
             or -planning may; bind a cheaper model or rename the judgment node"
                .to_string(),
        ));
    }
    if any_mentions(&agents, MEMORY_WRITE) && !ends_with_any(id, &MEMORY_SUFFIXES) {
        problems.push((
            id,
            MEMORY_WRITE_OUTSIDE_JUDGMENT,
            format!(
                "binds a memory-writing agent (id contains `{MEMORY_WRITE}`), which only \
                 a node whose id ends in -acceptance or -escalation may (ADR-0012); \
                 bind a read-only agent"
            ),
        ));
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

/// The loop problems of one workflow: a cycle, and a `$new-round` edge.
fn loop_problems<'a>(nodes: &'a [Value], edges: &'a [Value]) -> Vec<Problem<'a>> {
    let ids: Vec<&str> = nodes.iter().filter_map(|n| str_field(n, "id")).collect();
    let known: BTreeSet<&str> = ids.iter().copied().collect();
    let mut problems = Vec::new();
    if let Some(node) = cycle(&ids, &adjacency(&known, edges)) {
        problems.push((
            node,
            LOOP_NOT_UNROLLED,
            "sits on a cycle of edges; unroll the loop into fixed nodes \
             (for example review-1, repair-1, review-2)"
                .to_string(),
        ));
    }
    for edge in edges {
        if str_field(edge, "to") == Some(NEW_ROUND) {
            let from = str_field(edge, "from").unwrap_or(WORKFLOW);
            problems.push((
                from,
                LOOP_NOT_UNROLLED,
                format!("has an edge to `{NEW_ROUND}`; unroll the rounds into fixed nodes"),
            ));
            break;
        }
    }
    problems
}

/// Every rule problem in the parsed workflow, in a stable order (nodes in
/// document order, each node's problems in rule order, the loop rule last);
/// `Err` when the file is not workflow-shaped JSON. Each line reads
/// `<subject>: <rule>: <message>`.
pub fn lint(root: &Value, instance: Instance) -> Result<Vec<String>, String> {
    let def = definition(root)?;
    let nodes = field_array(def, "nodes")?;
    let edges = field_array(def, "edges")?;
    let bound = bindings(root);
    let mut problems: Vec<Problem> = Vec::new();
    for node in nodes {
        problems.extend(node_problems(node, bound, instance));
    }
    problems.extend(loop_problems(nodes, edges));
    Ok(problems
        .into_iter()
        .map(|(subject, rule, message)| format!("{subject}: {rule}: {message}"))
        .collect())
}

/// Lints one file and prints its problems; returns its exit code (0 clean,
/// 1 problems, 2 unreadable or invalid).
fn lint_file(path: &Path, shown: &str, instance: Instance) -> i32 {
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
    let problems = match lint(&root, instance) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("workflow-lint: {shown}: invalid workflow: {e}");
            return 2;
        }
    };
    for problem in &problems {
        eprintln!("workflow-lint: {shown}: {problem}");
    }
    i32::from(!problems.is_empty())
}

/// The instance and files a command line names; `None` on bad usage.
fn parse_args(args: &[OsString]) -> Option<(Instance, Vec<&OsString>)> {
    let mut instance = None;
    let mut files = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let text = arg.to_string_lossy();
        if text == "--instance" {
            instance = Instance::parse(&iter.next()?.to_string_lossy());
            instance?;
        } else if let Some(name) = text.strip_prefix("--instance=") {
            instance = Some(Instance::parse(name)?);
        } else {
            files.push(arg);
        }
    }
    if files.is_empty() {
        return None;
    }
    Some((instance?, files))
}

/// Runs one command line and returns its exit code.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    let Some((instance, files)) = parse_args(&args) else {
        eprintln!("usage: workflow-lint --instance gold-band|gold-band-personal <FILE>...");
        return 2;
    };
    let mut code = 0;
    for arg in files {
        let shown = arg.to_string_lossy();
        code = code.max(lint_file(Path::new(arg), &shown, instance));
    }
    if code == 0 {
        println!("workflow-lint: ok");
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDGE: &str = r#"{"from":"dev","to":"$end"}"#;

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    fn lint_def(nodes: &str, edges: &str) -> Vec<String> {
        let text = format!(r#"{{"nodes":[{nodes}],"edges":[{edges}]}}"#);
        lint(&parse(&text), Instance::Personal).unwrap()
    }

    fn lint_nodes(nodes: &str) -> Vec<String> {
        lint_def(nodes, EDGE)
    }

    fn worker(id: &str, provider: &str, model: &str) -> String {
        format!(r#"{{"id":"{id}","provider":"{provider}","model":"{model}"}}"#)
    }

    fn tmp(tag: &str, text: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("wf-lint-{tag}-{}", std::process::id()));
        fs::write(&path, text).unwrap();
        path
    }

    fn go(files: Vec<OsString>) -> i32 {
        let mut args = vec![
            OsString::from("--instance"),
            OsString::from("gold-band-personal"),
        ];
        args.extend(files);
        run(args)
    }

    fn rules(problems: &[String]) -> Vec<String> {
        problems
            .iter()
            .map(|p| p.splitn(3, ": ").take(2).collect::<Vec<_>>().join(": "))
            .collect()
    }

    #[test]
    fn codex_is_banned_and_names_the_node() {
        let found = lint_nodes(&worker("a", "CodeX-CLI", "gpt-5"));
        assert_eq!(rules(&found), ["a: banned-binding"]);
    }

    #[test]
    fn openrouter_and_opencode_bindings_pass() {
        for model in ["openrouter/x", "opencode/zen", "OpenCode/Zen"] {
            assert!(lint_nodes(&worker("a", "acp", model)).is_empty());
        }
    }

    #[test]
    fn opus_passes_only_on_the_three_judgment_suffixes() {
        for id in ["x-escalation", "x-acceptance", "x-planning"] {
            assert!(lint_nodes(&worker(id, "acp", "Opus-4")).is_empty(), "{id}");
        }
        for id in ["a", "x-escalations", "acceptance", "x-review"] {
            let found = lint_nodes(&worker(id, "acp", "opus-4"));
            assert_eq!(rules(&found), [format!("{id}: opus-outside-judgment")]);
        }
        let found = lint_nodes(&worker("a", "opus-crew", "m"));
        assert_eq!(rules(&found), ["a: opus-outside-judgment"]);
    }

    #[test]
    fn memory_write_passes_only_on_acceptance_and_escalation() {
        for id in ["x-acceptance", "x-escalation"] {
            assert!(lint_nodes(&worker(id, "acp+Memory-Write", "m")).is_empty());
        }
        for id in ["a", "x-planning"] {
            let found = lint_nodes(&worker(id, "acp+memory-write", "m"));
            assert_eq!(
                rules(&found),
                [format!("{id}: memory-write-outside-judgment")]
            );
        }
    }

    #[test]
    fn the_marker_only_counts_in_an_agent_id_not_a_model() {
        assert!(lint_nodes(&worker("a", "acp", "memory-write")).is_empty());
    }

    #[test]
    fn one_node_can_break_three_rules() {
        let found = lint_nodes(&worker("a", "codex+memory-write", "opus"));
        assert_eq!(
            rules(&found),
            [
                "a: banned-binding",
                "a: opus-outside-judgment",
                "a: memory-write-outside-judgment"
            ]
        );
    }

    #[test]
    fn a_wrapper_binding_applies_where_its_slot_joins_it() {
        let text = r#"{"workflow":{"nodes":[
            {"id":"dev","executionSlotId":"s"},{"id":"other","executionSlotId":"t"}],
            "edges":[]},
          "modelBindings":{"bindings":[
            {"executionSlotId":"s","agentId":"a+memory-write","modelId":"opus"},
            {"executionSlotId":"u","agentId":"codex"}]}}"#;
        let found = lint(&parse(text), Instance::Personal).unwrap();
        assert_eq!(
            rules(&found),
            [
                "dev: opus-outside-judgment",
                "dev: memory-write-outside-judgment"
            ]
        );
    }

    #[test]
    fn strategy_bindings_are_checked_in_fixed_and_dynamic_modes() {
        let nodes = concat!(
            r#"{"id":"f","agentStrategy":{"mode":"fixed","provider":"codex","model":"m"}},"#,
            r#"{"id":"d","agentStrategy":{"mode":"dynamic","#,
            r#""bootstrapProvider":"a+memory-write","bootstrapModel":"opencode/zen","#,
            r#""acceptanceModel":"opus","routingPrompt":"r","#,
            r#""availableAgents":[{"provider":"codex","model":"openrouter/x"}]}},"#,
            r#"{"id":"p","agentStrategy":{"mode":"dynamic","#,
            r#""bootstrapModel":"opencode/zen","availableAgents":[{"model":"openrouter/x"}]}}"#
        );
        assert_eq!(
            rules(&lint_nodes(nodes)),
            [
                "f: banned-binding",
                "d: banned-binding",
                "d: opus-outside-judgment",
                "d: memory-write-outside-judgment"
            ]
        );
    }

    #[test]
    fn a_strategy_without_a_known_mode_cannot_hide_a_binding() {
        let nodes = r#"{"id":"o","agentStrategy":{"provider":"codex","model":"opus-4"}}"#;
        assert_eq!(
            rules(&lint_nodes(nodes)),
            ["o: banned-binding", "o: opus-outside-judgment"]
        );
    }

    #[test]
    fn a_node_without_an_id_is_gold_bands_problem() {
        assert!(lint_nodes(r#"{"provider":"codex"}"#).is_empty());
    }

    const TWO: &str = concat!(
        r#"{"id":"a","provider":"acp","model":"m"},"#,
        r#"{"id":"b","provider":"acp","model":"m"}"#
    );

    #[test]
    fn a_back_edge_is_a_loop() {
        let edges = r#"{"from":"a","to":"b"},{"from":"b","to":"a"}"#;
        assert_eq!(rules(&lint_def(TWO, edges)), ["a: loop-not-unrolled"]);
    }

    #[test]
    fn a_back_edge_is_a_loop_even_with_a_limit_declared() {
        let text = format!(
            r#"{{"control":{{"max_attempts":2,"max_rounds":3}},"nodes":[{TWO}],
            "edges":[{{"from":"a","to":"b"}},{{"from":"b","to":"a"}}]}}"#
        );
        let found = lint(&parse(&text), Instance::Personal).unwrap();
        assert_eq!(rules(&found), ["a: loop-not-unrolled"]);
    }

    #[test]
    fn a_self_loop_is_a_loop() {
        let node = worker("a", "acp", "m");
        let found = lint_def(&node, r#"{"from":"a","to":"a"}"#);
        assert_eq!(rules(&found), ["a: loop-not-unrolled"]);
    }

    #[test]
    fn a_diamond_and_edges_to_unknown_nodes_are_not_loops() {
        let nodes = r#"{"id":"a"},{"id":"b"},{"id":"c"},{"id":"d"}"#;
        let edges = concat!(
            r#"{"from":"a","to":"b"},{"from":"a","to":"c"},"#,
            r#"{"from":"b","to":"d"},{"from":"c","to":"d"},"#,
            r#"{"from":"d","to":"$end"},{"from":"d","to":"ghost"},{"from":"ghost","to":"a"},{}"#
        );
        assert!(lint_def(nodes, edges).is_empty());
    }

    #[test]
    fn a_new_round_edge_is_a_loop() {
        let node = worker("a", "acp", "m");
        let found = lint_def(&node, r#"{"from":"a","to":"$new-round"}"#);
        assert_eq!(rules(&found), ["a: loop-not-unrolled"]);
        assert!(found[0].contains("`$new-round`"));
    }

    #[test]
    fn a_new_round_edge_without_a_source_names_the_workflow() {
        let node = worker("a", "acp", "m");
        let found = lint_def(&node, r#"{"to":"$new-round"}"#);
        assert_eq!(rules(&found), ["workflow: loop-not-unrolled"]);
    }

    #[test]
    fn shapes_the_lint_cannot_read_are_errors() {
        let err = |text: &str| lint(&parse(text), Instance::Work).unwrap_err();
        assert_eq!(err("[]"), "the file is not a JSON object");
        assert_eq!(err(r#"{"workflow":1}"#), "workflow is not a JSON object");
        assert_eq!(err("{}"), "nodes is not an array");
        assert_eq!(err(r#"{"nodes":1}"#), "nodes is not an array");
        assert_eq!(err(r#"{"nodes":[]}"#), "edges is not an array");
        assert_eq!(err(r#"{"nodes":[],"edges":1}"#), "edges is not an array");
    }

    #[test]
    fn bad_usage_exits_two() {
        assert_eq!(run(std::iter::empty::<OsString>()), 2);
    }

    #[test]
    fn an_unreadable_file_exits_two() {
        let path = std::env::temp_dir().join(format!("wf-lint-absent-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        assert_eq!(go(vec![path.into_os_string()]), 2);
    }

    #[test]
    fn invalid_json_and_invalid_workflows_exit_two() {
        assert_eq!(go(vec![tmp("j", "{{").into_os_string()]), 2);
        assert_eq!(go(vec![tmp("wf", "[]").into_os_string()]), 2);
    }

    #[test]
    fn many_files_exit_the_worst_outcome() {
        let neat = tmp("m-clean", r#"{"nodes":[],"edges":[]}"#).into_os_string();
        let text = format!(r#"{{"nodes":[{}],"edges":[]}}"#, worker("a", "codex", "m"));
        let wrong = tmp("m-bad", &text).into_os_string();
        let shape = tmp("m-shape", "[]").into_os_string();
        assert_eq!(go(vec![neat.clone(), neat.clone()]), 0);
        assert_eq!(go(vec![neat, wrong.clone()]), 1);
        assert_eq!(go(vec![wrong, shape]), 2);
    }

    #[test]
    fn codex_is_allowed_in_the_work_instance_and_refused_in_the_personal_one() {
        let text = format!(
            r#"{{"nodes":[{}],"edges":[]}}"#,
            worker("a", "codex", "gpt-5")
        );
        let root = parse(&text);
        assert!(lint(&root, Instance::Work).unwrap().is_empty());
        let found = lint(&root, Instance::Personal).unwrap();
        assert_eq!(rules(&found), ["a: banned-binding"]);
        assert!(found[0].contains("personal instance"));
    }

    #[test]
    fn the_instance_flag_is_required_and_must_name_an_instance() {
        let file = tmp("inst", r#"{"nodes":[],"edges":[]}"#).into_os_string();
        let flag = |s: &str| OsString::from(s);
        assert_eq!(run(vec![file.clone()]), 2);
        assert_eq!(run(vec![flag("--instance")]), 2);
        assert_eq!(run(vec![flag("--instance"), flag("nope"), file.clone()]), 2);
        assert_eq!(run(vec![flag("--instance=nope"), file.clone()]), 2);
        assert_eq!(run(vec![flag("--instance=gold-band")]), 2);
        assert_eq!(run(vec![flag("--instance=gold-band"), file.clone()]), 0);
        assert_eq!(
            run(vec![file, flag("--instance"), flag("gold-band-personal")]),
            0
        );
    }
}
