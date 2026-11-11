use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use crate::harness::{
    default_scenarios, exit_for_state, git_step, inspect, parse_run_state, parse_scenario, prepare,
    prove, run_scenario, ProveOptions, ScenarioSpec, WorkflowInfo, SKIP_NO_BINARY,
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
    }
}

fn spec(name: &str, verdict: &str, expected_exit: i32) -> ScenarioSpec {
    ScenarioSpec {
        name: name.to_string(),
        verdict: verdict.to_string(),
        expected_exit,
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

const WORKFLOW: &str =
    r#"{"workflow":{"id":"proof-test","nodes":[{"provider":"vibe","model":"m1","profile":"pf"}]}}"#;

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
    let roles: Vec<(String, Role)> = info
        .roles
        .iter()
        .map(|(agent_id, role)| (agent_id.clone(), role.clone()))
        .collect();
    assert_eq!(
        roles,
        vec![
            ("copy-build".to_string(), Role::Copy("build".to_string())),
            ("copy-lint".to_string(), Role::Copy("lint".to_string())),
            ("copy-test".to_string(), Role::Copy("test".to_string())),
            ("gate-build".to_string(), Role::Gate("build".to_string())),
            ("gate-lint".to_string(), Role::Gate("lint".to_string())),
            ("gate-test".to_string(), Role::Gate("test".to_string())),
            ("vibe".to_string(), Role::Answer),
            ("vibe-study".to_string(), Role::Answer),
        ]
    );
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
fn default_scenarios_without_a_manual_check() {
    let scenarios = default_scenarios(&info());
    assert_eq!(scenarios, vec![spec("clean", "clean", 0)]);
}

#[test]
fn default_scenarios_with_a_manual_check() {
    let mut info = info();
    info.has_manual_check = true;
    assert_eq!(
        default_scenarios(&info),
        vec![
            spec("clean", "clean", 0),
            spec("needs-human", "needs-human", 2)
        ]
    );
}

#[test]
fn default_scenarios_add_a_failing_gate_for_every_gate_shim() {
    assert_eq!(
        default_scenarios(&workflow_b()),
        vec![
            spec("clean", "clean", 0),
            spec("gate-fail-build", "gate-fail=build", 1),
            spec("gate-fail-lint", "gate-fail=lint", 1),
            spec("gate-fail-test", "gate-fail=test", 1),
        ]
    );
}

#[test]
fn default_scenarios_ignore_a_copier_role() {
    let mut info = info();
    info.roles
        .insert("vibe".to_string(), Role::Copy("build".to_string()));
    assert_eq!(default_scenarios(&info), vec![spec("clean", "clean", 0)]);
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
