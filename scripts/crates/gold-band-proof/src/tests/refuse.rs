use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::record::{record_path, write_record, Outcome, Record, ScenarioResult};
use crate::refuse::{check, load_workflow, Workflow};
use crate::sha256;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let tick = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "gold-band-proof-refuse-{}-{}-{}",
        std::process::id(),
        tick,
        label
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
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
        scenarios: vec![scenario("happy path", 0, Some(0))],
        skip_reason: None,
        recorded_at: "2026-11-11T16:53:26Z".to_string(),
    }
}

#[test]
fn load_workflow_reads_the_authoring_wrapper_id() {
    let dir = temp_dir("wrapper");
    let path = workflow_file(&dir, "{\"workflow\":{\"id\":\"release\"},\"id\":\"other\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    assert_eq!(workflow.id, "release");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn load_workflow_falls_back_to_the_top_level_id() {
    let dir = temp_dir("top");
    let path = workflow_file(&dir, "{\"id\":\"release\",\"steps\":[]}");
    assert_eq!(load_workflow(&path).expect("workflow loaded").id, "release");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn load_workflow_hashes_the_file_bytes() {
    let dir = temp_dir("hash");
    let body = "{\"id\":\"release\"}\n";
    let path = workflow_file(&dir, body);
    let workflow = load_workflow(&path).expect("workflow loaded");
    assert_eq!(workflow.sha256, sha256::hex(body.as_bytes()));
    assert_eq!(workflow.sha256.len(), 64);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn load_workflow_fails_when_the_file_is_missing() {
    let dir = temp_dir("absent");
    let path = dir.join("absent.json");
    let message = load_workflow(&path).expect_err("refused");
    assert!(message.contains("cannot read the workflow"), "{}", message);
    assert!(message.contains("absent.json"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn load_workflow_fails_when_the_file_is_not_json() {
    let dir = temp_dir("notjson");
    let path = workflow_file(&dir, "not json");
    let message = load_workflow(&path).expect_err("refused");
    assert!(message.contains("is not JSON"), "{}", message);
    assert!(message.contains("workflow.json"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn load_workflow_fails_when_the_file_is_not_utf8() {
    let dir = temp_dir("notutf8");
    let path = dir.join("workflow.json");
    fs::write(&path, [0xff, 0xfe, 0x00]).expect("workflow written");
    let message = load_workflow(&path).expect_err("refused");
    assert!(message.contains("is not UTF-8 text"), "{}", message);
    assert!(message.contains("workflow.json"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn load_workflow_fails_when_there_is_no_id() {
    let dir = temp_dir("noid");
    let path = workflow_file(&dir, "{\"workflow\":{},\"id\":5}");
    let message = load_workflow(&path).expect_err("refused");
    assert!(message.contains("has no id"), "{}", message);
    assert!(message.contains("workflow.json"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_accepts_a_passing_proof() {
    let dir = temp_dir("pass");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    write_record(&dir, &record_for(&workflow, Outcome::Passed)).expect("record written");
    assert_eq!(check(&dir, &path, Some("abc1234")), Ok(()));
    assert_eq!(check(&dir, &path, None), Ok(()));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_when_no_record_exists() {
    let dir = temp_dir("norecord");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    assert_eq!(
        check(&dir, &path, None),
        Err(
            "refused: no headless proof is recorded for workflow `release`; run `gold-band-proof prove` first"
                .to_string()
        )
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_skipped_proof_with_its_reason() {
    let dir = temp_dir("skipped");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    let mut record = record_for(&workflow, Outcome::Skipped);
    record.skip_reason = Some("gold-band is not installed".to_string());
    write_record(&dir, &record).expect("record written");
    assert_eq!(
        check(&dir, &path, None),
        Err(
            "refused: the headless proof of workflow `release` was skipped: gold-band is not installed"
                .to_string()
        )
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_skipped_proof_without_a_reason() {
    let dir = temp_dir("skippedbare");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    write_record(&dir, &record_for(&workflow, Outcome::Skipped)).expect("record written");
    assert_eq!(
        check(&dir, &path, None),
        Err(
            "refused: the headless proof of workflow `release` was skipped: no reason recorded"
                .to_string()
        )
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_failed_proof_with_the_failing_scenario() {
    let dir = temp_dir("failed");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    let mut record = record_for(&workflow, Outcome::Failed);
    record.scenarios = vec![
        scenario("happy path", 0, Some(0)),
        scenario("paused run", 2, Some(1)),
    ];
    write_record(&dir, &record).expect("record written");
    assert_eq!(
        check(&dir, &path, None),
        Err(
            "refused: the headless proof of workflow `release` failed: scenario `paused run` expected exit 2 but got 1"
                .to_string()
        )
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_failed_proof_whose_scenario_never_ran() {
    let dir = temp_dir("failednone");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    let mut record = record_for(&workflow, Outcome::Failed);
    record.scenarios = vec![scenario("happy path", 0, None)];
    write_record(&dir, &record).expect("record written");
    assert_eq!(
        check(&dir, &path, None),
        Err(
            "refused: the headless proof of workflow `release` failed: scenario `happy path` expected exit 0 but got none"
                .to_string()
        )
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_failed_proof_without_a_failing_scenario() {
    let dir = temp_dir("failedempty");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    let mut record = record_for(&workflow, Outcome::Failed);
    record.scenarios.clear();
    write_record(&dir, &record).expect("record written");
    assert_eq!(
        check(&dir, &path, None),
        Err(
            "refused: the headless proof of workflow `release` failed: no scenario recorded a failing exit"
                .to_string()
        )
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_workflow_that_changed_since_its_proof() {
    let dir = temp_dir("changed");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    let mut record = record_for(&workflow, Outcome::Passed);
    record.workflow_sha256 = "0000".to_string();
    write_record(&dir, &record).expect("record written");
    assert_eq!(
        check(&dir, &path, None),
        Err(
            "refused: workflow `release` changed since its headless proof was recorded".to_string()
        )
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_proof_from_another_gold_band_build() {
    let dir = temp_dir("otherbuild");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    write_record(&dir, &record_for(&workflow, Outcome::Passed)).expect("record written");
    assert_eq!(
        check(&dir, &path, Some("def5678")),
        Err(
            "refused: workflow `release` was proved on Gold Band abc1234, but def5678 is installed; prove it again"
                .to_string()
        )
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_proof_from_an_unknown_gold_band_build() {
    let dir = temp_dir("unknownbuild");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    let mut record = record_for(&workflow, Outcome::Passed);
    record.gold_band_commit = None;
    write_record(&dir, &record).expect("record written");
    assert_eq!(
        check(&dir, &path, Some("def5678")),
        Err(
            "refused: workflow `release` was proved on Gold Band an unknown build, but def5678 is installed; prove it again"
                .to_string()
        )
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_passing_record_without_scenarios() {
    let dir = temp_dir("noscenarios");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    let mut record = record_for(&workflow, Outcome::Passed);
    record.scenarios.clear();
    write_record(&dir, &record).expect("record written");
    assert_eq!(
        check(&dir, &path, None),
        Err("refused: the proof record of workflow `release` is not a passing proof".to_string())
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_passing_record_with_a_failing_scenario() {
    let dir = temp_dir("handedited");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let workflow = load_workflow(&path).expect("workflow loaded");
    let mut record = record_for(&workflow, Outcome::Passed);
    record.scenarios = vec![scenario("happy path", 0, Some(3))];
    write_record(&dir, &record).expect("record written");
    assert_eq!(
        check(&dir, &path, None),
        Err("refused: the proof record of workflow `release` is not a passing proof".to_string())
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_a_corrupt_record() {
    let dir = temp_dir("corrupt");
    let path = workflow_file(&dir, "{\"id\":\"release\"}");
    let target = record_path(&dir, "release");
    fs::write(&target, b"{\"workflow_id\":").expect("corrupt record written");
    let message = check(&dir, &path, None).expect_err("refused");
    assert!(message.starts_with("refused: "), "{}", message);
    assert!(message.contains("release.proof.json"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_when_the_workflow_cannot_be_read() {
    let dir = temp_dir("noworkflow");
    let path = dir.join("absent.json");
    let message = check(&dir, &path, None).expect_err("refused");
    assert!(message.starts_with("refused: "), "{}", message);
    assert!(message.contains("absent.json"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}
