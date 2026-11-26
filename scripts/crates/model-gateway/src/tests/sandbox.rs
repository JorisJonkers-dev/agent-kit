//! The sandbox backend a launch runs in, and the sbx plan a dry run turns a confined launch into.

use crate::json::{self, Value};
use crate::ports::ProcessCommand;
use crate::sandbox::{dry_run, Mount, SandboxBackend, SbxPlan, SANDBOX_VARIABLE};
use std::collections::HashMap;

fn env(value: Option<&str>) -> HashMap<String, String> {
    value.map(|value| HashMap::from([(SANDBOX_VARIABLE.to_string(), value.to_string())])).unwrap_or_default()
}

fn confined(grants: &[&str]) -> ProcessCommand {
    let mut args = vec!["confine"];
    args.extend(grants);
    args.extend(["--", "opencode", "run", "say ok"]);
    ProcessCommand { cwd: Some("/work".to_string()), ..ProcessCommand::new("/bin/agent-launcher", &args) }
}

#[test]
fn confine_is_the_backend_unless_the_machine_names_another_and_an_unknown_name_is_refused() {
    assert_eq!(SandboxBackend::select(&env(None)), Ok(SandboxBackend::Confine));
    assert_eq!(SandboxBackend::select(&env(Some(""))), Ok(SandboxBackend::Confine));
    assert_eq!(SandboxBackend::select(&env(Some("confine"))), Ok(SandboxBackend::Confine));
    assert_eq!(SandboxBackend::select(&env(Some("sbx-dry-run"))), Ok(SandboxBackend::SbxDryRun));
    let refused = SandboxBackend::select(&env(Some("sbx"))).expect_err("not a backend yet");
    assert_eq!(refused.code, "sandbox-unknown");
    assert!(refused.message.contains("AGENT_KIT_SANDBOX=sbx names no sandbox backend"), "{}", refused.message);
    assert_eq!(SandboxBackend::Confine.as_str(), "confine");
}

#[test]
fn a_read_only_launch_mounts_its_grants_read_only_and_its_run_directory_written() {
    let command = confined(&["--read", "/work", "--read", "/repo/.git/worktrees/w", "--run-from", "opencode", "--run", "/opt/git", "--write", "/run/export", "--net-connect", "443", "--net-loopback", "--env", "HOME"]);
    let plan = SbxPlan::of_command("opencode", &command).expect("a confined launch");
    assert_eq!(plan.template, "agent-kit/opencode");
    assert_eq!(plan.workdir, "/work");
    assert_eq!(plan.mounts.iter().map(Mount::arg).collect::<Vec<_>>(), ["/work:ro", "/repo/.git/worktrees/w:ro", "/run/export"]);
    assert_eq!(plan.programs, ["opencode", "/opt/git"]);
    assert_eq!(plan.env, ["HOME"]);
    assert_eq!(plan.ports, [443]);
    assert!(plan.loopback);
    // The port rule has no host names yet: that is said, not dropped.
    assert_eq!(plan.gaps.len(), 1);
    assert!(plan.gaps[0].starts_with("network: confine allows TCP ports 443 to any host"), "{}", plan.gaps[0]);
}

#[test]
fn a_path_granted_both_ways_is_written_once_and_the_keychain_is_a_gap() {
    let command = confined(&["--read", "/work", "--write", "/work", "--read", "/work", "--keychain", "--preferences"]);
    let plan = SbxPlan::of_command("codex", &command).expect("a confined launch");
    assert_eq!(plan.mounts, [Mount { path: "/work".to_string(), read_only: false }]);
    assert_eq!(plan.ports, Vec::<u16>::new());
    assert_eq!(plan.gaps.len(), 1);
    assert!(plan.gaps[0].starts_with("keychain:"), "{}", plan.gaps[0]);
}

#[test]
fn a_grant_the_plan_does_not_know_and_a_port_that_is_not_one_are_gaps() {
    let command = confined(&["--net-connect", "https", "--mystery", "x", "--env"]);
    let plan = SbxPlan::of_command("pi", &command).expect("a confined launch");
    assert_eq!(plan.gaps, ["--net-connect https is not a port", "--mystery x: a grant the plan does not know"]);
    // A flag cut off at the end takes an empty value rather than the `--` after it.
    assert_eq!(plan.env, [""]);
}

#[test]
fn a_launch_not_behind_the_launcher_has_no_plan() {
    assert_eq!(SbxPlan::of_command("claude", &ProcessCommand::new("claude", &["-p"])), None);
    assert_eq!(SbxPlan::of_command("claude", &ProcessCommand::new("claude", &[])), None);
    // `confine` with no `--` is not a launch the launcher would take.
    assert_eq!(SbxPlan::of_command("claude", &ProcessCommand::new("/bin/agent-launcher", &["confine", "--read", "/work"])), None);
    let refusal = dry_run("claude", &ProcessCommand::new("claude", &["-p"]));
    assert_eq!(refusal.code, "sandbox-dry-run");
    assert!(refusal.message.ends_with("there are no grants to plan from"), "{}", refusal.message);
}

#[test]
fn a_dry_run_refuses_with_the_plan_as_json_and_never_a_value() {
    let refusal = dry_run("opencode", &confined(&["--write", "/work", "--net-connect", "4000", "--env", "LITELLM_KEY"]));
    assert_eq!(refusal.code, "sandbox-dry-run");
    let (said, plan) = refusal.message.split_once("the sbx plan: ").expect("the plan");
    assert_eq!(said, "opencode: not started (AGENT_KIT_SANDBOX=sbx-dry-run); ");
    let plan = json::parse(plan).expect("JSON");
    let object = plan.as_object().expect("an object");
    assert_eq!(object.get("backend").and_then(|value| value.as_str()), Some("sbx-dry-run"));
    assert_eq!(object.get("mounts"), Some(&json::parse(r#"["/work"]"#).expect("JSON")));
    assert_eq!(object.get("env"), Some(&json::parse(r#"["LITELLM_KEY"]"#).expect("JSON")));
    assert_eq!(object.get("ports"), Some(&json::parse("[4000]").expect("JSON")));
    assert_eq!(object.get("loopback"), Some(&json::parse("false").expect("JSON")));
}

#[test]
fn a_path_granted_read_only_twice_stays_read_only() {
    let command = confined(&["--read", "/work", "--read", "/work"]);
    let plan = SbxPlan::of_command("pi", &command).expect("a confined launch");
    assert_eq!(plan.mounts, [Mount { path: "/work".to_string(), read_only: true }]);
    assert_eq!(plan.mounts[0].arg(), "/work:ro");
}

#[test]
fn the_sbx_backend_is_named_sbx_dry_run_in_the_plan() {
    assert_eq!(SandboxBackend::SbxDryRun.as_str(), "sbx-dry-run");
}

#[test]
fn the_plan_json_carries_every_field_with_the_grants_it_was_built_from() {
    let command = confined(&["--read", "/work", "--write", "/run/export", "--run-from", "opencode", "--run", "/opt/git", "--env", "HOME", "--net-connect", "443", "--net-connect", "https", "--net-loopback", "--keychain"]);
    let plan = SbxPlan::of_command("opencode", &command).expect("a confined launch");
    let json = plan.to_json();
    let object = json.as_object().expect("an object");
    let text = |value: &str| Value::str(value);
    let list = |items: &[&str]| Value::Arr(items.iter().map(|item| Value::str(item)).collect());
    assert_eq!(object.get("backend"), Some(&text("sbx-dry-run")));
    assert_eq!(object.get("client"), Some(&text("opencode")));
    assert_eq!(object.get("template"), Some(&text("agent-kit/opencode")));
    assert_eq!(object.get("workdir"), Some(&text("/work")));
    assert_eq!(object.get("mounts"), Some(&list(&["/work:ro", "/run/export"])));
    assert_eq!(object.get("programs"), Some(&list(&["opencode", "/opt/git"])));
    assert_eq!(object.get("env"), Some(&list(&["HOME"])));
    assert_eq!(object.get("ports"), Some(&Value::Arr(vec![Value::Num(443.0)])));
    assert_eq!(object.get("loopback"), Some(&Value::Bool(true)));
    // The https grant is a gap, the keychain is a gap, and the port rule is a gap: three, in grant order.
    assert_eq!(object.get("gaps"), Some(&Value::Arr(plan.gaps.iter().map(|gap| Value::str(gap)).collect())));
    assert_eq!(plan.gaps.len(), 3);
    assert_eq!(plan.gaps[0], "--net-connect https is not a port");
    assert!(plan.gaps[1].starts_with("keychain:"), "{}", plan.gaps[1]);
    assert!(plan.gaps[2].starts_with("network: confine allows TCP ports 443 to any host"), "{}", plan.gaps[2]);
}
