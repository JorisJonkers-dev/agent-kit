//! The command line, driven with in-memory streams and a fake gold-band.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cli::{run, run_with, Env, REFUSED};
use crate::record::{self, write_record, Outcome, Record, ScenarioResult};
use crate::refuse::{load_workflow, Workflow};

/// A workflow with one worker node and no manual check.
const MINIMAL_WORKFLOW: &str = r#"{
  "workflow": {
    "id": "cli-proof",
    "entry": "only",
    "nodes": [
      {
        "type": "worker",
        "id": "only",
        "executionSlotId": "slot-only",
        "provider": "vibe",
        "model": "mistral-large-4-0",
        "profile": "pf-spec-redteam",
        "goal": "Answer with one fenced json block holding the findings document and nothing else."
      }
    ],
    "edges": [{"from": "only", "to": "$end", "on": "success"}]
  },
  "modelBindings": {
    "bindings": [
      {"executionSlotId": "slot-only", "agentId": "vibe", "modelId": "mistral-large-4-0"}
    ]
  }
}
"#;

/// A fake gold-band that answers the verdict the harness wrote into the home.
const FAKE_VERDICT_SCRIPT: &str = r#"#!/bin/sh
verdict=$(sed -n 's/.*"STUB_ACP_VERDICT": *"\([^"]*\)".*/\1/p' "$GOLD_BAND_HOME/.gold-band/settings.json")
case "$verdict" in
  needs-human)
    printf '{\n  "status": "paused",\n  "outcome": null\n}\n'
    exit 2
    ;;
  *)
    printf '{\n  "status": "completed",\n  "outcome": "success"\n}\n'
    exit 0
    ;;
esac
"#;

/// A fake gold-band that always reports a clean run, whatever the verdict.
const FAKE_ALWAYS_CLEAN: &str = r#"#!/bin/sh
printf '{\n  "status": "completed",\n  "outcome": "success"\n}\n'
exit 0
"#;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let tick = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "gold-band-proof-cli-{}-{}-{}",
        std::process::id(),
        tick,
        label
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn env_for<'a>(get: &'a dyn Fn(&str) -> Option<String>, dir: &Path) -> Env<'a> {
    Env {
        get,
        exe: PathBuf::from("/usr/bin/gold-band-proof"),
        scratch: dir.join("scratch"),
        now: "2026-11-11T16:53:26Z".to_string(),
    }
}

fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

struct Output {
    code: i32,
    out: String,
    err: String,
}

fn drive(args: &[&str], env: &Env, input: &str) -> Output {
    let args: Vec<OsString> = args.iter().map(|arg| OsString::from(*arg)).collect();
    let mut stdin = io::Cursor::new(input.as_bytes().to_vec());
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = run_with(&args, env, &mut stdin, &mut out, &mut err);
    Output {
        code,
        out: String::from_utf8_lossy(&out).into_owned(),
        err: String::from_utf8_lossy(&err).into_owned(),
    }
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).expect("script written");
    let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("script executable");
    path
}

fn workflow_file(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("workflow.json");
    fs::write(&path, body).expect("workflow written");
    path
}

fn scenario(name: &str, expected_exit: i32, exit_code: Option<i32>) -> ScenarioResult {
    ScenarioResult {
        name: name.to_string(),
        expected_exit,
        exit_code,
        run_status: Some("completed".to_string()),
        run_outcome: Some("success".to_string()),
        note: None,
    }
}

fn record_for(workflow: &Workflow, outcome: Outcome) -> Record {
    Record {
        workflow_id: workflow.id.clone(),
        workflow_sha256: workflow.sha256.clone(),
        gold_band_commit: Some("abc1234".to_string()),
        outcome,
        scenarios: vec![scenario("clean", 0, Some(0))],
        skip_reason: None,
        recorded_at: "2026-11-11T16:53:26Z".to_string(),
    }
}

/// A struct whose writes always fail, for the stub's I/O error path.
struct Broken;

impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::Other, "broken pipe"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn no_arguments_prints_the_usage() {
    let dir = temp_dir("usage");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(&[], &env, "");
    assert_eq!(output.code, 0);
    assert!(
        output.out.starts_with("usage: gold-band-proof prove"),
        "{}",
        output.out
    );
    assert!(output.out.contains("gold-band-proof run"), "{}", output.out);
    assert_eq!(output.err, "");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn help_prints_the_usage() {
    let dir = temp_dir("help");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    for flag in ["-h", "--help"] {
        let output = drive(&[flag], &env, "");
        assert_eq!(output.code, 0, "{}", flag);
        assert!(
            output.out.starts_with("usage: gold-band-proof prove"),
            "{}",
            output.out
        );
        assert_eq!(output.err, "", "{}", flag);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_unknown_subcommand_is_a_usage_error() {
    let dir = temp_dir("unknown");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(&["frobnicate"], &env, "");
    assert_eq!(output.code, 2);
    assert_eq!(output.out, "");
    assert!(
        output
            .err
            .contains("gold-band-proof: unknown subcommand `frobnicate`"),
        "{}",
        output.err
    );
    assert!(
        output.err.contains("hint: run `gold-band-proof --help`"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
    let dir = temp_dir("unknownflag");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(&["check", "--workflow", "w.json", "--nope"], &env, "");
    assert_eq!(output.code, 2);
    assert!(
        output
            .err
            .contains("gold-band-proof: unknown flag `--nope`"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_flag_without_a_value_is_a_usage_error() {
    let dir = temp_dir("novalue");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(&["check", "--workflow"], &env, "");
    assert_eq!(output.code, 2);
    assert!(
        output.err.contains("`--workflow` needs a value"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_workflow_is_a_usage_error() {
    let dir = temp_dir("noworkflow");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(&["prove", "--timeout", "5"], &env, "");
    assert_eq!(output.code, 2);
    assert!(
        output.err.contains("`--workflow` is required"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_stray_argument_is_a_usage_error() {
    let dir = temp_dir("stray");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(&["check", "workflow.json"], &env, "");
    assert_eq!(output.code, 2);
    assert!(
        output.err.contains("unexpected argument `workflow.json`"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_timeout_that_is_not_a_positive_number_is_a_usage_error() {
    let dir = temp_dir("timeout");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    for value in ["0", "soon"] {
        let output = drive(
            &["prove", "--workflow", "w.json", "--timeout", value],
            &env,
            "",
        );
        assert_eq!(output.code, 2, "{}", value);
        assert!(
            output
                .err
                .contains("`--timeout` needs a positive number of seconds"),
            "{}",
            output.err
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_extra_arguments_are_only_for_run() {
    let dir = temp_dir("extras");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(&["check", "--workflow", "w.json", "--", "x"], &env, "");
    assert_eq!(output.code, 2);
    assert!(
        output.err.contains("`--` is only for `run`"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_record_dir_comes_from_the_environment() {
    let dir = temp_dir("proofdir");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let vars = vars(&[("GOLD_BAND_PROOF_DIR", proofs.to_str().expect("utf8"))]);
    let get = |name: &str| vars.get(name).cloned();
    let env = env_for(&get, &dir);
    let output = drive(
        &["check", "--workflow", workflow.to_str().expect("utf8")],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    assert_eq!(output.out, "gold-band-proof: ok\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_record_dir_falls_back_to_the_xdg_state_home() {
    let dir = temp_dir("xdg");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let state = dir.join("state");
    let proofs = state.join("agent-kit/gold-band/proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let vars = vars(&[("XDG_STATE_HOME", state.to_str().expect("utf8"))]);
    let get = |name: &str| vars.get(name).cloned();
    let env = env_for(&get, &dir);
    let output = drive(
        &["check", "--workflow", workflow.to_str().expect("utf8")],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_record_dir_falls_back_to_the_home_directory() {
    let dir = temp_dir("home");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let home = dir.join("home");
    let proofs = home.join(".local/state/agent-kit/gold-band/proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let vars = vars(&[("HOME", home.to_str().expect("utf8"))]);
    let get = |name: &str| vars.get(name).cloned();
    let env = env_for(&get, &dir);
    let output = drive(
        &["check", "--workflow", workflow.to_str().expect("utf8")],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_explicit_record_dir_wins_over_the_environment() {
    let dir = temp_dir("explicitdir");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let empty = dir.join("empty");
    fs::create_dir_all(&empty).expect("empty dir");
    let vars = vars(&[("GOLD_BAND_PROOF_DIR", empty.to_str().expect("utf8"))]);
    let get = |name: &str| vars.get(name).cloned();
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "check",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_record_dir_is_a_usage_error() {
    let dir = temp_dir("nodir");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(&["check", "--workflow", "w.json"], &env, "");
    assert_eq!(output.code, 2);
    assert!(output.err.contains("no proof directory"), "{}", output.err);
    assert!(output.err.contains("--record-dir"), "{}", output.err);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_build_record_comes_from_the_state_directory() {
    let dir = temp_dir("buildstate");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let state = dir.join("state");
    let build = state.join("agent-kit/gold-band/build.json");
    fs::create_dir_all(build.parent().expect("parent")).expect("build dir");
    fs::write(&build, "{\"commit\": \"abc1234\"}").expect("build record written");
    let vars = vars(&[("XDG_STATE_HOME", state.to_str().expect("utf8"))]);
    let get = |name: &str| vars.get(name).cloned();
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "check",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_build_record_falls_back_to_the_home_directory() {
    let dir = temp_dir("buildhome");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let home = dir.join("home");
    let build = home.join(".local/state/agent-kit/gold-band/build.json");
    fs::create_dir_all(build.parent().expect("parent")).expect("build dir");
    fs::write(&build, "{\"commit\": \"abc1234\"}").expect("build record written");
    let vars = vars(&[("HOME", home.to_str().expect("utf8"))]);
    let get = |name: &str| vars.get(name).cloned();
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "check",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_explicit_build_record_wins_over_the_environment() {
    let dir = temp_dir("buildexplicit");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let state = dir.join("state");
    let installed = state.join("agent-kit/gold-band/build.json");
    fs::create_dir_all(installed.parent().expect("parent")).expect("build dir");
    fs::write(&installed, "{\"commit\": \"def5678\"}").expect("build record written");
    let chosen = dir.join("chosen.json");
    fs::write(&chosen, "{\"commit\": \"abc1234\"}").expect("chosen record written");
    let vars = vars(&[("XDG_STATE_HOME", state.to_str().expect("utf8"))]);
    let get = |name: &str| vars.get(name).cloned();
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "check",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--build-record",
            chosen.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_build_record_without_a_commit_means_no_commit_check() {
    let dir = temp_dir("buildnone");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let build = dir.join("build.json");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    for body in ["not json", "{\"version\": \"0.19.1\"}", "{\"commit\": 5}"] {
        fs::write(&build, body).expect("build record written");
        let output = drive(
            &[
                "check",
                "--workflow",
                workflow.to_str().expect("utf8"),
                "--record-dir",
                proofs.to_str().expect("utf8"),
                "--build-record",
                build.to_str().expect("utf8"),
            ],
            &env,
            "",
        );
        assert_eq!(output.code, 0, "{}: {}", body, output.err);
    }
    let absent = dir.join("absent.json");
    let output = drive(
        &[
            "check",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--build-record",
            absent.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_proof_from_another_gold_band_build() {
    let dir = temp_dir("buildmismatch");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let build = dir.join("build.json");
    fs::write(&build, "{\"commit\": \"def5678\"}").expect("build record written");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "check",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--build-record",
            build.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, REFUSED);
    assert!(
        output
            .err
            .contains("was proved on Gold Band abc1234, but def5678 is installed"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_workflow_without_a_record() {
    let dir = temp_dir("checknone");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let proofs = dir.join("proofs");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "check",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, REFUSED);
    assert_eq!(output.out, "");
    assert!(
        output
            .err
            .contains("no headless proof is recorded for workflow `cli-proof`"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_skipped_proof() {
    let dir = temp_dir("checkskipped");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    let mut record = record_for(&loaded, Outcome::Skipped);
    record.skip_reason = Some("gold-band is not installed".to_string());
    write_record(&proofs, &record).expect("record written");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "check",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, REFUSED);
    assert!(
        output
            .err
            .contains("was skipped: gold-band is not installed"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_failed_proof() {
    let dir = temp_dir("checkfailed");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    let mut record = record_for(&loaded, Outcome::Failed);
    record.scenarios = vec![scenario("clean", 0, Some(1))];
    write_record(&proofs, &record).expect("record written");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "check",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, REFUSED);
    assert!(
        output
            .err
            .contains("failed: scenario `clean` expected exit 0 but got 1"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_workflow_that_changed_since_its_proof() {
    let dir = temp_dir("checkchanged");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    let mut record = record_for(&loaded, Outcome::Passed);
    record.workflow_sha256 = "0000".to_string();
    write_record(&proofs, &record).expect("record written");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "check",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, REFUSED);
    assert!(
        output
            .err
            .contains("changed since its headless proof was recorded"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_accepts_a_passing_proof() {
    let dir = temp_dir("checkpass");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "check",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    assert_eq!(output.out, "gold-band-proof: ok\n");
    assert_eq!(output.err, "");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_refuses_an_unproven_workflow_without_starting_gold_band() {
    let dir = temp_dir("runrefused");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let proofs = dir.join("proofs");
    let marker = dir.join("started");
    let fake = script(
        &dir,
        "gold-band",
        &format!("#!/bin/sh\necho \"$@\" > {}\nexit 7\n", marker.display()),
    );
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "run",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--gold-band",
            fake.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, REFUSED);
    assert!(
        output.err.contains("no headless proof is recorded"),
        "{}",
        output.err
    );
    assert!(!marker.exists(), "gold-band was started");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_starts_gold_band_with_the_workflow_and_the_extra_arguments() {
    let dir = temp_dir("runallowed");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let args_file = dir.join("args");
    let fake = script(
        &dir,
        "gold-band",
        &format!("#!/bin/sh\necho \"$@\" > {}\nexit 7\n", args_file.display()),
    );
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "run",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--gold-band",
            fake.to_str().expect("utf8"),
            "--",
            "--verbose",
            "extra",
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 7, "{}", output.err);
    let seen = fs::read_to_string(&args_file).expect("arguments read");
    assert_eq!(
        seen.trim(),
        format!(
            "run --workflow {} --verbose extra",
            workflow.to_str().expect("utf8")
        )
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_gold_band_binary_comes_from_the_environment() {
    let dir = temp_dir("runenv");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let args_file = dir.join("args");
    let fake = script(
        &dir,
        "gold-band",
        &format!("#!/bin/sh\necho \"$@\" > {}\nexit 7\n", args_file.display()),
    );
    let vars = vars(&[("GOLD_BAND_BIN", fake.to_str().expect("utf8"))]);
    let get = |name: &str| vars.get(name).cloned();
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "run",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 7, "{}", output.err);
    assert!(args_file.exists(), "the fake was not started");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_explicit_gold_band_wins_over_the_environment() {
    let dir = temp_dir("runexplicit");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let args_file = dir.join("args");
    let fake = script(
        &dir,
        "gold-band",
        &format!("#!/bin/sh\necho \"$@\" > {}\nexit 7\n", args_file.display()),
    );
    let absent = dir.join("absent-gold-band");
    let vars = vars(&[("GOLD_BAND_BIN", absent.to_str().expect("utf8"))]);
    let get = |name: &str| vars.get(name).cloned();
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "run",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--gold-band",
            fake.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 7, "{}", output.err);
    assert!(args_file.exists(), "the fake was not started");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_reports_a_gold_band_that_cannot_be_started() {
    let dir = temp_dir("runabsent");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let absent = dir.join("absent-gold-band");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "run",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--gold-band",
            absent.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 3);
    assert!(
        output.err.contains("gold-band-proof: cannot start"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_reports_a_gold_band_killed_by_a_signal() {
    let dir = temp_dir("runsignal");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let loaded = load_workflow(&workflow).expect("workflow loaded");
    let proofs = dir.join("proofs");
    write_record(&proofs, &record_for(&loaded, Outcome::Passed)).expect("record written");
    let fake = script(&dir, "gold-band", "#!/bin/sh\nkill -TERM $$\n");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "run",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--gold-band",
            fake.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 3, "{}", output.err);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_without_a_binary_records_a_skipped_proof() {
    let dir = temp_dir("proveskip");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let proofs = dir.join("proofs");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "prove",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    assert!(
        output
            .out
            .contains("gold-band-proof: skipped: no Gold Band binary supplied"),
        "{}",
        output.out
    );
    assert!(
        output.out.contains("gold-band-proof: skipped "),
        "{}",
        output.out
    );
    let record = record::read_record(&proofs, "cli-proof")
        .expect("record read")
        .expect("record present");
    assert_eq!(record.outcome, Outcome::Skipped);
    assert!(record.skip_reason.is_some());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_records_the_gold_band_commit_it_was_given() {
    let dir = temp_dir("provecommit");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let proofs = dir.join("proofs");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "prove",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--gold-band-commit",
            "abc1234",
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    let record = record::read_record(&proofs, "cli-proof")
        .expect("record read")
        .expect("record present");
    assert_eq!(record.gold_band_commit.as_deref(), Some("abc1234"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_records_the_installed_gold_band_commit() {
    let dir = temp_dir("proveinstalled");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let proofs = dir.join("proofs");
    let build = dir.join("build.json");
    fs::write(&build, "{\"commit\": \"def5678\"}").expect("build record written");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "prove",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--build-record",
            build.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    let record = record::read_record(&proofs, "cli-proof")
        .expect("record read")
        .expect("record present");
    assert_eq!(record.gold_band_commit.as_deref(), Some("def5678"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_passes_a_workflow_without_a_manual_check() {
    let dir = temp_dir("provepass");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let proofs = dir.join("proofs");
    let fake = script(&dir, "gold-band", FAKE_VERDICT_SCRIPT);
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "prove",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--gold-band",
            fake.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    assert!(
        output
            .out
            .contains("  clean: expected exit 0, got 0 (completed/success)"),
        "{}",
        output.out
    );
    assert!(
        output.out.contains("gold-band-proof: passed "),
        "{}",
        output.out
    );
    let record = record::read_record(&proofs, "cli-proof")
        .expect("record read")
        .expect("record present");
    assert_eq!(record.outcome, Outcome::Passed);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_fails_when_a_scenario_exits_wrongly() {
    let dir = temp_dir("provefail");
    let template = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../installer/gold-band/workflows/critique-spec-stage.json");
    let proofs = dir.join("proofs");
    let fake = script(&dir, "gold-band", FAKE_ALWAYS_CLEAN);
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "prove",
            "--workflow",
            template.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--gold-band",
            fake.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 1, "{}", output.err);
    assert!(
        output.err.contains("gold-band-proof: needs-human:"),
        "{}",
        output.err
    );
    assert!(
        output.out.contains("gold-band-proof: failed "),
        "{}",
        output.out
    );
    let record = record::read_record(&proofs, "critique-spec-stage")
        .expect("record read")
        .expect("record present");
    assert_eq!(record.outcome, Outcome::Failed);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_rejects_a_bad_scenario() {
    let dir = temp_dir("provescenario");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let proofs = dir.join("proofs");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "prove",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--scenario",
            "bogus",
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 2);
    assert!(output.err.contains("gold-band-proof: "), "{}", output.err);
    assert!(
        output.err.contains("hint: run `gold-band-proof --help`"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_accepts_an_extra_scenario_and_a_timeout() {
    let dir = temp_dir("provescenariook");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let proofs = dir.join("proofs");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "prove",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
            "--scenario",
            "extra:clean:0",
            "--timeout",
            "30",
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    let record = record::read_record(&proofs, "cli-proof")
        .expect("record read")
        .expect("record present");
    assert_eq!(record.outcome, Outcome::Skipped);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_reports_an_unwritable_record_dir() {
    let dir = temp_dir("proveunwritable");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let blocker = dir.join("records");
    fs::write(&blocker, "not a directory").expect("blocker written");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "prove",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            blocker.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 2);
    assert!(
        output.err.contains("cannot create the proof directory"),
        "{}",
        output.err
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_reports_a_workflow_it_cannot_read() {
    let dir = temp_dir("proveabsent");
    let absent = dir.join("absent.json");
    let proofs = dir.join("proofs");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "prove",
            "--workflow",
            absent.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 2);
    assert!(output.err.contains("gold-band-proof: "), "{}", output.err);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stub_acp_answers_a_canned_request() {
    let dir = temp_dir("stub");
    let vars = vars(&[
        ("STUB_ACP_VERDICT", "needs-human"),
        ("STUB_ACP_MODELS", "m1,m2"),
    ]);
    let get = |name: &str| vars.get(name).cloned();
    let env = env_for(&get, &dir);
    let request = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/new\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"session/prompt\",\"params\":{\"sessionId\":\"stub-session-1\"}}\n",
    );
    let output = drive(&["stub-acp"], &env, request);
    assert_eq!(output.code, 0, "{}", output.err);
    assert!(
        output.out.contains("\"protocolVersion\":1"),
        "{}",
        output.out
    );
    assert!(output.out.contains("stub-session-1"), "{}", output.out);
    assert!(output.out.contains("stub-needs-human-1"), "{}", output.out);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stub_acp_reports_an_io_error() {
    let dir = temp_dir("stubio");
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    let args = vec![OsString::from("stub-acp")];
    let mut stdin =
        io::Cursor::new(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n".to_vec());
    let mut err: Vec<u8> = Vec::new();
    let code = run_with(&args, &env, &mut stdin, &mut Broken, &mut err);
    assert_eq!(code, 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_wires_the_process_environment() {
    assert_eq!(run(vec![OsString::from("--help")].into_iter()), 0);
    let dir = temp_dir("smoke");
    let args = vec![
        OsString::from("check"),
        OsString::from("--workflow"),
        dir.join("absent.json").into_os_string(),
        OsString::from("--record-dir"),
        dir.join("proofs").into_os_string(),
    ];
    assert_eq!(run(args.into_iter()), REFUSED);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_uses_the_gold_band_binary_the_environment_names() {
    let dir = temp_dir("proveenv");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let proofs = dir.join("proofs");
    let fake = script(&dir, "gold-band", FAKE_ALWAYS_CLEAN);
    let fake_path = fake.to_str().expect("utf8").to_string();
    let get = move |name: &str| (name == "GOLD_BAND_BIN").then(|| fake_path.clone());
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "prove",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    assert!(
        output.out.contains("gold-band-proof: passed "),
        "{}",
        output.out
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_gold_band_variable_names_no_binary() {
    let dir = temp_dir("proveempty");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let proofs = dir.join("proofs");
    let get = |name: &str| (name == "GOLD_BAND_BIN").then(String::new);
    let env = env_for(&get, &dir);
    let output = drive(
        &[
            "prove",
            "--workflow",
            workflow.to_str().expect("utf8"),
            "--record-dir",
            proofs.to_str().expect("utf8"),
        ],
        &env,
        "",
    );
    assert_eq!(output.code, 0, "{}", output.err);
    assert!(
        output
            .out
            .contains("gold-band-proof: skipped: no Gold Band binary supplied"),
        "{}",
        output.out
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prove_and_run_without_a_proof_directory_are_usage_errors() {
    let dir = temp_dir("nodir");
    let workflow = workflow_file(&dir, MINIMAL_WORKFLOW);
    let get = |_: &str| None::<String>;
    let env = env_for(&get, &dir);
    for command in ["prove", "run"] {
        let output = drive(
            &[command, "--workflow", workflow.to_str().expect("utf8")],
            &env,
            "",
        );
        assert_eq!(output.code, 2, "{}", output.err);
        assert!(output.err.contains("no proof directory"), "{}", output.err);
    }
    let _ = fs::remove_dir_all(&dir);
}
