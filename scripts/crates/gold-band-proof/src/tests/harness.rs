use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};

use crate::harness::{
    default_scenarios, exit_for_state, git_step, inspect, parse_run_state, parse_scenario, prepare,
    prove, prune, run_scenario, scenario_workflow, walk, GraphEdge, GraphNode, ProveOptions,
    ScenarioSpec, Walk, WorkflowInfo, SEGMENT_FILE, SKIP_NO_BINARY,
};
use crate::record::Outcome;
use crate::stub::Role;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let tick = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "gold-band-proof-harness-{}-{}-{}",
        std::process::id(),
        tick,
        label
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A graph node for the tests.
fn node(id: &str, provider: Option<&str>, manual_check: bool) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        provider: provider.map(str::to_string),
        manual_check,
    }
}

/// A graph edge for the tests.
fn edge(from: &str, to: &str, on: &str) -> GraphEdge {
    GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
        on: on.to_string(),
    }
}

/// A workflow graph for the walk and scenario tests: the nodes, the edges and
/// the role of each agent id, starting at the first node.
fn graph(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>, roles: &[(&str, Role)]) -> WorkflowInfo {
    let mut info = info();
    info.entry = nodes
        .first()
        .map(|node| node.id.clone())
        .unwrap_or_default();
    info.has_manual_check = nodes.iter().any(|node| node.manual_check);
    info.nodes = nodes;
    info.edges = edges;
    info.roles = roles
        .iter()
        .map(|(agent_id, role)| (agent_id.to_string(), role.clone()))
        .collect();
    info
}

fn info() -> WorkflowInfo {
    let mut agent_ids = BTreeSet::new();
    agent_ids.insert("vibe".to_string());
    agent_ids.insert("agy".to_string());
    let mut models: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    models.insert("vibe".to_string(), BTreeSet::from(["m1".to_string()]));
    let mut profiles = BTreeSet::new();
    profiles.insert("pf".to_string());
    WorkflowInfo {
        id: "proof-test".to_string(),
        agent_ids,
        models,
        profiles,
        has_manual_check: false,
        roles: BTreeMap::new(),
        entry: "only".to_string(),
        nodes: vec![node("only", Some("vibe"), false)],
        edges: vec![edge("only", "$end", "success")],
    }
}

fn spec(name: &str, verdict: &str, expected_exit: i32) -> ScenarioSpec {
    ScenarioSpec {
        name: name.to_string(),
        verdict: verdict.to_string(),
        expected_exit,
        entry: None,
    }
}

/// A scenario that proves the stretch of the workflow after a manual check.
fn segment_spec(name: &str, verdict: &str, expected_exit: i32, entry: &str) -> ScenarioSpec {
    ScenarioSpec {
        name: name.to_string(),
        verdict: verdict.to_string(),
        expected_exit,
        entry: Some(entry.to_string()),
    }
}

fn fake(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("gold-band");
    fs::write(&path, body).expect("fake written");
    let mut permissions = fs::metadata(&path).expect("fake metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("fake executable");
    path
}

fn workflow_file(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("workflow.json");
    fs::write(&path, body).expect("workflow written");
    path
}

const CLEAN_FAKE: &str = r#"#!/bin/sh
printf '{"status":"completed","outcome":"success"}\n'
exit 0
"#;

const PAUSED_FAKE: &str = r#"#!/bin/sh
printf '{"status":"paused","outcome":null}\n'
exit 2
"#;

const WRONG_EXIT_FAKE: &str = r#"#!/bin/sh
printf '{"status":"completed","outcome":"success"}\n'
printf 'boom\n' >&2
exit 3
"#;

const WRONG_EXIT_SILENT_FAKE: &str = r#"#!/bin/sh
printf '{"status":"completed","outcome":"success"}\n'
exit 3
"#;

const DISAGREEING_FAKE: &str = r#"#!/bin/sh
printf '{"status":"completed","outcome":"failure"}\n'
exit 0
"#;

const DISAGREEING_NO_OUTCOME_FAKE: &str = r#"#!/bin/sh
printf '{"status":"completed"}\n'
exit 0
"#;

const UNKNOWN_STATE_FAKE: &str = r#"#!/bin/sh
printf '{"status":"weird"}\n'
exit 0
"#;

const NO_STATE_FAKE: &str = r#"#!/bin/sh
exit 0
"#;

const HANG_FAKE: &str = r#"#!/bin/sh
sleep 5
"#;

const SIGNAL_FAKE: &str = r#"#!/bin/sh
kill -9 $$
"#;

const FAILING_FAKE: &str = r#"#!/bin/sh
printf '{"status":"completed","outcome":"failure"}\n'
exit 1
"#;

const VERDICT_FAKE: &str = r#"#!/bin/sh
verdict=$(sed -n 's/.*"STUB_ACP_VERDICT": *"\([^"]*\)".*/\1/p' "$GOLD_BAND_HOME/.gold-band/settings.json")
case "$verdict" in
  needs-human) printf '{"status":"paused","outcome":null}\n'; exit 2 ;;
  fail) printf '{"status":"completed","outcome":"failure"}\n'; exit 1 ;;
  *) printf '{"status":"completed","outcome":"success"}\n'; exit 0 ;;
esac
"#;

/// A workflow whose only default scenario is `clean`: one answer node that
/// reaches `$end`, so no verdict but `clean` fails it.
const WORKFLOW: &str = r#"{"workflow":{"id":"proof-test","entry":"only","nodes":[
    {"id":"only","provider":"vibe","model":"m1","profile":"pf","output":{"kind":"json","schema":{"ok":true}}}
],"edges":[{"from":"only","to":"$end","on":"success"}]}}"#;

/// A workflow whose manual check sits off the clean path and leads on to
/// another node, so the proof runs the whole workflow and then the stretch
/// after the check, both expecting exit 0.
const SEGMENT_WORKFLOW: &str = r#"{"workflow":{"id":"proof-test","entry":"start","nodes":[
    {"id":"start","provider":"vibe","output":{"kind":"json","schema":{"ok":true}}},
    {"id":"check","manual_check":true},
    {"id":"after","provider":"vibe","output":{"kind":"json","schema":{"ok":true}}}
],"edges":[
    {"from":"start","to":"$end","on":"success"},
    {"from":"check","to":"after","on":"success"},
    {"from":"after","to":"$end","on":"success"}
]}}"#;

fn options(dir: &Path, gold_band: Option<PathBuf>) -> ProveOptions {
    ProveOptions {
        workflow: workflow_file(dir, WORKFLOW),
        gold_band,
        gold_band_commit: Some("abc1234".to_string()),
        stub_exe: dir.join("gold-band-proof"),
        scratch: dir.join("scratch"),
        extra_scenarios: Vec::new(),
        timeout_secs: 10,
        recorded_at: "2026-11-11T16:53:26Z".to_string(),
    }
}

#[test]
fn inspect_reads_the_spec_stage_template() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../installer/gold-band/workflows/critique-spec-stage.json");
    let text = fs::read_to_string(&path).expect("template read");
    let info = inspect(&text).expect("template inspected");
    assert_eq!(info.id, "critique-spec-stage");
    let agents: Vec<String> = info.agent_ids.iter().cloned().collect();
    assert_eq!(agents, vec!["agy".to_string(), "vibe".to_string()]);
    let vibe: Vec<String> = info.models["vibe"].iter().cloned().collect();
    assert_eq!(vibe, vec!["mistral-large-4-0".to_string()]);
    let agy: Vec<String> = info.models["agy"].iter().cloned().collect();
    assert_eq!(agy, vec!["gemini-3.8-flash-high".to_string()]);
    let profiles: Vec<String> = info.profiles.iter().cloned().collect();
    assert_eq!(profiles, vec!["pf-spec-redteam".to_string()]);
    assert!(info.has_manual_check);
}

/// The checked-in Workflow B, whose nodes are model nodes and gate shims.
fn workflow_b() -> WorkflowInfo {
    inspect(include_str!(
        "../../../../../installer/gold-band/workflows/workflow-b.json"
    ))
    .expect("workflow-b inspected")
}

#[test]
fn inspect_gives_every_agent_of_workflow_b_its_role() {
    let info = workflow_b();
    assert_eq!(info.id, "workflow-b");
    // Every gate shim: the main path, the findings check of each critic round
    // and the four gates again after each repair.
    let mut gates: Vec<String> = ["build", "test", "lint", "coverage"]
        .iter()
        .map(|gate| gate.to_string())
        .collect();
    for round in 1..=3 {
        gates.push(format!("findings-{}", round));
    }
    for repair in 1..=2 {
        for gate in ["build", "test", "lint", "coverage"] {
            gates.push(format!("{}-r{}", gate, repair));
        }
    }
    let mut expected: Vec<(String, Role)> = Vec::new();
    for gate in &gates {
        expected.push((format!("gate-{}", gate), Role::Gate(gate.clone())));
        expected.push((format!("copy-{}", gate), Role::Copy(gate.clone())));
    }
    for round in 1..=3 {
        expected.push((format!("verdict-{}", round), Role::Verdict(round)));
    }
    // The two critics' agents also serve the plain nodes (the test author, the
    // escalation judge and the manual check), which they answer as an answer.
    let critics = vec!["vibe".to_string(), "claude".to_string()];
    expected.push(("claude".to_string(), Role::Dynamic(critics.clone())));
    expected.push(("vibe".to_string(), Role::Dynamic(critics)));
    expected.push(("vibe-study".to_string(), Role::Answer));
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    let roles: Vec<(String, Role)> = info
        .roles
        .iter()
        .map(|(agent_id, role)| (agent_id.clone(), role.clone()))
        .collect();
    assert_eq!(roles, expected);
    assert_eq!(info.roles.len(), info.agent_ids.len());
}

#[test]
fn inspect_decides_a_role_for_every_agent_id() {
    let text = r#"{"id":"roles","nodes":[
        {"provider":"gate-"},
        {"provider":"copy-"},
        {"provider":"critic","output":{"schema":{"findings":[]}}},
        {"provider":"bare"},
        {"provider":"noschema","output":{"kind":"json"}},
        {"provider":"ok","output":{"schema":{"ok":true}}},
        {"provider":"text","output":{"schema":"not an object"}}
    ],"modelBindings":{"bindings":[{"agentId":"bound"}]}}"#;
    let info = inspect(text).expect("inspected");
    assert_eq!(info.roles["gate-"], Role::Findings);
    assert_eq!(info.roles["copy-"], Role::Findings);
    assert_eq!(info.roles["critic"], Role::Findings);
    assert_eq!(info.roles["bare"], Role::Findings);
    assert_eq!(info.roles["noschema"], Role::Findings);
    assert_eq!(info.roles["ok"], Role::Answer);
    assert_eq!(info.roles["text"], Role::Findings);
    assert_eq!(info.roles["bound"], Role::Findings);
}

#[test]
fn inspect_gives_the_spec_stage_template_findings_roles() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../installer/gold-band/workflows/critique-spec-stage.json");
    let info = inspect(&fs::read_to_string(&path).expect("template read")).expect("inspected");
    for agent_id in &info.agent_ids {
        assert_eq!(info.roles[agent_id], Role::Findings, "{agent_id}");
    }
}

#[test]
fn inspect_reads_a_bare_definition() {
    let text = r#"{"id":"bare","nodes":[{"provider":"vibe","model":"m1","profile":"pf"},{"provider":"agy"}],"modelBindings":{"bindings":[{"agentId":"vibe","modelId":"m2"},{"agentId":"solo"}]}}"#;
    let info = inspect(text).expect("inspected");
    assert_eq!(info.id, "bare");
    let agents: Vec<String> = info.agent_ids.iter().cloned().collect();
    assert_eq!(
        agents,
        vec!["agy".to_string(), "solo".to_string(), "vibe".to_string()]
    );
    let vibe: Vec<String> = info.models["vibe"].iter().cloned().collect();
    assert_eq!(vibe, vec!["m1".to_string(), "m2".to_string()]);
    assert!(info.models["agy"].is_empty());
    assert!(info.models["solo"].is_empty());
    assert_eq!(info.profiles.len(), 1);
    assert!(!info.has_manual_check);
}

#[test]
fn inspect_skips_a_wrapper_that_is_not_an_object() {
    let info = inspect(r#"{"workflow":5,"id":"top","nodes":[]}"#).expect("inspected");
    assert_eq!(info.id, "top");
    assert!(info.agent_ids.is_empty());
}

#[test]
fn inspect_ignores_nodes_that_are_not_an_array() {
    let info =
        inspect(r#"{"id":"odd","nodes":5,"modelBindings":{"bindings":7}}"#).expect("inspected");
    assert_eq!(info.id, "odd");
    assert!(info.agent_ids.is_empty());
    assert!(info.profiles.is_empty());
}

#[test]
fn inspect_skips_entries_that_are_not_objects() {
    let text = r#"{"id":"odd","nodes":[5,{"provider":"vibe","manual_check":false}],"modelBindings":{"bindings":[7,{"agentId":"agy"}]}}"#;
    let info = inspect(text).expect("inspected");
    let agents: Vec<String> = info.agent_ids.iter().cloned().collect();
    assert_eq!(agents, vec!["agy".to_string(), "vibe".to_string()]);
    assert!(info.models["vibe"].is_empty());
    assert!(!info.has_manual_check);
}

#[test]
fn inspect_rejects_text_that_is_not_json() {
    let message = inspect("not json").expect_err("rejected");
    assert!(message.contains("is not valid JSON"), "{}", message);
}

#[test]
fn inspect_rejects_a_workflow_without_an_id() {
    assert_eq!(
        inspect(r#"{"nodes":[]}"#),
        Err("the workflow has no id".to_string())
    );
    assert_eq!(
        inspect(r#"{"workflow":{"nodes":[]},"id":"top"}"#),
        Err("the workflow has no id".to_string())
    );
}

#[test]
fn inspect_reads_the_graph() {
    let text = r#"{"workflow":{"id":"graph","entry":"start","nodes":[
        {"id":"start","provider":"vibe"},
        {"id":"check","manual_check":true},
        {"provider":"no-id"}
    ],"edges":[
        {"from":"start","to":"check","on":"success"},
        {"from":"check","to":"$end","on":"success"},
        {"from":"start","to":"$end"},
        {"from":"start","on":"success"},
        {"to":"$end","on":"success"},
        "not an edge"
    ]}}"#;
    let info = inspect(text).expect("inspected");
    assert_eq!(info.entry, "start");
    assert_eq!(
        info.nodes,
        vec![
            node("start", Some("vibe"), false),
            node("check", None, true)
        ]
    );
    assert_eq!(
        info.edges,
        vec![
            edge("start", "check", "success"),
            edge("check", "$end", "success"),
        ]
    );
}

#[test]
fn inspect_leaves_the_entry_empty_when_the_workflow_names_none() {
    let info = inspect(r#"{"id":"noentry","nodes":[],"edges":5}"#).expect("inspected");
    assert_eq!(info.entry, "");
    assert!(info.nodes.is_empty());
    assert!(info.edges.is_empty());
}

#[test]
fn inspect_makes_an_open_questions_schema_the_questions_role() {
    let text = r#"{"id":"questions","nodes":[
        {"provider":"grill","output":{"schema":{"open_questions":[]}}},
        {"provider":"both","output":{"schema":{"open_questions":[],"ok":true}}},
        {"provider":"gate-build","output":{"schema":{"open_questions":[]}}},
        {"provider":"copy-build","output":{"schema":{"open_questions":[]}}}
    ]}"#;
    let info = inspect(text).expect("inspected");
    assert_eq!(info.roles["grill"], Role::Questions);
    assert_eq!(info.roles["both"], Role::Questions);
    assert_eq!(info.roles["gate-build"], Role::Gate("build".to_string()));
    assert_eq!(info.roles["copy-build"], Role::Copy("build".to_string()));
}

#[test]
fn walk_follows_success_edges_to_the_end() {
    let info = graph(
        vec![
            node("a", Some("vibe"), false),
            node("b", Some("vibe"), false),
        ],
        vec![edge("a", "b", "success"), edge("b", "$end", "success")],
        &[("vibe", Role::Answer)],
    );
    assert_eq!(
        walk(&info, "a", "clean"),
        Walk {
            exit: 0,
            failed: None,
            failures: Vec::new(),
            visited: vec!["a".to_string(), "b".to_string()],
        }
    );
}

#[test]
fn walk_routes_a_failure_to_a_manual_check_and_exits_2() {
    let info = graph(
        vec![
            node("verdicts", Some("vibe"), false),
            node("check", None, true),
        ],
        vec![
            edge("verdicts", "$end", "success"),
            edge("verdicts", "check", "failure"),
            edge("check", "$end", "success"),
        ],
        &[("vibe", Role::Findings)],
    );
    assert_eq!(
        walk(&info, "verdicts", " needs-human "),
        Walk {
            exit: 2,
            failed: Some("verdicts".to_string()),
            failures: vec!["verdicts".to_string()],
            visited: vec!["verdicts".to_string(), "check".to_string()],
        }
    );
}

#[test]
fn walk_exits_1_when_a_failure_has_no_failure_edge() {
    let info = graph(
        vec![node("verdicts", Some("vibe"), false)],
        vec![edge("verdicts", "$end", "success")],
        &[("vibe", Role::Findings)],
    );
    // No failure edge: the node has no success condition to miss.
    assert_eq!(walk(&info, "verdicts", "needs-human").exit, 0);
    let info = graph(
        vec![node("gate", Some("gate-x"), false)],
        vec![edge("gate", "$end", "success")],
        &[("gate-x", Role::Gate("x".to_string()))],
    );
    assert_eq!(
        walk(&info, "gate", "gate-fail=x"),
        Walk {
            exit: 1,
            failed: Some("gate".to_string()),
            failures: vec!["gate".to_string()],
            visited: vec!["gate".to_string()],
        }
    );
}

#[test]
fn walk_exits_0_when_a_success_has_no_success_edge() {
    let info = graph(vec![node("a", None, false)], Vec::new(), &[]);
    assert_eq!(
        walk(&info, "a", "clean"),
        Walk {
            exit: 0,
            failed: None,
            failures: Vec::new(),
            visited: vec!["a".to_string()],
        }
    );
}

#[test]
fn walk_fails_a_questions_node_under_open_questions() {
    let info = graph(
        vec![node("grill", Some("vibe-study"), false)],
        vec![
            edge("grill", "$end", "success"),
            edge("grill", "$end", "failure"),
        ],
        &[("vibe-study", Role::Questions)],
    );
    assert_eq!(walk(&info, "grill", "clean").exit, 0);
    assert_eq!(walk(&info, "grill", "needs-human").exit, 0);
    // A drafting node on the same agent has no failure edge and never fails.
    let drafting = graph(
        vec![node("draft", Some("vibe-study"), false)],
        vec![edge("draft", "$end", "success")],
        &[("vibe-study", Role::Questions)],
    );
    assert_eq!(walk(&drafting, "draft", "open-questions").exit, 0);
    assert_eq!(
        walk(&info, "grill", "open-questions"),
        Walk {
            exit: 1,
            failed: Some("grill".to_string()),
            failures: vec!["grill".to_string()],
            visited: vec!["grill".to_string()],
        }
    );
}

#[test]
fn walk_fails_a_copier_under_its_gate_failure() {
    let info = graph(
        vec![node("copy-build", Some("copy-build"), false)],
        vec![edge("copy-build", "$end", "success")],
        &[("copy-build", Role::Copy("build".to_string()))],
    );
    assert_eq!(walk(&info, "copy-build", "gate-fail=build").exit, 1);
    assert_eq!(walk(&info, "copy-build", "gate-fail=lint").exit, 0);
}

#[test]
fn walk_exits_1_when_the_entry_is_not_a_node() {
    let info = graph(
        vec![node("a", Some("vibe"), false)],
        Vec::new(),
        &[("vibe", Role::Answer)],
    );
    assert_eq!(
        walk(&info, "absent", "clean"),
        Walk {
            exit: 1,
            failed: None,
            failures: Vec::new(),
            visited: Vec::new(),
        }
    );
}

#[test]
fn walk_exits_1_when_a_cycle_hits_the_step_cap() {
    let info = graph(
        vec![
            node("a", Some("vibe"), false),
            node("b", Some("vibe"), false),
        ],
        vec![edge("a", "b", "success"), edge("b", "a", "success")],
        &[("vibe", Role::Answer)],
    );
    let walked = walk(&info, "a", "clean");
    assert_eq!(walked.exit, 1);
    assert_eq!(walked.failed, None);
    assert_eq!(walked.visited.len(), 1000);
    assert_eq!(walked.visited.first().map(String::as_str), Some("a"));
    assert_eq!(walked.visited.last().map(String::as_str), Some("b"));
}

#[test]
fn walk_keeps_the_first_failed_node_and_every_visited_node() {
    let info = graph(
        vec![
            node("first", Some("vibe"), false),
            node("second", Some("vibe"), false),
        ],
        vec![
            edge("first", "second", "failure"),
            edge("second", "$end", "failure"),
        ],
        &[("vibe", Role::Findings)],
    );
    assert_eq!(
        walk(&info, "first", "needs-human"),
        Walk {
            exit: 1,
            failed: Some("first".to_string()),
            failures: vec!["first".to_string(), "second".to_string()],
            visited: vec!["first".to_string(), "second".to_string()],
        }
    );
}

#[test]
fn default_scenarios_without_a_manual_check() {
    let mut info = info();
    info.roles.insert("vibe".to_string(), Role::Answer);
    assert_eq!(default_scenarios(&info), vec![spec("clean", "clean", 0)]);
    // A node that never fails and no edge to follow still only proves `clean`.
    info.edges.clear();
    assert_eq!(default_scenarios(&info), vec![spec("clean", "clean", 0)]);
}

#[test]
fn default_scenarios_with_a_manual_check() {
    let info = graph(
        vec![node("only", Some("vibe"), false), node("check", None, true)],
        vec![
            edge("only", "$end", "success"),
            edge("only", "check", "failure"),
            edge("check", "$end", "success"),
        ],
        &[("vibe", Role::Findings)],
    );
    assert!(info.has_manual_check);
    // The check ends the run, so it starts no segment of its own.
    assert_eq!(
        default_scenarios(&info),
        vec![
            spec("clean", "clean", 0),
            spec("needs-human", "needs-human", 2)
        ]
    );
}

#[test]
fn default_scenarios_make_no_segment_after_a_manual_check_without_a_success_edge() {
    let info = graph(
        vec![node("only", Some("vibe"), false), node("check", None, true)],
        vec![
            edge("only", "$end", "success"),
            edge("only", "check", "failure"),
        ],
        &[("vibe", Role::Findings)],
    );
    assert_eq!(
        default_scenarios(&info),
        vec![
            spec("clean", "clean", 0),
            spec("needs-human", "needs-human", 2)
        ]
    );
}

#[test]
fn default_scenarios_add_a_failing_gate_for_every_gate_the_clean_path_reaches() {
    let names: Vec<String> = default_scenarios(&workflow_b())
        .into_iter()
        .map(|scenario| scenario.name)
        .collect();
    assert_eq!(
        names,
        vec![
            "clean",
            "actionable-1",
            "actionable-2",
            "actionable-3",
            "gate-fail-build",
            "gate-fail-coverage",
            "gate-fail-findings-1",
            "gate-fail-lint",
            "gate-fail-test",
        ]
    );
}

#[test]
fn default_scenarios_prove_each_repair_and_the_escalation_of_workflow_b() {
    let scenarios = default_scenarios(&workflow_b());
    let exit = |name: &str| {
        scenarios
            .iter()
            .find(|scenario| scenario.name == name)
            .map(|scenario| scenario.expected_exit)
    };
    // A repair that clears the findings ends the run; the second failed repair
    // reaches the escalation node and its manual check, which pauses the run.
    assert_eq!(exit("clean"), Some(0));
    assert_eq!(exit("actionable-1"), Some(0));
    assert_eq!(exit("actionable-2"), Some(0));
    assert_eq!(exit("actionable-3"), Some(2));
    assert_eq!(exit("gate-fail-findings-1"), Some(1));
}

#[test]
fn walk_of_workflow_b_reaches_the_escalation_only_after_two_repairs() {
    let info = workflow_b();
    let walked = walk(&info, &info.entry, "actionable=3");
    assert_eq!(walked.exit, 2);
    assert_eq!(walked.failures, vec!["verdict-1", "verdict-2", "verdict-3"]);
    let tail: Vec<&str> = walked
        .visited
        .iter()
        .rev()
        .take(2)
        .map(String::as_str)
        .collect();
    assert_eq!(tail, vec!["escalation-check", "workflow-b-escalation"]);
    assert!(walked.visited.contains(&"repair-1".to_string()));
    assert!(walked.visited.contains(&"repair-2".to_string()));
    assert!(!walked.visited.contains(&"repair-3".to_string()));
}

#[test]
fn default_scenarios_ignore_a_copier_role() {
    let mut info = info();
    info.roles
        .insert("vibe".to_string(), Role::Copy("build".to_string()));
    assert_eq!(default_scenarios(&info), vec![spec("clean", "clean", 0)]);
}

#[test]
fn default_scenarios_add_open_questions_for_a_questions_node() {
    let info = graph(
        vec![node("grill", Some("vibe-study"), false)],
        vec![
            edge("grill", "$end", "success"),
            edge("grill", "$end", "failure"),
        ],
        &[("vibe-study", Role::Questions)],
    );
    assert_eq!(
        default_scenarios(&info),
        vec![
            spec("clean", "clean", 0),
            spec("open-questions", "open-questions", 1),
        ]
    );
}

/// The graph the segment tests walk: a start gate, a verdicts node that routes
/// a failure to a manual check and a success to the end, and the refute gate
/// behind the check.
fn segment_graph() -> WorkflowInfo {
    graph(
        vec![
            node("start", Some("gate-start"), false),
            node("verdicts", Some("vibe"), false),
            node("check", None, true),
            node("refute", Some("gate-refute"), false),
        ],
        vec![
            edge("start", "verdicts", "success"),
            edge("verdicts", "$end", "success"),
            edge("verdicts", "check", "failure"),
            edge("check", "refute", "success"),
            edge("refute", "$end", "success"),
        ],
        &[
            ("gate-start", Role::Gate("start".to_string())),
            ("gate-refute", Role::Gate("refute".to_string())),
            ("vibe", Role::Findings),
        ],
    )
}

#[test]
fn default_scenarios_walk_the_whole_workflow_and_the_stretch_after_a_manual_check() {
    assert_eq!(
        default_scenarios(&segment_graph()),
        vec![
            spec("clean", "clean", 0),
            spec("needs-human", "needs-human", 2),
            spec("gate-fail-start", "gate-fail=start", 1),
            segment_spec("after-check.clean", "clean", 0, "refute"),
            segment_spec(
                "after-check.gate-fail-refute",
                "gate-fail=refute",
                1,
                "refute"
            ),
        ]
    );
}

#[test]
fn default_scenarios_give_a_gate_the_first_segment_walks_its_own_scenario() {
    let info = graph(
        vec![
            node("start", Some("gate-start"), false),
            node("verdicts", Some("vibe"), false),
            node("check", None, true),
            node("refute", Some("gate-refute"), false),
        ],
        vec![
            edge("start", "verdicts", "success"),
            edge("verdicts", "refute", "success"),
            edge("verdicts", "check", "failure"),
            edge("check", "refute", "success"),
            edge("refute", "$end", "success"),
        ],
        &[
            ("gate-start", Role::Gate("start".to_string())),
            ("gate-refute", Role::Gate("refute".to_string())),
            ("vibe", Role::Findings),
        ],
    );
    // The refute gate sits on the first segment's success path too, so it is
    // proved there, and the segment after the check does not prove it again.
    assert_eq!(
        default_scenarios(&info),
        vec![
            spec("clean", "clean", 0),
            spec("needs-human", "needs-human", 2),
            spec("gate-fail-refute", "gate-fail=refute", 1),
            spec("gate-fail-start", "gate-fail=start", 1),
            segment_spec("after-check.clean", "clean", 0, "refute"),
        ]
    );
}

#[test]
fn parse_scenario_reads_a_spec() {
    assert_eq!(
        parse_scenario("clean:clean:0").expect("parsed"),
        spec("clean", "clean", 0)
    );
    assert_eq!(
        parse_scenario("paused-run:needs-human:2").expect("parsed"),
        spec("paused-run", "needs-human", 2)
    );
    assert_eq!(
        parse_scenario("a.b_c-1:fail:1").expect("parsed"),
        spec("a.b_c-1", "fail", 1)
    );
}

#[test]
fn parse_scenario_reads_the_open_questions_verdict() {
    assert_eq!(
        parse_scenario("questions:open-questions:2").expect("parsed"),
        spec("questions", "open-questions", 2)
    );
}

#[test]
fn parse_scenario_rejects_the_wrong_number_of_parts() {
    let message = parse_scenario("clean:clean").expect_err("rejected");
    assert!(
        message.contains("three colon separated parts"),
        "{}",
        message
    );
    let message = parse_scenario("clean:clean:0:extra").expect_err("rejected");
    assert!(
        message.contains("three colon separated parts"),
        "{}",
        message
    );
}

#[test]
fn parse_scenario_rejects_an_empty_name() {
    let message = parse_scenario(":clean:0").expect_err("rejected");
    assert!(message.contains("empty name"), "{}", message);
}

#[test]
fn parse_scenario_rejects_a_name_with_other_characters() {
    let message = parse_scenario("clean run:clean:0").expect_err("rejected");
    assert!(message.contains("may only hold letters"), "{}", message);
}

#[test]
fn parse_scenario_reads_a_gate_failure() {
    assert_eq!(
        parse_scenario("gate-fail-build:gate-fail=build:1").expect("parsed"),
        spec("gate-fail-build", "gate-fail=build", 1)
    );
    assert_eq!(
        parse_scenario("g:gate-fail=a.b_c-1:1").expect("parsed"),
        spec("g", "gate-fail=a.b_c-1", 1)
    );
}

#[test]
fn parse_scenario_rejects_a_gate_failure_without_a_name() {
    let message = parse_scenario("g:gate-fail=:1").expect_err("rejected");
    assert!(message.contains("unknown verdict"), "{}", message);
    assert!(message.contains("gate-fail=NAME"), "{}", message);
}

#[test]
fn parse_scenario_rejects_a_gate_failure_with_other_characters() {
    let message = parse_scenario("g:gate-fail=build run:1").expect_err("rejected");
    assert!(message.contains("gate-fail=NAME"), "{}", message);
}

#[test]
fn parse_scenario_rejects_an_unknown_verdict() {
    let message = parse_scenario("clean:broken:0").expect_err("rejected");
    assert!(message.contains("unknown verdict `broken`"), "{}", message);
    assert!(message.contains("gate-fail=NAME"), "{}", message);
}

#[test]
fn parse_scenario_rejects_an_exit_that_is_not_an_integer() {
    let message = parse_scenario("clean:clean:zero").expect_err("rejected");
    assert!(message.contains("not an integer"), "{}", message);
}

/// A wrapper workflow with a branch the entry cannot reach, a node without an
/// id and a binding without a slot.
const PRUNE_WORKFLOW: &str = r#"{"description":"keep me","workflow":{"id":"prune","entry":"a","nodes":[
    {"id":"a","executionSlotId":"slot-a","provider":"vibe"},
    {"id":"b","executionSlotId":"slot-b","provider":"vibe"},
    {"id":"c","executionSlotId":"slot-c","provider":"agy"},
    {"provider":"no-id"}
],"edges":[
    {"from":"a","to":"b","on":"success"},
    {"from":"b","to":"$end","on":"success"},
    {"from":"c","to":"a","on":"success"}
]},"modelBindings":{"bindings":[
    {"executionSlotId":"slot-a","agentId":"vibe"},
    {"executionSlotId":"slot-b","agentId":"vibe"},
    {"executionSlotId":"slot-c","agentId":"agy"},
    {"agentId":"unslotted"}
]}}"#;

#[test]
fn prune_keeps_only_what_the_entry_reaches() {
    let pruned: Value =
        serde_json::from_str(&prune(PRUNE_WORKFLOW, "a").expect("pruned")).expect("json");
    assert_eq!(pruned["description"], "keep me");
    assert_eq!(pruned["workflow"]["id"], "prune");
    assert_eq!(pruned["workflow"]["entry"], "a");
    let ids: Vec<&str> = pruned["workflow"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|node| node["id"].as_str().expect("every kept node has an id"))
        .collect();
    assert_eq!(ids, vec!["a", "b"]);
    assert_eq!(
        pruned["workflow"]["edges"],
        json!([
            {"from": "a", "to": "b", "on": "success"},
            {"from": "b", "to": "$end", "on": "success"}
        ])
    );
    let slots: Vec<&str> = pruned["modelBindings"]["bindings"]
        .as_array()
        .expect("bindings")
        .iter()
        .map(|binding| binding["executionSlotId"].as_str().expect("slot"))
        .collect();
    assert_eq!(slots, vec!["slot-a", "slot-b"]);
}

#[test]
fn prune_keeps_a_bare_definition_bare() {
    let text = r#"{"id":"bare","entry":"a","nodes":[{"id":"a"},{"id":"b"}],"edges":[{"from":"a","to":"b","on":"success"}]}"#;
    let pruned: Value = serde_json::from_str(&prune(text, "b").expect("pruned")).expect("json");
    assert!(pruned.get("workflow").is_none());
    assert_eq!(pruned["id"], "bare");
    assert_eq!(pruned["entry"], "b");
    assert_eq!(pruned["nodes"], json!([{"id": "b"}]));
    assert_eq!(pruned["edges"], json!([]));
}

#[test]
fn prune_treats_a_wrapper_that_is_not_an_object_as_bare() {
    let text = r#"{"workflow":5,"id":"top","nodes":[{"id":"a"}],"edges":[]}"#;
    let pruned: Value = serde_json::from_str(&prune(text, "a").expect("pruned")).expect("json");
    assert_eq!(pruned["workflow"], json!(5));
    assert_eq!(pruned["id"], "top");
    assert_eq!(pruned["entry"], "a");
    assert_eq!(pruned["nodes"], json!([{"id": "a"}]));
}

#[test]
fn prune_keeps_a_wrapper_whose_bindings_it_cannot_read() {
    for bindings in ["", r#","modelBindings":{"bindings":7}"#] {
        let text = format!(
            r#"{{"workflow":{{"id":"w","entry":"a","nodes":[{{"id":"a"}}],"edges":[]}}{}}}"#,
            bindings
        );
        let pruned: Value =
            serde_json::from_str(&prune(&text, "a").expect("pruned")).expect("json");
        assert_eq!(pruned["workflow"]["entry"], "a", "{}", bindings);
        assert_eq!(pruned["workflow"]["nodes"], json!([{"id": "a"}]));
    }
}

#[test]
fn prune_rejects_an_entry_that_is_not_a_node() {
    assert_eq!(
        prune(PRUNE_WORKFLOW, "absent"),
        Err("the entry `absent` is not a node of the workflow".to_string())
    );
}

#[test]
fn prune_rejects_a_document_that_is_not_an_object() {
    assert_eq!(
        prune("[1,2]", "a"),
        Err("the entry `a` is not a node of the workflow".to_string())
    );
}

#[test]
fn prune_rejects_text_that_is_not_json() {
    let message = prune("not json", "a").expect_err("rejected");
    assert!(message.contains("is not valid JSON"), "{}", message);
}

#[test]
fn exit_for_state_maps_every_state() {
    assert_eq!(exit_for_state("completed", Some("success")), Some(0));
    assert_eq!(exit_for_state("completed", Some("failure")), Some(1));
    assert_eq!(exit_for_state("completed", None), Some(1));
    assert_eq!(exit_for_state("paused", None), Some(2));
    assert_eq!(exit_for_state("running", Some("success")), Some(4));
    assert_eq!(exit_for_state("unknown", Some("success")), None);
}

#[test]
fn parse_run_state_reads_one_object() {
    assert_eq!(
        parse_run_state(r#"{"status":"completed","outcome":"success"}"#),
        Some(("completed".to_string(), Some("success".to_string())))
    );
    assert_eq!(
        parse_run_state("{\"status\":\"running\"}\n"),
        Some(("running".to_string(), None))
    );
    assert_eq!(
        parse_run_state("{\"status\":\"paused\",\"outcome\":null}"),
        Some(("paused".to_string(), None))
    );
}

#[test]
fn parse_run_state_finds_the_object_after_log_noise() {
    let output = "gold-band: starting\n{\n  \"status\": \"paused\",\n  \"outcome\": null\n}\n";
    assert_eq!(parse_run_state(output), Some(("paused".to_string(), None)));
}

#[test]
fn parse_run_state_returns_none_for_garbage() {
    assert_eq!(parse_run_state("no state here"), None);
    assert_eq!(parse_run_state(""), None);
    assert_eq!(parse_run_state("[1,2]"), None);
}

#[test]
fn parse_run_state_returns_none_when_the_object_does_not_parse() {
    assert_eq!(parse_run_state("{\nnot json\n"), None);
}

#[test]
fn parse_run_state_returns_none_without_a_status() {
    assert_eq!(parse_run_state(r#"{"outcome":"success"}"#), None);
    assert_eq!(parse_run_state(r#"{"status":5}"#), None);
}

#[test]
fn prepare_writes_the_home_and_the_workspace() {
    let dir = temp_dir("prepare");
    let root = dir.join("root");
    let stub = Path::new("/usr/local/bin/gold-band-proof");
    let (home, workspace) = prepare(&root, &info(), stub, "clean").expect("prepared");
    assert_eq!(home, root.join("home"));
    assert_eq!(workspace, root.join("workspace"));

    let settings_text =
        fs::read_to_string(home.join(".gold-band/settings.json")).expect("settings");
    let settings: Value = serde_json::from_str(&settings_text).expect("settings parsed");
    assert_eq!(settings["settingsSchemaVersion"].as_i64(), Some(12));
    let adapter = &settings["agents"]["vibe"]["adapter"];
    assert_eq!(adapter["command"], "/usr/local/bin/gold-band-proof");
    assert_eq!(adapter["args"][0], "stub-acp");
    assert_eq!(adapter["displayName"], "Stub ACP (vibe)");
    assert_eq!(adapter["env"]["STUB_ACP_VERDICT"], "clean");
    assert_eq!(adapter["env"]["STUB_ACP_MODELS"], "m1");
    assert_eq!(adapter["env"]["STUB_ACP_ROLE"], "findings");
    assert_eq!(
        adapter["env"]["STUB_ACP_REPORT_DIR"],
        root.join("reports").to_str().expect("utf8")
    );
    assert!(root.join("reports").is_dir());
    assert_eq!(
        settings["agents"]["agy"]["adapter"]["env"]["STUB_ACP_MODELS"],
        ""
    );

    let profile =
        fs::read_to_string(home.join(".gold-band/context/profiles/pf.md")).expect("profile");
    assert_eq!(
        profile,
        "---\nid: pf\nname: Stub profile pf\nsummary: Stub profile for the headless proof.\ncreatedAt: 2026-11-03T00:00:00Z\nupdatedAt: 2026-11-03T00:00:00Z\ndynamicTemplate: false\n---\n\nStub profile for the headless proof.\n"
    );

    let requirement = fs::read_to_string(workspace.join("requirement.md")).expect("requirement");
    assert!(requirement.contains("Requirement"), "{}", requirement);
    assert!(workspace.join(".git").is_dir());

    let count = Command::new("git")
        .arg("-C")
        .arg(&workspace)
        .args(["rev-list", "--count", "HEAD"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git rev-list ran");
    assert!(
        count.status.success(),
        "{}",
        String::from_utf8_lossy(&count.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&count.stdout).trim(), "1");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prepare_writes_the_role_of_each_agent() {
    let dir = temp_dir("prepare-roles");
    let root = dir.join("root");
    let mut info = info();
    info.roles
        .insert("vibe".to_string(), Role::Gate("build".to_string()));
    info.roles
        .insert("agy".to_string(), Role::Copy("build".to_string()));
    let (home, _) = prepare(&root, &info, Path::new("/bin/true"), "clean").expect("prepared");
    let text = fs::read_to_string(home.join(".gold-band/settings.json")).expect("settings");
    let settings: Value = serde_json::from_str(&text).expect("settings parsed");
    assert_eq!(
        settings["agents"]["vibe"]["adapter"]["env"]["STUB_ACP_ROLE"],
        "gate:build"
    );
    assert_eq!(
        settings["agents"]["agy"]["adapter"]["env"]["STUB_ACP_ROLE"],
        "copy:build"
    );
    assert_eq!(
        settings["agents"]["agy"]["adapter"]["env"]["STUB_ACP_REPORT_DIR"],
        root.join("reports").to_str().expect("utf8")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prepare_fails_when_the_report_directory_cannot_be_created() {
    let dir = temp_dir("prepare-reports");
    let root = dir.join("root");
    fs::create_dir_all(&root).expect("root created");
    fs::write(root.join("reports"), "not a directory").expect("reports written");
    let message = prepare(&root, &info(), Path::new("/bin/true"), "clean").expect_err("failed");
    assert!(
        message.contains("cannot create the report directory"),
        "{}",
        message
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prepare_fails_when_the_root_is_a_file() {
    let dir = temp_dir("prepare-root-file");
    let root = dir.join("root");
    fs::write(&root, "not a directory").expect("root written");
    let message = prepare(&root, &info(), Path::new("/bin/true"), "clean").expect_err("failed");
    assert!(
        message.contains("cannot create the Gold Band home"),
        "{}",
        message
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prepare_fails_when_the_settings_cannot_be_written() {
    let dir = temp_dir("prepare-settings");
    let root = dir.join("root");
    fs::create_dir_all(root.join("home/.gold-band/settings.json")).expect("blocking dir");
    let message = prepare(&root, &info(), Path::new("/bin/true"), "clean").expect_err("failed");
    assert!(
        message.contains("cannot write the Gold Band settings"),
        "{}",
        message
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prepare_fails_when_a_profile_cannot_be_written() {
    let dir = temp_dir("prepare-profile");
    let root = dir.join("root");
    fs::create_dir_all(root.join("home/.gold-band/context/profiles/pf.md")).expect("blocking dir");
    let message = prepare(&root, &info(), Path::new("/bin/true"), "clean").expect_err("failed");
    assert!(message.contains("cannot write the profile"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prepare_fails_when_the_workspace_cannot_be_created() {
    let dir = temp_dir("prepare-workspace");
    let root = dir.join("root");
    fs::create_dir_all(&root).expect("root created");
    fs::write(root.join("workspace"), "not a directory").expect("workspace written");
    let message = prepare(&root, &info(), Path::new("/bin/true"), "clean").expect_err("failed");
    assert!(
        message.contains("cannot create the workspace"),
        "{}",
        message
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prepare_fails_when_the_requirement_cannot_be_written() {
    let dir = temp_dir("prepare-requirement");
    let root = dir.join("root");
    fs::create_dir_all(root.join("workspace/requirement.md")).expect("blocking dir");
    let message = prepare(&root, &info(), Path::new("/bin/true"), "clean").expect_err("failed");
    assert!(
        message.contains("cannot write the requirement"),
        "{}",
        message
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn git_step_names_a_missing_program() {
    let dir = temp_dir("git-missing");
    let message = git_step(
        Path::new("/nonexistent/git"),
        &dir,
        &dir,
        "init",
        &["init", "-q"],
    )
    .expect_err("failed");
    assert!(message.contains("cannot run git init"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn git_step_names_a_failing_step() {
    let dir = temp_dir("git-failing");
    let message = git_step(
        Path::new("git"),
        &dir,
        &dir,
        "rev-parse",
        &["rev-parse", "--verify", "refs/heads/definitely-absent"],
    )
    .expect_err("failed");
    assert!(message.contains("git rev-parse failed"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_reports_a_clean_run() {
    let dir = temp_dir("run-clean");
    let gold_band = fake(&dir, CLEAN_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(result.name, "clean");
    assert_eq!(result.expected_exit, 0);
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.run_status.as_deref(), Some("completed"));
    assert_eq!(result.run_outcome.as_deref(), Some("success"));
    assert_eq!(result.note, None);
    assert!(result.passed());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_reports_a_paused_run() {
    let dir = temp_dir("run-paused");
    let gold_band = fake(&dir, PAUSED_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("needs-human", "needs-human", 2),
        10,
    );
    assert_eq!(result.exit_code, Some(2));
    assert_eq!(result.run_status.as_deref(), Some("paused"));
    assert_eq!(result.run_outcome, None);
    assert_eq!(result.note, None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_reports_a_run_without_a_state() {
    let dir = temp_dir("run-nostate");
    let gold_band = fake(&dir, NO_STATE_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.run_status, None);
    assert_eq!(
        result.note.as_deref(),
        Some("exit 0 is not backed by a printed run state (none/none)")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_notes_a_wrong_exit_with_stderr() {
    let dir = temp_dir("run-wrong");
    let gold_band = fake(&dir, WRONG_EXIT_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(result.exit_code, Some(3));
    assert_eq!(result.note.as_deref(), Some("expected exit 0, got 3: boom"));
    assert!(!result.passed());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_notes_a_wrong_exit_without_stderr() {
    let dir = temp_dir("run-wrong-silent");
    let gold_band = fake(&dir, WRONG_EXIT_SILENT_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(result.note.as_deref(), Some("expected exit 0, got 3"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_keeps_the_last_300_characters_of_stderr() {
    let dir = temp_dir("run-wrong-long");
    let long = "x".repeat(400);
    let body = format!(
        "#!/bin/sh\nprintf '{{\"status\":\"completed\",\"outcome\":\"success\"}}\\n'\nprintf '%s\\n' '{}' >&2\nexit 3\n",
        long
    );
    let gold_band = fake(&dir, &body);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(
        result.note,
        Some(format!("expected exit 0, got 3: {}", "x".repeat(300)))
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_notes_a_disagreeing_run_state() {
    let dir = temp_dir("run-disagree");
    let gold_band = fake(&dir, DISAGREEING_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(
        result.note.as_deref(),
        Some("exit 0 is not backed by a printed run state (completed/failure)")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_notes_a_disagreeing_state_without_an_outcome() {
    let dir = temp_dir("run-disagree-bare");
    let gold_band = fake(&dir, DISAGREEING_NO_OUTCOME_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(
        result.note.as_deref(),
        Some("exit 0 is not backed by a printed run state (completed/none)")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_ignores_an_unknown_run_state() {
    let dir = temp_dir("run-unknown");
    let gold_band = fake(&dir, UNKNOWN_STATE_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(result.run_status.as_deref(), Some("weird"));
    assert_eq!(
        result.note.as_deref(),
        Some("exit 0 is not backed by a printed run state (weird/none)")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_kills_a_hanging_run() {
    let dir = temp_dir("run-hang");
    let gold_band = fake(&dir, HANG_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        1,
    );
    assert_eq!(result.exit_code, None);
    assert_eq!(result.note.as_deref(), Some("timed out after 1 seconds"));
    assert!(!result.passed());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_notes_a_missing_binary() {
    let dir = temp_dir("run-missing");
    let gold_band = dir.join("absent-gold-band");
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(result.exit_code, None);
    let note = result.note.expect("note");
    assert!(note.contains("cannot run"), "{}", note);
    assert!(note.contains("absent-gold-band"), "{}", note);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_notes_a_signal_death() {
    let dir = temp_dir("run-signal");
    let gold_band = fake(&dir, SIGNAL_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(result.exit_code, None);
    assert_eq!(result.note.as_deref(), Some("killed by a signal"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_notes_a_prepare_failure() {
    let dir = temp_dir("run-prepare");
    let gold_band = fake(&dir, CLEAN_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    fs::write(&root, "not a directory").expect("root written");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(result.exit_code, None);
    let note = result.note.expect("note");
    assert!(
        note.contains("cannot create the Gold Band home"),
        "{}",
        note
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_notes_an_unwritable_stdout() {
    let dir = temp_dir("run-stdout");
    let gold_band = fake(&dir, CLEAN_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    fs::create_dir_all(root.join("stdout.txt")).expect("blocking dir");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(result.exit_code, None);
    let note = result.note.expect("note");
    assert!(note.contains("cannot create"), "{}", note);
    assert!(note.contains("stdout.txt"), "{}", note);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_scenario_notes_an_unwritable_stderr() {
    let dir = temp_dir("run-stderr");
    let gold_band = fake(&dir, CLEAN_FAKE);
    let workflow = workflow_file(&dir, WORKFLOW);
    let root = dir.join("root");
    fs::create_dir_all(root.join("stderr.txt")).expect("blocking dir");
    let result = run_scenario(
        &gold_band,
        &workflow,
        &root,
        &info(),
        Path::new("/bin/true"),
        &spec("clean", "clean", 0),
        10,
    );
    assert_eq!(result.exit_code, None);
    let note = result.note.expect("note");
    assert!(note.contains("cannot create"), "{}", note);
    assert!(note.contains("stderr.txt"), "{}", note);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_skips_without_a_binary() {
    let dir = temp_dir("prove-skip");
    let record = prove(&options(&dir, None)).expect("proved");
    assert_eq!(record.outcome, Outcome::Skipped);
    assert!(record.scenarios.is_empty());
    assert_eq!(record.skip_reason.as_deref(), Some(SKIP_NO_BINARY));
    assert_eq!(record.workflow_id, "proof-test");
    assert_eq!(record.workflow_sha256.len(), 64);
    assert_eq!(record.gold_band_commit.as_deref(), Some("abc1234"));
    assert_eq!(record.recorded_at, "2026-11-11T16:53:26Z");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_passes_a_clean_workflow() {
    let dir = temp_dir("prove-pass");
    let gold_band = fake(&dir, VERDICT_FAKE);
    let record = prove(&options(&dir, Some(gold_band))).expect("proved");
    assert_eq!(record.outcome, Outcome::Passed);
    assert_eq!(record.skip_reason, None);
    assert_eq!(record.scenarios.len(), 1);
    assert_eq!(record.scenarios[0].name, "clean");
    assert_eq!(record.scenarios[0].exit_code, Some(0));
    assert_eq!(record.scenarios[0].note, None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_fails_a_workflow_that_exits_wrongly() {
    let dir = temp_dir("prove-fail");
    let gold_band = fake(&dir, FAILING_FAKE);
    let record = prove(&options(&dir, Some(gold_band))).expect("proved");
    assert_eq!(record.outcome, Outcome::Failed);
    assert_eq!(record.scenarios.len(), 1);
    assert_eq!(
        record.scenarios[0].note.as_deref(),
        Some("expected exit 0, got 1")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_replaces_a_default_scenario() {
    let dir = temp_dir("prove-replace");
    let gold_band = fake(&dir, VERDICT_FAKE);
    let mut options = options(&dir, Some(gold_band));
    options.extra_scenarios = vec![spec("clean", "needs-human", 2)];
    let record = prove(&options).expect("proved");
    assert_eq!(record.outcome, Outcome::Passed);
    assert_eq!(record.scenarios.len(), 1);
    assert_eq!(record.scenarios[0].name, "clean");
    assert_eq!(record.scenarios[0].expected_exit, 2);
    assert_eq!(record.scenarios[0].exit_code, Some(2));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_appends_an_extra_scenario() {
    let dir = temp_dir("prove-append");
    let gold_band = fake(&dir, VERDICT_FAKE);
    let mut options = options(&dir, Some(gold_band));
    options.extra_scenarios = vec![spec("fail-path", "fail", 1)];
    let record = prove(&options).expect("proved");
    assert_eq!(record.outcome, Outcome::Passed);
    let names: Vec<&str> = record
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();
    assert_eq!(names, vec!["clean", "fail-path"]);
    assert_eq!(record.scenarios[1].exit_code, Some(1));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_fails_on_an_unreadable_workflow() {
    let dir = temp_dir("prove-unreadable");
    let mut options = options(&dir, None);
    options.workflow = dir.join("absent.json");
    let message = prove(&options).expect_err("failed");
    assert!(message.contains("cannot read the workflow"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_fails_on_a_workflow_that_is_not_json() {
    let dir = temp_dir("prove-notjson");
    let mut options = options(&dir, None);
    options.workflow = workflow_file(&dir, "not json");
    let message = prove(&options).expect_err("failed");
    assert!(message.contains("is not JSON"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_fails_on_a_workflow_without_an_id() {
    let dir = temp_dir("prove-noid");
    let mut options = options(&dir, None);
    options.workflow = workflow_file(&dir, r#"{"nodes":[]}"#);
    let message = prove(&options).expect_err("failed");
    assert!(message.contains("has no id"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

/// A fake gold-band that appends the arguments of every run to `args` and
/// reports a clean run.
fn args_fake(dir: &Path) -> (PathBuf, PathBuf) {
    let args = dir.join("args");
    let body = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nprintf '{{\"status\":\"completed\",\"outcome\":\"success\"}}\\n'\nexit 0\n",
        args.display()
    );
    (fake(dir, &body), args)
}

#[test]
fn prove_runs_a_segment_from_a_pruned_copy_of_the_workflow() {
    let dir = temp_dir("prove-segment");
    let (gold_band, args) = args_fake(&dir);
    let mut options = options(&dir, Some(gold_band));
    options.workflow = workflow_file(&dir, SEGMENT_WORKFLOW);
    let record = prove(&options).expect("proved");
    assert_eq!(record.outcome, Outcome::Passed);
    let names: Vec<&str> = record
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();
    assert_eq!(names, vec!["clean", "after-check.clean"]);

    let runs = fs::read_to_string(&args).expect("arguments read");
    let runs: Vec<&str> = runs.lines().collect();
    assert_eq!(runs.len(), 2, "{:?}", runs);
    let whole = fs::canonicalize(&options.workflow).expect("canonical workflow");
    assert!(
        runs[0].contains(&format!("--workflow {}", whole.display())),
        "{}",
        runs[0]
    );
    let segment = options.scratch.join("1-after-check.clean/segment.json");
    assert!(
        runs[1].contains(&format!("--workflow {}", segment.display())),
        "{}",
        runs[1]
    );
    let pruned: Value =
        serde_json::from_str(&fs::read_to_string(&segment).expect("segment read")).expect("json");
    assert_eq!(pruned["workflow"]["entry"], "after");
    assert_eq!(
        pruned["workflow"]["nodes"],
        json!([{"id": "after", "provider": "vibe", "output": {"kind": "json", "schema": {"ok": true}}}])
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_notes_a_segment_whose_entry_is_not_a_node() {
    let dir = temp_dir("prove-segment-absent");
    let gold_band = fake(&dir, CLEAN_FAKE);
    let mut options = options(&dir, Some(gold_band));
    options.extra_scenarios = vec![segment_spec("segment", "clean", 0, "absent")];
    let record = prove(&options).expect("proved");
    assert_eq!(record.outcome, Outcome::Failed);
    let names: Vec<&str> = record
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();
    assert_eq!(names, vec!["clean", "segment"]);
    assert_eq!(record.scenarios[0].exit_code, Some(0));
    assert_eq!(record.scenarios[1].exit_code, None);
    assert_eq!(
        record.scenarios[1].note.as_deref(),
        Some("the entry `absent` is not a node of the workflow")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn inspect_keeps_agents_that_name_no_model() {
    let info = crate::harness::inspect(
        r#"{"id":"bare","nodes":["not a node",{"provider":"solo"}],
            "modelBindings":{"bindings":["not a binding",{"agentId":"bound"}]}}"#,
    )
    .expect("inspects");
    assert!(info.agent_ids.contains("solo") && info.agent_ids.contains("bound"));
    assert!(info.models["solo"].is_empty());
    assert!(info.models["bound"].is_empty());
}

#[test]
fn inspect_keeps_a_node_without_a_provider_out_of_the_agents() {
    let info = inspect(
        r#"{"id":"x","nodes":[{"id":"manual","manual_check":true},{"id":"n","provider":"a","model":"m"}]}"#,
    )
    .expect("inspected");
    assert_eq!(
        info.agent_ids.into_iter().collect::<Vec<_>>(),
        vec!["a".to_string()]
    );
    assert!(info.has_manual_check);
}

#[test]
fn prune_follows_a_cycle_once() {
    let workflow = r#"{"id":"loop","entry":"a","nodes":[{"id":"a"},{"id":"b"}],"edges":[
        {"from":"a","to":"b","on":"success"},
        {"from":"b","to":"a","on":"failure"},
        {"from":"b","to":"$end","on":"success"}
    ]}"#;
    let pruned: Value = serde_json::from_str(&prune(workflow, "a").expect("pruned")).expect("json");
    assert_eq!(pruned["nodes"].as_array().expect("nodes").len(), 2);
    assert_eq!(pruned["edges"].as_array().expect("edges").len(), 3);
}

#[test]
fn scenario_workflow_names_a_directory_it_cannot_create() {
    let dir = temp_dir("segment-no-dir");
    let blocker = dir.join("file");
    fs::write(&blocker, "x").expect("blocker");
    let spec = ScenarioSpec {
        name: "after-check.clean".to_string(),
        verdict: "clean".to_string(),
        expected_exit: 0,
        entry: Some("a".to_string()),
    };
    let text = r#"{"id":"w","entry":"a","nodes":[{"id":"a"}],"edges":[]}"#;
    let error = scenario_workflow(
        text,
        &info(),
        &dir.join("w.json"),
        &spec,
        &blocker.join("sub"),
    )
    .expect_err("the directory cannot be created");
    assert!(
        error.contains("cannot create the scenario directory"),
        "{error}"
    );
}

/// An `ai-dynamic` node with a dynamic strategy that fans out to two agents.
const DYNAMIC_WORKFLOW: &str = r#"{"id":"dynamic","nodes":[
    {"id":"critics-1","type":"ai-dynamic","agentStrategy":{
        "mode":"dynamic","bootstrapProvider":"vibe","bootstrapModel":"glm-5-3",
        "acceptanceModel":"glm-5-3",
        "availableAgents":[{"provider":"vibe","model":"glm-5-3"},{"provider":"claude","model":"sonnet"}],
        "routingPrompt":"route"
    },"control":{"max_rounds":3}}
]}"#;

#[test]
fn inspect_reads_a_dynamic_strategy() {
    let info = inspect(DYNAMIC_WORKFLOW).expect("inspected");
    assert_eq!(
        info.agent_ids.iter().cloned().collect::<Vec<_>>(),
        vec!["claude".to_string(), "vibe".to_string()]
    );
    assert_eq!(
        info.models["vibe"].iter().cloned().collect::<Vec<_>>(),
        vec!["glm-5-3".to_string()]
    );
    assert_eq!(
        info.models["claude"].iter().cloned().collect::<Vec<_>>(),
        vec!["sonnet".to_string()]
    );
    let workers = vec!["vibe".to_string(), "claude".to_string()];
    assert_eq!(info.roles["vibe"], Role::Dynamic(workers.clone()));
    assert_eq!(info.roles["claude"], Role::Dynamic(workers));
    assert_eq!(info.nodes, vec![node("critics-1", Some("vibe"), false)]);
}

#[test]
fn inspect_reads_a_fixed_strategy() {
    let text = r#"{"id":"fixed","nodes":[
        {"id":"critics-1","type":"ai-dynamic","agentStrategy":{"mode":"fixed","provider":"claude","model":"sonnet"}}
    ]}"#;
    let info = inspect(text).expect("inspected");
    assert_eq!(
        info.agent_ids.iter().cloned().collect::<Vec<_>>(),
        vec!["claude".to_string()]
    );
    assert_eq!(
        info.models["claude"].iter().cloned().collect::<Vec<_>>(),
        vec!["sonnet".to_string()]
    );
    assert_eq!(
        info.roles["claude"],
        Role::Dynamic(vec!["claude".to_string()])
    );
    assert_eq!(info.nodes, vec![node("critics-1", Some("claude"), false)]);
}

#[test]
fn inspect_fans_a_strategy_without_a_list_out_to_its_bootstrap_provider() {
    let text = r#"{"id":"bootstrap","nodes":[
        {"id":"critics-1","type":"ai-dynamic","agentStrategy":{"mode":"dynamic","bootstrapProvider":"vibe",
            "bootstrapModel":"glm-5-3","availableAgents":[5,{"model":"m"}]}}
    ]}"#;
    let info = inspect(text).expect("inspected");
    assert_eq!(info.roles["vibe"], Role::Dynamic(vec!["vibe".to_string()]));
    assert_eq!(
        info.models["vibe"].iter().cloned().collect::<Vec<_>>(),
        vec!["glm-5-3".to_string()]
    );
    assert_eq!(info.nodes, vec![node("critics-1", Some("vibe"), false)]);
}

#[test]
fn inspect_does_not_repeat_a_worker_that_is_also_the_bootstrap_provider() {
    let text = r#"{"id":"same","nodes":[
        {"id":"critics-1","type":"ai-dynamic","agentStrategy":{"mode":"fixed","provider":"vibe",
            "bootstrapProvider":"vibe","model":"m1"}}
    ]}"#;
    let info = inspect(text).expect("inspected");
    assert_eq!(info.roles["vibe"], Role::Dynamic(vec!["vibe".to_string()]));
    assert_eq!(
        info.models["vibe"].iter().cloned().collect::<Vec<_>>(),
        vec!["m1".to_string()]
    );
}

#[test]
fn inspect_keeps_a_prefix_role_over_a_dynamic_membership() {
    let text = r#"{"id":"prefix","nodes":[
        {"id":"critics-1","type":"ai-dynamic","agentStrategy":{"mode":"dynamic","bootstrapProvider":"vibe",
            "availableAgents":[{"provider":"gate-build"},{"provider":"copy-lint"},{"provider":"verdict-2"}]}},
        {"id":"g","provider":"gate-build"},
        {"id":"c","provider":"copy-lint"},
        {"id":"v","provider":"verdict-2"}
    ]}"#;
    let info = inspect(text).expect("inspected");
    assert_eq!(info.roles["gate-build"], Role::Gate("build".to_string()));
    assert_eq!(info.roles["copy-lint"], Role::Copy("lint".to_string()));
    assert_eq!(info.roles["verdict-2"], Role::Verdict(2));
    assert_eq!(
        info.roles["vibe"],
        Role::Dynamic(vec![
            "gate-build".to_string(),
            "copy-lint".to_string(),
            "verdict-2".to_string()
        ])
    );
}

#[test]
fn inspect_reads_a_verdict_round_from_the_agent_id() {
    let text = r#"{"id":"rounds","nodes":[
        {"provider":"verdict-1"},
        {"provider":"verdict-12"},
        {"provider":"verdict-x"},
        {"provider":"verdict-"}
    ]}"#;
    let info = inspect(text).expect("inspected");
    assert_eq!(info.roles["verdict-1"], Role::Verdict(1));
    assert_eq!(info.roles["verdict-12"], Role::Verdict(12));
    assert_eq!(info.roles["verdict-x"], Role::Findings);
    assert_eq!(info.roles["verdict-"], Role::Findings);
}

#[test]
fn inspect_unions_the_workers_two_dynamic_nodes_list_for_one_id() {
    let text = r#"{"id":"union","nodes":[
        {"id":"a","type":"ai-dynamic","agentStrategy":{"mode":"dynamic","bootstrapProvider":"vibe",
            "availableAgents":[{"provider":"claude"},{"provider":"agy"}]}},
        {"id":"b","type":"ai-dynamic","agentStrategy":{"mode":"dynamic","bootstrapProvider":"vibe",
            "availableAgents":[{"provider":"agy"},{"provider":"grok"}]}}
    ]}"#;
    let info = inspect(text).expect("inspected");
    let workers = vec!["claude".to_string(), "agy".to_string(), "grok".to_string()];
    assert_eq!(info.roles["vibe"], Role::Dynamic(workers.clone()));
    assert_eq!(info.roles["agy"], Role::Dynamic(workers));
}

#[test]
fn inspect_falls_back_to_the_node_provider_without_a_strategy() {
    let text = r#"{"id":"fallback","nodes":[
        {"id":"a","type":"ai-dynamic","provider":"vibe"},
        {"id":"b","type":"ai-dynamic","agentStrategy":{"mode":"dynamic"}},
        {"id":"c","type":"ai-dynamic","agentStrategy":5}
    ]}"#;
    let info = inspect(text).expect("inspected");
    assert_eq!(
        info.nodes,
        vec![
            node("a", Some("vibe"), false),
            node("b", None, false),
            node("c", None, false),
        ]
    );
    assert_eq!(info.roles["vibe"], Role::Findings);
}

/// The graph the verdict tests walk: round 1 fails into a repair node, round 2
/// fails into a manual check.
fn verdict_graph() -> WorkflowInfo {
    graph(
        vec![
            node("verdict-1", Some("verdict-1"), false),
            node("repair", Some("vibe"), false),
            node("verdict-2", Some("verdict-2"), false),
            node("check", None, true),
        ],
        vec![
            edge("verdict-1", "verdict-2", "success"),
            edge("verdict-1", "repair", "failure"),
            edge("repair", "verdict-2", "success"),
            edge("verdict-2", "$end", "success"),
            edge("verdict-2", "check", "failure"),
            edge("check", "$end", "success"),
        ],
        &[
            ("verdict-1", Role::Verdict(1)),
            ("verdict-2", Role::Verdict(2)),
            ("vibe", Role::Answer),
        ],
    )
}

#[test]
fn walk_fails_a_verdict_round_up_to_the_actionable_count() {
    let info = verdict_graph();
    assert_eq!(
        walk(&info, "verdict-1", "actionable=1"),
        Walk {
            exit: 0,
            failed: Some("verdict-1".to_string()),
            failures: vec!["verdict-1".to_string()],
            visited: vec![
                "verdict-1".to_string(),
                "repair".to_string(),
                "verdict-2".to_string()
            ],
        }
    );
    // Round 2 fails too, and its failure reaches the manual check.
    for verdict in ["actionable=2", "actionable=3"] {
        assert_eq!(
            walk(&info, "verdict-1", verdict),
            Walk {
                exit: 2,
                failed: Some("verdict-1".to_string()),
                failures: vec!["verdict-1".to_string(), "verdict-2".to_string()],
                visited: vec![
                    "verdict-1".to_string(),
                    "repair".to_string(),
                    "verdict-2".to_string(),
                    "check".to_string()
                ],
            },
            "{}",
            verdict
        );
    }
    // A verdict that names no count fails no round.
    assert_eq!(
        walk(&info, "verdict-1", "clean").failures,
        Vec::<String>::new()
    );
}

#[test]
fn walk_never_fails_a_dynamic_node() {
    let info = graph(
        vec![node("critics", Some("vibe"), false)],
        vec![
            edge("critics", "$end", "success"),
            edge("critics", "$end", "failure"),
        ],
        &[("vibe", Role::Dynamic(vec!["vibe".to_string()]))],
    );
    for verdict in [
        "clean",
        "needs-human",
        "open-questions",
        "actionable=3",
        "fail",
    ] {
        assert_eq!(walk(&info, "critics", verdict).exit, 0, "{}", verdict);
    }
}

/// The graph the actionable scenario test walks: three critic rounds, each
/// failing into its own repair node.
fn rounds_graph() -> WorkflowInfo {
    graph(
        vec![
            node("verdict-1", Some("verdict-1"), false),
            node("repair-1", Some("vibe"), false),
            node("verdict-2", Some("verdict-2"), false),
            node("repair-2", Some("vibe"), false),
            node("verdict-3", Some("verdict-3"), false),
            node("repair-3", Some("vibe"), false),
        ],
        vec![
            edge("verdict-1", "verdict-2", "success"),
            edge("verdict-1", "repair-1", "failure"),
            edge("repair-1", "verdict-2", "success"),
            edge("verdict-2", "verdict-3", "success"),
            edge("verdict-2", "repair-2", "failure"),
            edge("repair-2", "verdict-3", "success"),
            edge("verdict-3", "$end", "success"),
            edge("verdict-3", "repair-3", "failure"),
            edge("repair-3", "$end", "success"),
        ],
        &[
            ("verdict-1", Role::Verdict(1)),
            ("verdict-2", Role::Verdict(2)),
            ("verdict-3", Role::Verdict(3)),
            ("vibe", Role::Answer),
        ],
    )
}

#[test]
fn default_scenarios_prove_every_actionable_count() {
    assert_eq!(
        default_scenarios(&rounds_graph()),
        vec![
            spec("clean", "clean", 0),
            spec("actionable-1", "actionable=1", 0),
            spec("actionable-2", "actionable=2", 0),
            spec("actionable-3", "actionable=3", 0),
        ]
    );
}

#[test]
fn parse_scenario_reads_an_actionable_count() {
    assert_eq!(
        parse_scenario("actionable-2:actionable=2:1").expect("parsed"),
        spec("actionable-2", "actionable=2", 1)
    );
}

#[test]
fn parse_scenario_rejects_an_actionable_count_that_is_not_a_number() {
    let message = parse_scenario("a:actionable=x:1").expect_err("rejected");
    assert!(
        message.contains("unknown verdict `actionable=x`"),
        "{}",
        message
    );
    assert!(message.contains("actionable=N"), "{}", message);
}

#[test]
fn scenario_workflow_names_a_file_it_cannot_write() {
    let dir = temp_dir("segment-no-file");
    fs::create_dir_all(dir.join(SEGMENT_FILE)).expect("a directory where the file goes");
    let spec = ScenarioSpec {
        name: "after-check.clean".to_string(),
        verdict: "clean".to_string(),
        expected_exit: 0,
        entry: Some("a".to_string()),
    };
    let text = r#"{"id":"w","entry":"a","nodes":[{"id":"a"}],"edges":[]}"#;
    let error = scenario_workflow(text, &info(), &dir.join("w.json"), &spec, &dir)
        .expect_err("the file cannot be written");
    assert!(
        error.contains("cannot write the segment workflow"),
        "{error}"
    );
}
