//! The headless proof itself: a throw-away Gold Band home whose every agent id
//! is the stub ACP agent, one `gold-band run` per scenario, and the record.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::record::{Outcome, Record, ScenarioResult};
use crate::refuse;
use crate::stub::{Role, NEEDS_HUMAN, OPEN_QUESTIONS};

/// What the harness needs to know about a workflow to stand the stub in for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowInfo {
    /// The workflow id (`workflow.id` in the authoring wrapper, else `id`).
    pub id: String,
    /// Every agent id the workflow names: node `provider`s and `modelBindings` `agentId`s.
    pub agent_ids: BTreeSet<String>,
    /// For each agent id, the model ids bound to it (node `model` of the nodes it runs, and `modelId`s of its bindings).
    pub models: BTreeMap<String, BTreeSet<String>>,
    /// Every profile id a node names.
    pub profiles: BTreeSet<String>,
    /// Whether any node has `"manual_check": true`.
    pub has_manual_check: bool,
    /// For each agent id, what the stub standing in for it answers with.
    pub roles: BTreeMap<String, Role>,
    /// The node the workflow starts at, empty when it names none.
    pub entry: String,
    /// The workflow's nodes, in the order the workflow lists them.
    pub nodes: Vec<GraphNode>,
    /// The workflow's edges, in the order the workflow lists them.
    pub edges: Vec<GraphEdge>,
}

/// One node of the workflow graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphNode {
    /// The node id.
    pub id: String,
    /// The agent id the node runs, when it names one.
    pub provider: Option<String>,
    /// Whether the node is a manual check, which pauses the run when reached.
    pub manual_check: bool,
}

/// One edge of the workflow graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    /// The node the edge leaves.
    pub from: String,
    /// The node the edge reaches, or `$end`.
    pub to: String,
    /// The outcome the edge follows: `success` or `failure`.
    pub on: String,
}

/// What walking the workflow graph under one verdict predicts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Walk {
    /// The exit code `gold-band run` must return.
    pub exit: i32,
    /// The first node that failed, when one did.
    pub failed: Option<String>,
    /// Every node the walk reached, in order.
    pub visited: Vec<String>,
}

/// One run of the workflow: the stub's verdict and the exit code expected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioSpec {
    /// The scenario name, such as `clean`.
    pub name: String,
    /// The stub verdict: `clean` (or empty), `needs-human`, `open-questions`,
    /// `fail` or `gate-fail=<name>`.
    pub verdict: String,
    /// The exit code `gold-band run` must return.
    pub expected_exit: i32,
    /// The node the scenario starts from, when it proves the stretch after a
    /// manual check; `None` runs the whole workflow.
    pub entry: Option<String>,
}

/// Everything `prove` needs.
#[derive(Clone, Debug)]
pub struct ProveOptions {
    /// The workflow file.
    pub workflow: PathBuf,
    /// The gold-band binary, or `None` when none was supplied (the proof is skipped).
    pub gold_band: Option<PathBuf>,
    /// The Gold Band commit the binary was built from, when known.
    pub gold_band_commit: Option<String>,
    /// The executable that answers `stub-acp` (the harness itself).
    pub stub_exe: PathBuf,
    /// A directory the proof may create its temporary homes and workspaces in.
    pub scratch: PathBuf,
    /// Scenarios to run in addition to the defaults.
    pub extra_scenarios: Vec<ScenarioSpec>,
    /// Seconds one `gold-band run` may take before it is killed.
    pub timeout_secs: u64,
    /// The timestamp to record, RFC 3339 UTC.
    pub recorded_at: String,
}

/// The message printed when no Gold Band binary is supplied.
pub const SKIP_NO_BINARY: &str =
    "no Gold Band binary supplied (set GOLD_BAND_BIN or pass --gold-band); nothing was proved";

/// The requirement document the proof's workspace holds.
const REQUIREMENT: &str =
    "# Requirement\n\nThe headless proof runs this requirement through the workflow.\n";

/// How long the harness waits between two looks at a running child.
const POLL_MILLIS: u64 = 50;

/// How many characters of stderr a failing scenario's note keeps.
const STDERR_TAIL: usize = 300;

/// The edge target that ends a run.
const END: &str = "$end";

/// How many nodes one walk may reach before it is treated as a cycle.
const MAX_STEPS: usize = 1000;

/// The name of the pruned workflow a segment scenario runs.
pub const SEGMENT_FILE: &str = "segment.json";

/// Reads the facts about a workflow from its JSON text.
pub fn inspect(workflow_json: &str) -> Result<WorkflowInfo, String> {
    let value: Value = serde_json::from_str(workflow_json)
        .map_err(|error| format!("the workflow is not valid JSON: {}", error))?;
    let definition = match value.get("workflow") {
        Some(wrapper) if wrapper.is_object() => wrapper,
        _ => &value,
    };
    let id = definition
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "the workflow has no id".to_string())?
        .to_string();
    let entry = definition
        .get("entry")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut agent_ids = BTreeSet::new();
    let mut models: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut profiles = BTreeSet::new();
    let mut has_manual_check = false;
    let mut answers = BTreeSet::new();
    let mut questions = BTreeSet::new();
    let mut nodes = Vec::new();
    if let Some(Value::Array(entries)) = definition.get("nodes") {
        for node in entries {
            let Some(object) = node.as_object() else {
                continue;
            };
            if let Some(node_id) = object.get("id").and_then(Value::as_str) {
                nodes.push(GraphNode {
                    id: node_id.to_string(),
                    provider: object
                        .get("provider")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    manual_check: object.get("manual_check").and_then(Value::as_bool) == Some(true),
                });
            }
            if let Some(provider) = object.get("provider").and_then(Value::as_str) {
                agent_ids.insert(provider.to_string());
                let bound = models.entry(provider.to_string()).or_default();
                bound.extend(
                    object
                        .get("model")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                );
                if asks_questions(object) {
                    questions.insert(provider.to_string());
                }
                if answers_with_ok(object) {
                    answers.insert(provider.to_string());
                }
            }
            if let Some(profile) = object.get("profile").and_then(Value::as_str) {
                profiles.insert(profile.to_string());
            }
            if object.get("manual_check").and_then(Value::as_bool) == Some(true) {
                has_manual_check = true;
            }
        }
    }
    let mut edges = Vec::new();
    if let Some(Value::Array(entries)) = definition.get("edges") {
        for edge in entries {
            let Some(object) = edge.as_object() else {
                continue;
            };
            let (Some(from), Some(to), Some(on)) = (
                object.get("from").and_then(Value::as_str),
                object.get("to").and_then(Value::as_str),
                object.get("on").and_then(Value::as_str),
            ) else {
                continue;
            };
            edges.push(GraphEdge {
                from: from.to_string(),
                to: to.to_string(),
                on: on.to_string(),
            });
        }
    }
    if let Some(bindings) = value
        .get("modelBindings")
        .and_then(|wrapper| wrapper.get("bindings"))
        .and_then(Value::as_array)
    {
        for binding in bindings {
            let Some(object) = binding.as_object() else {
                continue;
            };
            if let Some(agent_id) = object.get("agentId").and_then(Value::as_str) {
                agent_ids.insert(agent_id.to_string());
                let bound = models.entry(agent_id.to_string()).or_default();
                bound.extend(
                    object
                        .get("modelId")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                );
            }
        }
    }
    let roles = agent_ids
        .iter()
        .map(|agent_id| (agent_id.clone(), role_for(agent_id, &answers, &questions)))
        .collect();
    Ok(WorkflowInfo {
        id,
        agent_ids,
        models,
        profiles,
        has_manual_check,
        roles,
        entry,
        nodes,
        edges,
    })
}

/// Whether a node's `output.schema` is a JSON object without a `findings` key.
fn answers_with_ok(node: &Map<String, Value>) -> bool {
    match node.get("output").and_then(|output| output.get("schema")) {
        Some(Value::Object(schema)) => !schema.contains_key("findings"),
        _ => false,
    }
}

/// Whether a node's `output.schema` is a JSON object holding an
/// `open_questions` key.
fn asks_questions(node: &Map<String, Value>) -> bool {
    match node.get("output").and_then(|output| output.get("schema")) {
        Some(Value::Object(schema)) => schema.contains_key("open_questions"),
        _ => false,
    }
}

/// The role an agent id plays: a `gate-` or `copy-` prefix names a shim, else
/// the schema of the nodes it runs decides between open questions, an answer
/// and findings.
fn role_for(agent_id: &str, answers: &BTreeSet<String>, questions: &BTreeSet<String>) -> Role {
    if let Some(name) = agent_id
        .strip_prefix("gate-")
        .filter(|name| !name.is_empty())
    {
        return Role::Gate(name.to_string());
    }
    if let Some(name) = agent_id
        .strip_prefix("copy-")
        .filter(|name| !name.is_empty())
    {
        return Role::Copy(name.to_string());
    }
    if questions.contains(agent_id) {
        return Role::Questions;
    }
    if answers.contains(agent_id) {
        Role::Answer
    } else {
        Role::Findings
    }
}

/// Walks the workflow graph from `entry` under `verdict` and predicts what
/// `gold-band run` does: a manual check pauses the run (exit 2), a node with no
/// edge for its outcome ends it, and `$end` ends it too.
pub fn walk(info: &WorkflowInfo, entry: &str, verdict: &str) -> Walk {
    let mut visited = Vec::new();
    let mut failed: Option<String> = None;
    let mut current = entry.to_string();
    let mut steps = 0usize;
    loop {
        steps += 1;
        if steps > MAX_STEPS {
            return Walk {
                exit: 1,
                failed,
                visited,
            };
        }
        let Some(node) = info.nodes.iter().find(|node| node.id == current) else {
            return Walk {
                exit: 1,
                failed,
                visited,
            };
        };
        visited.push(node.id.clone());
        if node.manual_check {
            return Walk {
                exit: 2,
                failed,
                visited,
            };
        }
        let outcome = if node_fails(info, node, verdict) {
            if failed.is_none() {
                failed = Some(node.id.clone());
            }
            "failure"
        } else {
            "success"
        };
        let exit = if outcome == "failure" { 1 } else { 0 };
        let Some(edge) = info
            .edges
            .iter()
            .find(|edge| edge.from == node.id && edge.on == outcome)
        else {
            return Walk {
                exit,
                failed,
                visited,
            };
        };
        if edge.to == END {
            return Walk {
                exit,
                failed,
                visited,
            };
        }
        current = edge.to.clone();
    }
}

/// The role of the agent a node runs; a node that names no agent, or an agent
/// the workflow does not declare, answers as a critic.
fn node_role(info: &WorkflowInfo, node: &GraphNode) -> Role {
    node.provider
        .as_ref()
        .and_then(|provider| info.roles.get(provider))
        .cloned()
        .unwrap_or(Role::Findings)
}

/// Whether a node's outcome under `verdict` is a failure. A manual check never
/// gets this far: [`walk`] stops at it, because a paused run cannot be
/// continued headlessly.
fn node_fails(info: &WorkflowInfo, node: &GraphNode, verdict: &str) -> bool {
    let verdict = verdict.trim();
    match node_role(info, node) {
        Role::Gate(name) | Role::Copy(name) => {
            verdict.strip_prefix("gate-fail=") == Some(name.as_str())
        }
        // A critic or questions agent also runs nodes with no success
        // condition (a drafting node), so only a node that routes failures
        // (a merge, a grill round) is taken to fail.
        Role::Findings => verdict == NEEDS_HUMAN && routes_failure(info, node),
        Role::Questions => verdict == OPEN_QUESTIONS && routes_failure(info, node),
        Role::Answer => false,
    }
}

/// Whether the node has an edge for its failure.
fn routes_failure(info: &WorkflowInfo, node: &GraphNode) -> bool {
    info.edges
        .iter()
        .any(|edge| edge.from == node.id && edge.on == "failure")
}

/// One stretch of the workflow the proof can reach: the whole workflow, or the
/// part after a manual check.
struct Segment {
    /// The prefix every scenario name of the segment carries, empty for the
    /// whole workflow.
    prefix: String,
    /// The node the segment starts at, `None` for the workflow's own entry.
    entry: Option<String>,
}

/// The scenarios every workflow gets, segment by segment: `clean` always, then
/// `needs-human`, `open-questions` and one `gate-fail-<name>` per gate, each
/// kept only when walking the segment under that verdict fails a node. The
/// expected exit code of each comes from the walk.
pub fn default_scenarios(info: &WorkflowInfo) -> Vec<ScenarioSpec> {
    let mut scenarios = Vec::new();
    let mut proved = BTreeSet::new();
    for segment in segments(info) {
        scenarios.extend(segment_scenarios(info, &segment, &mut proved));
    }
    scenarios
}

/// The segments the proof walks: the whole workflow from its entry, then the
/// part after each manual check that leads on to another node.
fn segments(info: &WorkflowInfo) -> Vec<Segment> {
    let mut segments = vec![Segment {
        prefix: String::new(),
        entry: None,
    }];
    for node in &info.nodes {
        if !node.manual_check {
            continue;
        }
        let Some(target) = info
            .edges
            .iter()
            .find(|edge| edge.from == node.id && edge.on == "success")
            .map(|edge| edge.to.clone())
        else {
            continue;
        };
        if target == END {
            continue;
        }
        segments.push(Segment {
            prefix: format!("after-{}.", node.id),
            entry: Some(target),
        });
    }
    segments
}

/// The scenarios one segment gets, in order: `clean`, then the candidate
/// verdicts that fail a node of the segment, except a node `proved` already
/// holds a failing scenario for (`proved` collects the nodes as it goes).
fn segment_scenarios(
    info: &WorkflowInfo,
    segment: &Segment,
    proved: &mut BTreeSet<String>,
) -> Vec<ScenarioSpec> {
    let entry = segment.entry.clone().unwrap_or_else(|| info.entry.clone());
    let clean = walk(info, &entry, "clean");
    let mut scenarios = vec![ScenarioSpec {
        name: format!("{}clean", segment.prefix),
        verdict: "clean".to_string(),
        expected_exit: clean.exit,
        entry: segment.entry.clone(),
    }];
    for verdict in candidate_verdicts(info) {
        let walked = walk(info, &entry, &verdict);
        // A failure of a node an earlier segment already proved is not run again.
        let Some(failed) = walked.failed.clone() else {
            continue;
        };
        if !proved.insert(failed) {
            continue;
        }
        scenarios.push(ScenarioSpec {
            name: format!("{}{}", segment.prefix, verdict_name(&verdict)),
            verdict,
            expected_exit: walked.exit,
            entry: segment.entry.clone(),
        });
    }
    scenarios
}

/// The verdicts a segment is proved under besides `clean`: `needs-human`,
/// `open-questions`, then one failing gate per gate name, sorted.
fn candidate_verdicts(info: &WorkflowInfo) -> Vec<String> {
    let mut verdicts = vec![NEEDS_HUMAN.to_string(), OPEN_QUESTIONS.to_string()];
    for name in gate_names(info) {
        verdicts.push(format!("gate-fail={}", name));
    }
    verdicts
}

/// The scenario name a verdict gets: `gate-fail=NAME` becomes `gate-fail-NAME`.
fn verdict_name(verdict: &str) -> String {
    verdict.replace('=', "-")
}

/// The gate names the workflow's roles name, sorted.
fn gate_names(info: &WorkflowInfo) -> Vec<String> {
    let mut names: Vec<String> = info
        .roles
        .values()
        .filter_map(|role| match role {
            Role::Gate(name) => Some(name.clone()),
            _ => None,
        })
        .collect();
    names.sort();
    names
}

/// Parses `NAME:VERDICT:EXIT` (verdict one of `clean`, `needs-human`,
/// `open-questions`, `fail` or `gate-fail=NAME`; exit an integer). The scenario
/// runs the whole workflow, never a segment.
pub fn parse_scenario(text: &str) -> Result<ScenarioSpec, String> {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != 3 {
        return Err(format!(
            "the scenario `{}` is not NAME:VERDICT:EXIT (three colon separated parts)",
            text
        ));
    }
    let name = parts[0];
    if name.is_empty() {
        return Err(format!("the scenario `{}` has an empty name", text));
    }
    if !is_name(name) {
        return Err(format!(
            "the scenario name `{}` may only hold letters, digits, `.`, `_` and `-`",
            name
        ));
    }
    let verdict = parts[1];
    if !is_known_verdict(verdict) {
        return Err(format!(
            "the scenario `{}` has the unknown verdict `{}` (use clean, needs-human, open-questions, fail or gate-fail=NAME)",
            name, verdict
        ));
    }
    let exit_text = parts[2];
    let expected_exit = exit_text.parse::<i32>().map_err(|_| {
        format!(
            "the scenario `{}` has the exit code `{}`, which is not an integer",
            name, exit_text
        )
    })?;
    Ok(ScenarioSpec {
        name: name.to_string(),
        verdict: verdict.to_string(),
        expected_exit,
        entry: None,
    })
}

/// Whether a verdict is one the stub understands: `clean`, `needs-human`,
/// `open-questions`, `fail`, or `gate-fail=` and a non-empty gate name.
fn is_known_verdict(verdict: &str) -> bool {
    if matches!(verdict, "clean" | NEEDS_HUMAN | OPEN_QUESTIONS | "fail") {
        return true;
    }
    match verdict.strip_prefix("gate-fail=") {
        Some(name) => is_name(name),
        None => false,
    }
}

/// Whether `c` may appear in a scenario name or a gate name.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'
}

/// Whether `text` is a non-empty name of letters, digits, `.`, `_` and `-`.
fn is_name(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_name_char)
}

/// A copy of the workflow that starts at `entry`: the nodes reachable from it,
/// the edges that leave them, the bindings of those nodes, and `entry` set to
/// `entry`. Gold Band refuses a node unreachable from the entry, so the stretch
/// after a manual check needs its own copy to be proved at all.
pub fn prune(workflow_json: &str, entry: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(workflow_json)
        .map_err(|error| format!("the workflow is not valid JSON: {}", error))?;
    let wrapper = value.as_object().cloned().unwrap_or_default();
    let wrapped = matches!(wrapper.get("workflow"), Some(Value::Object(_)));
    let definition = if wrapped {
        wrapper
            .get("workflow")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    } else {
        wrapper.clone()
    };
    let nodes = definition
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let edges = definition
        .get("edges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let ids: Vec<String> = nodes.iter().filter_map(node_id).collect();
    if !ids.iter().any(|id| id == entry) {
        return Err(format!(
            "the entry `{}` is not a node of the workflow",
            entry
        ));
    }
    let reachable = reachable_from(&ids, &edges, entry);
    let kept: Vec<Value> = nodes
        .into_iter()
        .filter(|node| node_id(node).is_some_and(|id| reachable.contains(&id)))
        .collect();
    let slots: BTreeSet<String> = kept
        .iter()
        .filter_map(|node| {
            node.get("executionSlotId")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let kept_edges: Vec<Value> = edges
        .into_iter()
        .filter(|edge| {
            edge.get("from")
                .and_then(Value::as_str)
                .is_some_and(|from| reachable.contains(from))
        })
        .collect();
    let mut pruned_definition = definition;
    pruned_definition.insert("nodes".to_string(), Value::Array(kept));
    pruned_definition.insert("edges".to_string(), Value::Array(kept_edges));
    pruned_definition.insert("entry".to_string(), Value::String(entry.to_string()));
    let mut pruned = if wrapped {
        let mut object = wrapper;
        object.insert("workflow".to_string(), Value::Object(pruned_definition));
        Value::Object(object)
    } else {
        Value::Object(pruned_definition)
    };
    if wrapped {
        if let Some(bindings) = pruned
            .get_mut("modelBindings")
            .and_then(|wrapper| wrapper.get_mut("bindings"))
            .and_then(Value::as_array_mut)
        {
            bindings.retain(|binding| {
                binding
                    .get("executionSlotId")
                    .and_then(Value::as_str)
                    .is_some_and(|slot| slots.contains(slot))
            });
        }
    }
    // A document built from parsed JSON always serialises.
    Ok(serde_json::to_string_pretty(&pruned).unwrap_or_default())
}

/// The id of a node object, when it has one.
fn node_id(node: &Value) -> Option<String> {
    node.get("id").and_then(Value::as_str).map(str::to_string)
}

/// The node ids reachable from `entry` by following every edge's `to`, with
/// `$end` and ids the workflow does not declare ignored.
fn reachable_from(ids: &[String], edges: &[Value], entry: &str) -> BTreeSet<String> {
    let mut reachable = BTreeSet::new();
    let mut pending = vec![entry.to_string()];
    while let Some(id) = pending.pop() {
        if id == END || !ids.contains(&id) {
            continue;
        }
        if !reachable.insert(id.clone()) {
            continue;
        }
        for edge in edges {
            if edge.get("from").and_then(Value::as_str) != Some(id.as_str()) {
                continue;
            }
            if let Some(to) = edge.get("to").and_then(Value::as_str) {
                pending.push(to.to_string());
            }
        }
    }
    reachable
}

/// Gold Band's exit code for a printed run state: completed and success is 0, completed otherwise 1, paused 2, running 4.
pub fn exit_for_state(status: &str, outcome: Option<&str>) -> Option<i32> {
    match status {
        "completed" => Some(if outcome == Some("success") { 0 } else { 1 }),
        "paused" => Some(2),
        "running" => Some(4),
        _ => None,
    }
}

/// Finds the run state `gold-band run` printed (a JSON object that starts at a line holding only `{`) and returns its `status` and `outcome`.
pub fn parse_run_state(output: &str) -> Option<(String, Option<String>)> {
    if let Some(state) = state_from_json(output) {
        return Some(state);
    }
    let mut start = 0usize;
    while start <= output.len() {
        let end = match output[start..].find('\n') {
            Some(index) => start + index,
            None => output.len(),
        };
        if &output[start..end] == "{" {
            return state_from_json(&output[start..]);
        }
        if end == output.len() {
            break;
        }
        start = end + 1;
    }
    None
}

/// The `status` and `outcome` of a JSON object, when the text is one and holds a status.
fn state_from_json(text: &str) -> Option<(String, Option<String>)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;
    let status = object.get("status")?.as_str()?.to_string();
    let outcome = object
        .get("outcome")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((status, outcome))
}

/// Builds `root/home` (a Gold Band home) and `root/workspace` for one scenario and returns `(home, workspace)`.
pub fn prepare(
    root: &Path,
    info: &WorkflowInfo,
    stub_exe: &Path,
    verdict: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let home = root.join("home");
    let workspace = root.join("workspace");
    let gold_band_dir = home.join(".gold-band");
    let profiles_dir = gold_band_dir.join("context").join("profiles");
    fs::create_dir_all(&profiles_dir).map_err(|error| {
        format!(
            "cannot create the Gold Band home {}: {}",
            gold_band_dir.display(),
            error
        )
    })?;
    let settings_path = gold_band_dir.join("settings.json");
    let reports = root.join("reports");
    let settings = settings_json(info, stub_exe, verdict, &reports);
    fs::write(&settings_path, settings).map_err(|error| {
        format!(
            "cannot write the Gold Band settings {}: {}",
            settings_path.display(),
            error
        )
    })?;
    for profile in &info.profiles {
        let path = profiles_dir.join(format!("{}.md", profile));
        fs::write(&path, profile_text(profile))
            .map_err(|error| format!("cannot write the profile {}: {}", path.display(), error))?;
    }
    fs::create_dir_all(&reports).map_err(|error| {
        format!(
            "cannot create the report directory {}: {}",
            reports.display(),
            error
        )
    })?;
    fs::create_dir_all(&workspace).map_err(|error| {
        format!(
            "cannot create the workspace {}: {}",
            workspace.display(),
            error
        )
    })?;
    let requirement = workspace.join("requirement.md");
    fs::write(&requirement, REQUIREMENT).map_err(|error| {
        format!(
            "cannot write the requirement {}: {}",
            requirement.display(),
            error
        )
    })?;
    git_init(&workspace, &home)?;
    Ok((home, workspace))
}

/// Runs one scenario: prepares a fresh home under `root`, runs `gold-band run`, and reports what happened.
pub fn run_scenario(
    gold_band: &Path,
    workflow: &Path,
    root: &Path,
    info: &WorkflowInfo,
    stub_exe: &Path,
    spec: &ScenarioSpec,
    timeout_secs: u64,
) -> ScenarioResult {
    let mut result = ScenarioResult {
        name: spec.name.clone(),
        expected_exit: spec.expected_exit,
        exit_code: None,
        run_status: None,
        run_outcome: None,
        note: None,
    };
    let (home, workspace) = match prepare(root, info, stub_exe, &spec.verdict) {
        Ok(paths) => paths,
        Err(message) => {
            result.note = Some(message);
            return result;
        }
    };
    let stdout_path = root.join("stdout.txt");
    let stderr_path = root.join("stderr.txt");
    let mut child = match spawn_run(
        gold_band,
        workflow,
        &workspace,
        &home,
        &stdout_path,
        &stderr_path,
    ) {
        Ok(child) => child,
        Err(message) => {
            result.note = Some(message);
            return result;
        }
    };
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().ok().flatten() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break None;
        }
        sleep(Duration::from_millis(POLL_MILLIS));
    };
    let mut note = None;
    let exit_code = if timed_out {
        note = Some(format!("timed out after {} seconds", timeout_secs));
        None
    } else {
        match status.and_then(|status| status.code()) {
            Some(code) => Some(code),
            None => {
                note = Some("killed by a signal".to_string());
                None
            }
        }
    };
    let output = fs::read_to_string(&stdout_path).unwrap_or_default();
    if let Some((run_status, run_outcome)) = parse_run_state(&output) {
        result.run_status = Some(run_status);
        result.run_outcome = run_outcome;
    }
    if note.is_none() {
        note = disagreement_note(
            exit_code,
            spec.expected_exit,
            &stderr_path,
            result.run_status.as_deref(),
            result.run_outcome.as_deref(),
        );
    }
    result.exit_code = exit_code;
    result.note = note;
    result
}

/// Runs the whole proof and returns the record (not yet written).
pub fn prove(options: &ProveOptions) -> Result<Record, String> {
    let text = fs::read_to_string(&options.workflow).map_err(|error| {
        format!(
            "cannot read the workflow {}: {}",
            options.workflow.display(),
            error
        )
    })?;
    let workflow = refuse::load_workflow(&options.workflow)?;
    let info = inspect(&text)?;
    let mut record = Record {
        workflow_id: workflow.id,
        workflow_sha256: workflow.sha256,
        gold_band_commit: options.gold_band_commit.clone(),
        outcome: Outcome::Skipped,
        scenarios: Vec::new(),
        skip_reason: Some(SKIP_NO_BINARY.to_string()),
        recorded_at: options.recorded_at.clone(),
    };
    let Some(gold_band) = options.gold_band.as_deref() else {
        return Ok(record);
    };
    // Gold Band runs inside the scenario's workspace, so the workflow path must not depend on the caller's directory.
    let workflow_path = fs::canonicalize(&options.workflow).unwrap_or(options.workflow.clone());
    let scenarios = scenario_list(&info, &options.extra_scenarios);
    let mut results = Vec::with_capacity(scenarios.len());
    for (index, spec) in scenarios.iter().enumerate() {
        let root = options.scratch.join(format!("{}-{}", index, spec.name));
        let prepared = scenario_workflow(&text, &info, &workflow_path, spec, &root);
        let (path, scenario_info) = match prepared {
            Ok(pair) => pair,
            Err(message) => {
                results.push(ScenarioResult {
                    name: spec.name.clone(),
                    expected_exit: spec.expected_exit,
                    exit_code: None,
                    run_status: None,
                    run_outcome: None,
                    note: Some(message),
                });
                continue;
            }
        };
        results.push(run_scenario(
            gold_band,
            &path,
            &root,
            &scenario_info,
            &options.stub_exe,
            spec,
            options.timeout_secs,
        ));
    }
    let passed = results
        .iter()
        .all(|result| result.passed() && result.note.is_none());
    record.outcome = if passed {
        Outcome::Passed
    } else {
        Outcome::Failed
    };
    record.scenarios = results;
    record.skip_reason = None;
    Ok(record)
}

/// The default scenarios with the extra ones folded in: an extra replaces a default of the same name, others append.
fn scenario_list(info: &WorkflowInfo, extra: &[ScenarioSpec]) -> Vec<ScenarioSpec> {
    let mut scenarios = default_scenarios(info);
    for spec in extra {
        match scenarios
            .iter_mut()
            .find(|existing| existing.name == spec.name)
        {
            Some(existing) => *existing = spec.clone(),
            None => scenarios.push(spec.clone()),
        }
    }
    scenarios
}

/// The workflow file one scenario runs and the facts the stub needs for it: the
/// whole workflow, or a pruned copy under the scenario's root that starts at the
/// scenario's entry.
pub fn scenario_workflow(
    text: &str,
    info: &WorkflowInfo,
    whole: &Path,
    spec: &ScenarioSpec,
    root: &Path,
) -> Result<(PathBuf, WorkflowInfo), String> {
    let Some(entry) = spec.entry.as_deref() else {
        return Ok((whole.to_path_buf(), info.clone()));
    };
    let pruned = prune(text, entry)?;
    fs::create_dir_all(root).map_err(|error| {
        format!(
            "cannot create the scenario directory {}: {}",
            root.display(),
            error
        )
    })?;
    let path = root.join(SEGMENT_FILE);
    fs::write(&path, &pruned).map_err(|error| {
        format!(
            "cannot write the segment workflow {}: {}",
            path.display(),
            error
        )
    })?;
    let pruned_info = inspect(&pruned)?;
    Ok((path, pruned_info))
}

/// The settings document that makes every agent id of the workflow the stub,
/// answering as its role and keeping its gate reports under `report_dir`.
fn settings_json(info: &WorkflowInfo, stub_exe: &Path, verdict: &str, report_dir: &Path) -> String {
    let command = stub_exe.to_string_lossy().into_owned();
    let report_dir = report_dir.to_string_lossy().into_owned();
    let mut agents = Map::new();
    for agent_id in &info.agent_ids {
        let models = info
            .models
            .get(agent_id)
            .map(|set| set.iter().cloned().collect::<Vec<String>>().join(","))
            .unwrap_or_default();
        let role = info.roles.get(agent_id).cloned().unwrap_or(Role::Findings);
        agents.insert(
            agent_id.clone(),
            json!({
                "adapter": {
                    "command": &command,
                    "args": ["stub-acp"],
                    "displayName": format!("Stub ACP ({})", agent_id),
                    "env": {
                        "STUB_ACP_VERDICT": verdict,
                        "STUB_ACP_MODELS": models,
                        "STUB_ACP_ROLE": role.as_text(),
                        "STUB_ACP_REPORT_DIR": &report_dir,
                    },
                },
            }),
        );
    }
    json!({
        "settingsSchemaVersion": 12,
        "agents": Value::Object(agents),
    })
    .to_string()
}

/// The profile file Gold Band needs for a profile id a node names.
fn profile_text(id: &str) -> String {
    format!(
        "---\nid: {}\nname: Stub profile {}\nsummary: Stub profile for the headless proof.\ncreatedAt: 2026-11-03T00:00:00Z\nupdatedAt: 2026-11-03T00:00:00Z\ndynamicTemplate: false\n---\n\nStub profile for the headless proof.\n",
        id, id
    )
}

/// Makes `workspace` a git repository with one empty commit, isolated from the user's git configuration.
fn git_init(workspace: &Path, home: &Path) -> Result<(), String> {
    let git = Path::new("git");
    git_step(git, workspace, home, "init", &["init", "-q"])?;
    git_step(
        git,
        workspace,
        home,
        "commit",
        &[
            "-c",
            "user.name=proof",
            "-c",
            "user.email=proof@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "proof",
        ],
    )
}

/// Runs one git step in `workspace` with no user or system configuration, naming the step when it fails.
pub(crate) fn git_step(
    program: &Path,
    workspace: &Path,
    home: &Path,
    step: &str,
    args: &[&str],
) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(workspace)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", home)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            format!(
                "cannot run git {} in {}: {}",
                step,
                workspace.display(),
                error
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "git {} failed in {}: {}",
        step,
        workspace.display(),
        stderr_text(&output.stderr)
    ))
}

/// The trimmed text of a command's stderr.
fn stderr_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

/// Starts `gold-band run` with its output redirected to files, so no pipe can fill and block.
fn spawn_run(
    gold_band: &Path,
    workflow: &Path,
    workspace: &Path,
    home: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<Child, String> {
    let stdout = fs::File::create(stdout_path)
        .map_err(|error| format!("cannot create {}: {}", stdout_path.display(), error))?;
    let stderr = fs::File::create(stderr_path)
        .map_err(|error| format!("cannot create {}: {}", stderr_path.display(), error))?;
    Command::new(gold_band)
        .arg("run")
        .arg("--workflow")
        .arg(workflow)
        .arg("--requirement-file")
        .arg(workspace.join("requirement.md"))
        .current_dir(workspace)
        .env("GOLD_BAND_HOME", home)
        .env("HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|error| format!("cannot run {}: {}", gold_band.display(), error))
}

/// Why a finished run does not match its scenario, when it does not.
fn disagreement_note(
    exit_code: Option<i32>,
    expected_exit: i32,
    stderr_path: &Path,
    run_status: Option<&str>,
    run_outcome: Option<&str>,
) -> Option<String> {
    let code = exit_code?;
    if code != expected_exit {
        let mut note = format!("expected exit {}, got {}", expected_exit, code);
        let stderr = fs::read_to_string(stderr_path).unwrap_or_default();
        let tail = tail_text(&stderr, STDERR_TAIL);
        if !tail.is_empty() {
            note.push_str(": ");
            note.push_str(&tail);
        }
        return Some(note);
    }
    let state_exit = run_status.and_then(|status| exit_for_state(status, run_outcome));
    if state_exit != Some(code) {
        return Some(format!(
            "exit {} is not backed by a printed run state ({}/{})",
            code,
            run_status.unwrap_or("none"),
            run_outcome.unwrap_or("none")
        ));
    }
    None
}

/// The last `limit` characters of `text`, trimmed.
fn tail_text(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if count <= limit {
        return trimmed.to_string();
    }
    trimmed.chars().skip(count - limit).collect()
}
