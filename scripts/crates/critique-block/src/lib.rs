//! The generator of the shared critique block (#795, audit "Generated critique
//! block"): every artifact a Gold Band template has a model write is critiqued
//! by other families, then accepted, revised or escalated, never just passed on.
//!
//! One block is generated per stage (`spec`, `tickets`) into a template, after
//! the node that writes the artifact (the *writer*):
//!
//! ```text
//! writer -> [pre-check] -> CRIT(1) -> accept-1 --success--> next
//!                                     accept-1 --failure--> revise-1 -> [pre-check]
//!                                       -> CRIT(2) -> accept-2 --success--> next
//!                                                     accept-2 --failure--> escalation
//!                                                       -> escalation-check (manual) -> $end
//! ```
//!
//! * `CRIT(N)` is the critique runner chain of the stage's own template
//!   (`critique-<stage>.json`: prepare, fan-out, the orchestrator's verdicts and
//!   its needs-human check, refute, record), cloned node by node, each gate with
//!   its copier. Round 2 appends `-2` to every id and artifact of the chain and
//!   asks `prepare` for fresh critics over the first round's run directory.
//! * `accept-<stage>-N` is a shim that reads round N's record and answers
//!   `{"open_blocking": n}`; it accepts when `n` is 0.
//! * `<stage>-revise-1` is the writer's own binding applying the accepted
//!   findings of round 1 and nothing else.
//! * `<stage>-escalation` is the judge after a second failed round and
//!   `<stage>-escalation-check` the manual check that ends the run needing the
//!   owner. No path from a failed second round reaches `next`.
//!
//! Loops are unrolled (two rounds, the most a stage runs), so the graph is a
//! DAG and its cost is known before it runs. Every revise is followed by a
//! critic before any exit, and the only edges into `next` come from an accept
//! node. A node bound to Codex (the work instance) is preceded by a grant-check
//! node like the templates' own, whose failure edge goes to the template's
//! refusal node.
//!
//! The recipes (`installer/gold-band/critique-blocks.json`) name, per block, the
//! template, stage, writer, next node and the bindings; the generator rewrites
//! the templates in place and is idempotent: it removes what it generated before
//! (the nodes whose ids it owns, which include a template's own hand-written
//! runner chain for the stage) and generates again. `check` regenerates in
//! memory and fails when a template differs.
//!
//! Usage:
//!
//! ```text
//! critique-block generate --recipes FILE --dir DIR
//! critique-block check --recipes FILE --dir DIR
//! ```
//!
//! Exit 0 done or current, 1 a template differs (`check`), 2 bad usage or an
//! unreadable or invalid input.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use model_gateway::json::{self, object, Value};

/// The crate's name, which prefixes every message line.
pub const NAME: &str = "critique-block";
/// The most critique rounds a stage runs.
const ROUNDS: u32 = 2;
const END: &str = "$end";

/// A node's string field, or "".
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.as_object().and_then(|o| o.get(key)).and_then(Value::as_str).unwrap_or("")
}

fn set(value: &mut Value, key: &str, to: Value) {
    if let Value::Obj(object) = value {
        object.set(key, to);
    }
}

fn set_text(value: &mut Value, key: &str, to: &str) {
    set(value, key, Value::str(to));
}

/// A member of an object member: `value[outer][key]`.
fn nested<'a>(value: &'a mut Value, outer: &str, key: &str) -> Option<&'a mut Value> {
    match value {
        Value::Obj(object) => match object.get_mut(outer) {
            Some(Value::Obj(inner)) => inner.get_mut(key),
            _ => None,
        },
        _ => None,
    }
}

/// An agent id and a model id.
type Binding = (String, String);

/// One block of the recipes file.
struct Recipe {
    template: String,
    stage: String,
    writer: String,
    next: String,
    label: String,
    artifact: String,
    orchestrator: Binding,
    needs_human: Option<Binding>,
    escalation: Binding,
    escalation_check: Binding,
    precheck: Option<(String, String)>,
    grant: Option<(String, String)>,
    routing_note: Option<String>,
}

fn member<'a>(of: &'a Value, key: &str, whose: &str) -> Result<&'a Value, String> {
    of.as_object().and_then(|o| o.get(key)).ok_or(format!("{whose}: {key} is required"))
}

fn string(of: &Value, key: &str, whose: &str) -> Result<String, String> {
    member(of, key, whose)?.as_str().map(str::to_string).ok_or(format!("{whose}: {key} must be a string"))
}

fn binding(of: &Value, key: &str, whose: &str) -> Result<Binding, String> {
    let inner = member(of, key, whose)?;
    let whose = format!("{whose}.{key}");
    Ok((string(inner, "provider", &whose)?, string(inner, "model", &whose)?))
}

/// An optional member, read with `read` when present.
fn optional<T>(of: &Value, key: &str, read: impl Fn(&Value) -> Result<T, String>) -> Result<Option<T>, String> {
    of.as_object().and_then(|o| o.get(key)).map(read).transpose()
}

impl Recipe {
    fn read(of: &Value, at: usize) -> Result<Recipe, String> {
        let whose = format!("blocks[{at}]");
        let pair = |value: &Value, a: &str, b: &str| -> Result<(String, String), String> {
            Ok((string(value, a, &whose)?, string(value, b, &whose)?))
        };
        Ok(Recipe {
            template: string(of, "template", &whose)?,
            stage: string(of, "stage", &whose)?,
            writer: string(of, "writer", &whose)?,
            next: string(of, "next", &whose)?,
            label: string(of, "label", &whose)?,
            artifact: string(of, "artifact", &whose)?,
            orchestrator: binding(of, "orchestrator", &whose)?,
            needs_human: optional(of, "needs_human", |_| binding(of, "needs_human", &whose))?,
            escalation: binding(of, "escalation", &whose)?,
            escalation_check: binding(of, "escalation_check", &whose)?,
            precheck: optional(of, "precheck", |v| pair(v, "command", "purpose"))?,
            grant: optional(of, "grant", |v| pair(v, "refusal", "prototype"))?,
            routing_note: optional(of, "routing_note", |v| v.as_str().map(str::to_string).ok_or(format!("{whose}: routing_note must be a string")))?,
        })
    }
}

/// The recipes of a recipes file.
fn recipes(text: &str) -> Result<Vec<Recipe>, String> {
    let root = json::parse(text)?;
    let Some(Value::Arr(blocks)) = root.as_object().and_then(|o| o.get("blocks")) else {
        return Err("the recipes file needs a blocks list".to_string());
    };
    blocks.iter().enumerate().map(|(at, block)| Recipe::read(block, at)).collect()
}

/// A template's definition parts, taken apart.
struct Template {
    root: Value,
}

impl Template {
    fn list(&mut self, outer: &[&str]) -> Option<&mut Vec<Value>> {
        let mut at = &mut self.root;
        for key in outer {
            at = match at {
                Value::Obj(object) => object.get_mut(key)?,
                _ => return None,
            };
        }
        match at {
            Value::Arr(list) => Some(list),
            _ => None,
        }
    }
}

fn id_of(node: &Value) -> String {
    text(node, "id").to_string()
}

/// The node with an id, cloned.
fn node_named(nodes: &[Value], id: &str, file: &str) -> Result<Value, String> {
    nodes.iter().find(|n| text(n, "id") == id).cloned().ok_or(format!("{file} has no node {id}"))
}

fn slot_of(id: &str) -> String {
    format!("slot-{id}")
}

/// A clone of `node` as the node `id`: its slot, and its provider when that was its own id.
fn renamed(node: &Value, id: &str) -> Value {
    let mut clone = node.clone();
    if text(&clone, "provider") == text(node, "id") {
        set_text(&mut clone, "provider", id);
    }
    set_text(&mut clone, "id", id);
    set_text(&mut clone, "executionSlotId", &slot_of(id));
    clone
}

fn with_artifact(mut node: Value, artifact: &str) -> Value {
    if let Some(slot) = nested(&mut node, "output", "artifact") {
        *slot = Value::str(artifact);
    }
    node
}

fn bound(mut node: Value, to: &Binding) -> Value {
    set_text(&mut node, "provider", &to.0);
    set_text(&mut node, "model", &to.1);
    node
}

fn suffix(round: u32) -> String {
    if round == 1 {
        String::new()
    } else {
        format!("-{round}")
    }
}

/// A node of the stage template's chain as round `round` of it, with the recipe's overrides.
fn chain_node(recipe: &Recipe, round: u32, base: &Value) -> Value {
    let old = id_of(base);
    let sfx = suffix(round);
    let mut node = renamed(base, &format!("{old}{sfx}"));
    let artifact = text(base.as_object().and_then(|o| o.get("output")).unwrap_or(base), "artifact");
    if !artifact.is_empty() {
        node = with_artifact(node, &format!("{artifact}{sfx}"));
    }
    let is = |ending: &str| old.ends_with(ending);
    if is("-verdicts") {
        node = bound(node, &recipe.orchestrator);
    }
    if let (true, Some(to)) = (is("-needs-human"), &recipe.needs_human) {
        node = bound(node, to);
    }
    let mut goal = text(&node, "goal").to_string();
    let runner_gate = old.starts_with("gate-") && (is("-prepare") || is("-fan-out"));
    if let (true, Some(note)) = (runner_gate, &recipe.routing_note) {
        goal = format!("{goal} {note}");
    }
    if round > 1 && old.starts_with("gate-") && is("-prepare") {
        goal.push_str(" This is round 2: pass the first round's run directory as previous_run_dir, so fresh critics review the revised artifact and the first round's findings are not raised again.");
    }
    set_text(&mut node, "goal", &goal);
    node
}

fn edge(from: &str, to: &str, on: &str) -> Value {
    object(vec![("from", Value::str(from)), ("to", Value::str(to)), ("on", Value::str(on))])
}

/// The runner chain's node ids of a stage, in the stage template.
fn chain_ids(stage: &str) -> Vec<String> {
    let gate = |step: &str| format!("gate-critique-{stage}-{step}");
    let copy = |step: &str| format!("copy-critique-{stage}-{step}");
    vec![
        gate("prepare"),
        copy("prepare"),
        gate("fan-out"),
        copy("fan-out"),
        format!("{stage}-verdicts"),
        format!("{stage}-needs-human"),
        gate("refute"),
        copy("refute"),
        gate("record"),
        copy("record"),
    ]
}

/// The pre-check gate and copier of a round: the stage template's gate shims as the recipe's command.
fn precheck_nodes(recipe: &Recipe, round: u32, command: &str, purpose: &str, gate: &Value, copy: &Value) -> [Value; 2] {
    let (stage, sfx) = (&recipe.stage, suffix(round));
    let gate_id = format!("gate-{stage}-precheck{sfx}");
    let mut first = with_artifact(renamed(gate, &gate_id), &format!("{gate_id}-status"));
    set_text(&mut first, "goal", &format!("Run the pinned gold-band-gate wrapper around `{command}` (the mechanical pre-check: {purpose}) before the critics of {} and answer with its exit status as one fenced json block and nothing else.", recipe.label));
    let mut second = with_artifact(renamed(copy, &format!("copy-{stage}-precheck{sfx}")), &format!("{gate_id}-report"));
    set_text(&mut second, "goal", &format!("Run the pinned gold-band-gate verify copier for the pre-check gate of {} and answer with the report it prints as one fenced json block and nothing else.", recipe.label));
    [first, second]
}

/// The accept shim of a round: the open blocking findings of its record, 0 to accept.
fn accept_node(recipe: &Recipe, round: u32, proto: &Value) -> Value {
    let id = format!("accept-{}-{round}", recipe.stage);
    let mut node = renamed(proto, &id);
    set_text(&mut node, "provider", &id);
    set_text(&mut node, "goal", &format!("Run the pinned accept shim for round {round} of {}: it reads the round's record report in the run directory and answers open_blocking, the number of accepted findings of blocking severity still open, as one fenced json block and nothing else. The round is accepted when it is 0.", recipe.label));
    set(&mut node, "output", object(vec![
        ("kind", Value::str("json")),
        ("artifact", Value::str(&format!("{}-open-blocking-{round}", recipe.stage))),
        ("schema", object(vec![("open_blocking", Value::Num(0.0))])),
    ]));
    set(&mut node, "success_condition", object(vec![("path", Value::str("$.open_blocking")), ("equals", Value::Num(0.0))]));
    node
}

/// The writer applying the accepted findings of round `round`.
fn revise_node(recipe: &Recipe, round: u32, writer: &Value) -> Value {
    let id = format!("{}-revise-{round}", recipe.stage);
    let mut node = with_artifact(renamed(writer, &id), &format!("{id}-write"));
    set_text(&mut node, "goal", &format!("You are the author of {}. Round {round} of {} left accepted findings: read the round's record report and its accepted findings in the run directory and apply exactly those to {}, nothing the findings do not call for and no finding that was not accepted. Fresh critics review the result next. Answer with one fenced json block holding ok and nothing else.", recipe.label, recipe.label, recipe.artifact));
    node
}

/// The escalation judge and its manual check, after the last round failed.
fn escalation_nodes(recipe: &Recipe, writer: &Value, needs_human: &Value) -> [Value; 2] {
    let stage = &recipe.stage;
    let id = format!("{stage}-escalation");
    let mut judge = bound(with_artifact(renamed(writer, &id), &format!("{id}-judgment")), &recipe.escalation);
    set_text(&mut judge, "goal", &format!("You are the escalation judge for {}. Both critique rounds ended with accepted blocking findings still open, so nothing passes automatically. Read the two rounds' record reports and the artifact and state to the owner, in one short paragraph, which findings are still open, whether the author or the findings are at fault and what you recommend. Do not edit any file. Answer with one fenced json block holding ok and nothing else.", recipe.label));
    let mut check = bound(renamed(needs_human, &format!("{id}-check")), &recipe.escalation_check);
    set_text(&mut check, "goal", &format!("{} could not be accepted in two rounds: read the escalation judge's paragraph and state it to the owner. Do not write any file; the manual check after your answer is where the owner decides, and the run ends here whatever the owner decides: no later stage runs on this artifact automatically.", recipe.label));
    [judge, check]
}

/// What a block generates: nodes in document order and edges in wiring order.
struct Block {
    nodes: Vec<Value>,
    edges: Vec<Value>,
}

fn block(recipe: &Recipe, stage_nodes: &[Value], writer: &Value, stage_file: &str) -> Result<Block, String> {
    let stage = recipe.stage.as_str();
    let ids = chain_ids(stage);
    let base: Vec<Value> = ids.iter().map(|id| node_named(stage_nodes, id, stage_file)).collect::<Result<_, _>>()?;
    let needs_human = format!("{stage}-needs-human");
    let mut out = Block { nodes: Vec::new(), edges: Vec::new() };
    let mut entries: Vec<String> = Vec::new();
    let mut accepts: Vec<String> = Vec::new();
    for round in 1..=ROUNDS {
        let sfx = suffix(round);
        let mut steps: Vec<Value> = Vec::new();
        if let Some((command, purpose)) = &recipe.precheck {
            steps.extend(precheck_nodes(recipe, round, command, purpose, &base[0], &base[1]));
        }
        steps.extend(base.iter().map(|b| chain_node(recipe, round, b)));
        let accept = accept_node(recipe, round, &base[0]);
        accepts.push(id_of(&accept));
        steps.push(accept);
        let step_ids: Vec<String> = steps.iter().map(id_of).collect();
        entries.push(step_ids[0].clone());
        let human = format!("{needs_human}{sfx}");
        let verdicts = format!("{stage}-verdicts{sfx}");
        let refute = format!("gate-critique-{stage}-refute{sfx}");
        let line: Vec<&String> = step_ids.iter().filter(|id| **id != human).collect();
        for pair in line.windows(2) {
            out.edges.push(edge(pair[0], pair[1], "success"));
        }
        out.edges.push(edge(&verdicts, &human, "failure"));
        out.edges.push(edge(&human, &refute, "success"));
        out.nodes.extend(steps);
        out.edges.push(edge(&accepts[round as usize - 1], &recipe.next, "success"));
        if round == 1 {
            let revise = revise_node(recipe, round, writer);
            out.edges.push(edge(&accepts[0], &id_of(&revise), "failure"));
            out.nodes.push(revise);
        }
    }
    let revise = format!("{stage}-revise-1");
    out.edges.push(edge(&revise, &entries[1], "success"));
    let [judge, check] = escalation_nodes(recipe, writer, &base[5]);
    out.edges.push(edge(&accepts[1], &id_of(&judge), "failure"));
    out.edges.push(edge(&id_of(&judge), &id_of(&check), "success"));
    out.edges.push(edge(&id_of(&check), END, "success"));
    out.edges.push(edge(&recipe.writer, &entries[0], "success"));
    out.nodes.extend([judge, check]);
    Ok(out)
}

/// Puts a grant-check node, cloned from the template's own, before every node bound to Codex.
fn grants(recipe: &Recipe, out: &mut Block, proto: &Value) {
    let Some((refusal, _)) = &recipe.grant else { return };
    let codex: Vec<String> = out.nodes.iter().filter(|n| text(n, "provider").to_ascii_lowercase().contains("codex")).map(id_of).collect();
    let named = id_of(proto).trim_start_matches("grant-check-").to_string();
    for id in codex {
        let grant_id = format!("grant-check-{id}");
        let mut grant = renamed(proto, &grant_id);
        set_text(&mut grant, "provider", &format!("grant-{id}"));
        set_text(&mut grant, "goal", &text(proto, "goal").replace(&named, &id));
        let at = out.nodes.iter().position(|n| id_of(n) == id).unwrap_or(0);
        out.nodes.insert(at, grant);
        let mut redirected = false;
        let mut rewired: Vec<Value> = Vec::new();
        for edge_value in std::mem::take(&mut out.edges) {
            let into = text(&edge_value, "to") == id;
            let mut edge_value = edge_value;
            if into {
                set_text(&mut edge_value, "to", &grant_id);
            }
            rewired.push(edge_value);
            if into && !redirected {
                redirected = true;
                rewired.push(edge(&grant_id, &id, "success"));
                rewired.push(edge(&grant_id, refusal, "failure"));
            }
        }
        out.edges = rewired;
    }
}

/// The ids a block owns: every node it generates.
fn owned(out: &Block) -> BTreeSet<String> {
    out.nodes.iter().map(id_of).collect()
}

/// A binding entry of a node.
fn binding_of(node: &Value) -> Value {
    object(vec![
        ("executionSlotId", Value::str(text(node, "executionSlotId"))),
        ("agentId", Value::str(text(node, "provider"))),
        ("modelId", Value::str(text(node, "model"))),
    ])
}

fn nodes_of(template: &mut Template) -> Result<&mut Vec<Value>, String> {
    template.list(&["workflow", "nodes"]).ok_or("the template has no workflow.nodes list".to_string())
}

/// Regenerates one block into the template.
fn apply(template: &mut Template, recipe: &Recipe, stage_nodes: &[Value], stage_file: &str) -> Result<(), String> {
    let writer = node_named(nodes_of(template)?, &recipe.writer, &recipe.template)?;
    let mut out = block(recipe, stage_nodes, &writer, stage_file)?;
    if let Some((_, prototype)) = &recipe.grant {
        let proto = node_named(nodes_of(template)?, prototype, &recipe.template)?;
        grants(recipe, &mut out, &proto);
    }
    let owned = owned(&out);
    // Nodes: drop what an earlier run (or the template's own chain) holds under the block's ids.
    let nodes = nodes_of(template)?;
    nodes.retain(|n| !owned.contains(&id_of(n)));
    let at = nodes.iter().position(|n| id_of(n) == recipe.writer).map_or(0, |i| i + 1);
    let slots: Vec<String> = out.nodes.iter().map(|n| text(n, "executionSlotId").to_string()).collect();
    let writer_slot = text(&writer, "executionSlotId").to_string();
    let new_bindings: Vec<Value> = out.nodes.iter().map(binding_of).collect();
    nodes.splice(at..at, out.nodes);
    // Edges: the writer's success edge is where the block goes.
    let edges = template.list(&["workflow", "edges"]).ok_or("the template has no workflow.edges list".to_string())?;
    let first = edges.iter().position(|e| text(e, "from") == recipe.writer && text(e, "on") == "success").unwrap_or(edges.len());
    let doomed = |e: &Value| owned.contains(text(e, "from")) || owned.contains(text(e, "to")) || (text(e, "from") == recipe.writer && text(e, "on") == "success");
    let before = edges.iter().take(first).filter(|e| !doomed(e)).count();
    // The writer's own edge into the first node of the block is part of the block, so it is dropped and made again.
    edges.retain(|e| !doomed(e));
    edges.splice(before..before, out.edges);
    // Bindings follow their nodes.
    if let Some(list) = template.list(&["modelBindings", "bindings"]) {
        list.retain(|b| !slots.iter().any(|s| s == text(b, "executionSlotId")));
        let after = list.iter().position(|b| text(b, "executionSlotId") == writer_slot).map_or(list.len(), |i| i + 1);
        list.splice(after..after, new_bindings);
    }
    Ok(())
}

/// The templates a recipes file produces, as (file name, text), in recipe order and once per template.
fn generated(recipes_text: &str, dir: &Path) -> Result<Vec<(String, String)>, String> {
    let recipes = recipes(recipes_text)?;
    let mut done: Vec<(String, Template)> = Vec::new();
    for recipe in &recipes {
        if !done.iter().any(|(name, _)| *name == recipe.template) {
            let loaded = read_json(&dir.join(&recipe.template))?;
            done.push((recipe.template.clone(), Template { root: loaded }));
        }
        let stage_file = format!("critique-{}.json", recipe.stage);
        let mut stage = Template { root: read_json(&dir.join(&stage_file))? };
        let stage_nodes = nodes_of(&mut stage)?.clone();
        let at = done.iter().position(|(name, _)| *name == recipe.template).unwrap_or(0);
        apply(&mut done[at].1, recipe, &stage_nodes, &stage_file)?;
    }
    Ok(done.into_iter().map(|(name, template)| (name, format!("{}\n", json::pretty(&template.root)))).collect())
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: cannot read: {e}", path.display()))?;
    json::parse(&text).map_err(|e| format!("{}: not JSON: {e}", path.display()))
}

fn usage() -> String {
    format!("usage: {NAME} generate|check --recipes FILE --dir DIR")
}

/// Runs one command line and returns its exit code.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<String> = args.into_iter().map(|a| a.to_string_lossy().into_owned()).collect();
    match command(&args) {
        Ok(code) => code,
        Err(why) => {
            eprintln!("{NAME}: {why}");
            2
        }
    }
}

fn command(args: &[String]) -> Result<i32, String> {
    let mut words = args.iter();
    let verb = words.next().map(String::as_str).filter(|v| matches!(*v, "generate" | "check")).ok_or_else(usage)?;
    let (mut recipes_path, mut dir) = (None, None);
    while let Some(flag) = words.next() {
        let slot = match flag.as_str() {
            "--recipes" => &mut recipes_path,
            "--dir" => &mut dir,
            _ => return Err(usage()),
        };
        *slot = Some(words.next().ok_or_else(usage)?.clone());
    }
    let (Some(recipes_path), Some(dir)) = (recipes_path, dir) else { return Err(usage()) };
    let recipes_text = fs::read_to_string(&recipes_path).map_err(|e| format!("{recipes_path}: cannot read: {e}"))?;
    let dir = Path::new(&dir);
    let mut stale = 0;
    for (name, text) in generated(&recipes_text, dir)? {
        let path = dir.join(&name);
        let current = fs::read_to_string(&path).unwrap_or_default();
        if current == text {
            continue;
        }
        if verb == "check" {
            eprintln!("{NAME}: {name} differs from what the recipes generate; run `{NAME} generate`");
            stale += 1;
        } else {
            fs::write(&path, text).map_err(|e| format!("{}: cannot write: {e}", path.display()))?;
            println!("{NAME}: wrote {name}");
        }
    }
    Ok(i32::from(stale > 0))
}

#[cfg(test)]
mod tests;
