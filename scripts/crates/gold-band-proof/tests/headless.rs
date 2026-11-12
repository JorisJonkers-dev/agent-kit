//! The headless proof against a real Gold Band binary (issue #447).
//!
//! These tests run the built `gold-band-proof` over the checked-in spec-stage
//! template and the fails-closed fixture, with the real `gold-band` named by
//! `GOLD_BAND_BIN`. Without it they print why and pass: a machine without
//! Gold Band proves nothing, and `check` still refuses (the last test).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const HARNESS: &str = env!("CARGO_BIN_EXE_gold-band-proof");

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn template() -> PathBuf {
    manifest_dir().join("../../../installer/gold-band/workflows/critique-spec-stage.json")
}

fn workflow_b() -> PathBuf {
    manifest_dir().join("../../../installer/gold-band/workflows/workflow-b.json")
}

fn workflow_c() -> PathBuf {
    manifest_dir().join("../../../installer/gold-band/workflows/workflow-c.json")
}

fn fixture() -> PathBuf {
    manifest_dir().join("tests/fixtures/fails-closed.json")
}

/// A fresh directory for one test's records.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gold-band-proof-it-{}-{}",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// The real Gold Band binary, or `None` with the reason printed.
fn gold_band() -> Option<String> {
    match std::env::var("GOLD_BAND_BIN") {
        Ok(path) if !path.is_empty() => Some(path),
        _ => {
            println!("skipped: GOLD_BAND_BIN is not set; no Gold Band binary was supplied");
            None
        }
    }
}

fn harness(args: &[&str], gold_band: Option<&str>) -> Output {
    let mut command = Command::new(HARNESS);
    command.args(args);
    match gold_band {
        Some(path) => command.env("GOLD_BAND_BIN", path),
        None => command.env_remove("GOLD_BAND_BIN"),
    };
    command.output().expect("the harness starts")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn prove(workflow: &Path, records: &Path, extra: &[&str], gold_band: &str) -> Output {
    let mut args = vec![
        "prove",
        "--workflow",
        workflow.to_str().expect("utf8"),
        "--record-dir",
        records.to_str().expect("utf8"),
    ];
    args.extend_from_slice(extra);
    harness(&args, Some(gold_band))
}

#[test]
fn the_spec_stage_template_ends_with_exit_0_and_pauses_with_exit_2() {
    let Some(gold_band) = gold_band() else { return };
    let records = scratch("template");
    let output = prove(&template(), &records, &[], &gold_band);
    let stdout = text(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{stdout}\n{}",
        text(&output.stderr)
    );
    assert!(
        stdout.contains("clean: expected exit 0, got 0 (completed/success)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("needs-human: expected exit 2, got 2 (paused/"),
        "{stdout}"
    );
    assert!(stdout.contains("gold-band-proof: passed"), "{stdout}");
}

#[test]
fn workflow_b_repairs_twice_then_escalates_to_a_manual_check_and_fails_closed_on_a_gate() {
    let Some(gold_band) = gold_band() else { return };
    let records = scratch("workflow-b");
    let output = prove(&workflow_b(), &records, &[], &gold_band);
    let stdout = text(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{stdout}\n{}",
        text(&output.stderr)
    );
    assert!(
        stdout.contains("clean: expected exit 0, got 0 (completed/success)"),
        "{stdout}"
    );
    // A repair that clears the findings ends the run in success.
    for round in ["actionable-1", "actionable-2"] {
        assert!(
            stdout.contains(&format!(
                "{round}: expected exit 0, got 0 (completed/success)"
            )),
            "{stdout}"
        );
    }
    // The second failed repair reaches the escalation node and pauses (exit 2);
    // the run never ends on the control limit.
    assert!(
        stdout.contains("actionable-3: expected exit 2, got 2 (paused/none)"),
        "{stdout}"
    );
    for gate in ["build", "test", "lint", "coverage", "findings-1"] {
        assert!(
            stdout.contains(&format!(
                "gate-fail-{gate}: expected exit 1, got 1 (completed/failure)"
            )),
            "{stdout}"
        );
    }
    assert!(stdout.contains("gold-band-proof: passed"), "{stdout}");
}

#[test]
fn workflow_c_pauses_at_each_grill_round_and_finding_and_publishes_only_after_the_check() {
    let Some(gold_band) = gold_band() else { return };
    let records = scratch("workflow-c");
    let output = prove(&workflow_c(), &records, &[], &gold_band);
    let stdout = text(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{stdout}\n{}",
        text(&output.stderr)
    );
    // Every pause is a manual check: the grill rounds, the needs-human
    // findings and the publication check.
    for scenario in [
        "clean",
        "needs-human",
        "open-questions",
        "after-grill-check-1.open-questions",
        "after-grill-check-2.open-questions",
        "after-spec-needs-human.needs-human",
    ] {
        assert!(
            stdout.contains(&format!("{scenario}: expected exit 2, got 2 (paused/")),
            "{scenario}: {stdout}"
        );
    }
    // The publish gate is reachable only after publication-check.
    assert!(
        stdout
            .contains("after-publication-check.clean: expected exit 0, got 0 (completed/success)"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "after-publication-check.gate-fail-workflow-c-publish: expected exit 1, got 1 (completed/failure)"
        ),
        "{stdout}"
    );
    assert!(
        !stdout.contains("\n  gate-fail-workflow-c-publish:"),
        "the publish gate is not reachable before the check: {stdout}"
    );
    assert!(stdout.contains("gold-band-proof: passed"), "{stdout}");
}

/// The Wave 4 human-gate workflows (issue #628): the workflow, its round-check
/// prefix, its round cap and the scenarios that must end in the expected state.
/// Every round's manual check is reached by the first segment's
/// `open-questions` scenario or by the one after the check before it; the last
/// check's segment ends the run.
const WAVE4: [(&str, &str, u32); 5] = [
    ("grill-me", "grill-check-", 5),
    ("grill-with-docs", "grill-check-", 5),
    ("brainstorming", "explore-check-", 4),
    ("to-spec", "spec-check-", 3),
    ("to-tickets", "tickets-check-", 3),
];

#[test]
fn each_wave4_workflow_pauses_at_every_round_check_and_ends_once_answered() {
    let Some(gold_band) = gold_band() else { return };
    for (name, prefix, cap) in WAVE4 {
        let workflow = manifest_dir().join(format!(
            "../../../installer/gold-band/workflows/{name}.json"
        ));
        let records = scratch(&format!("wave4-{name}"));
        let output = prove(&workflow, &records, &[], &gold_band);
        let stdout = text(&output.stdout);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{name}: {stdout}\n{}",
            text(&output.stderr)
        );
        // Round 1's check pauses the first segment; round n+1's pauses the segment after check n.
        assert!(
            stdout.contains("open-questions: expected exit 2, got 2 (paused/"),
            "{name}: {stdout}"
        );
        for round in 1..cap {
            assert!(
                stdout.contains(&format!(
                    "after-{prefix}{round}.open-questions: expected exit 2, got 2 (paused/"
                )),
                "{name} {round}: {stdout}"
            );
        }
        // Once the last check is answered the run ends (or, for brainstorming, pauses at the design approval).
        let after_last = format!("after-{prefix}{cap}.clean: expected exit ");
        assert!(stdout.contains(&after_last), "{name}: {stdout}");
        assert!(
            stdout.contains("gold-band-proof: passed"),
            "{name}: {stdout}"
        );
    }
}

#[test]
fn brainstorming_records_the_design_only_after_the_approval_check() {
    let Some(gold_band) = gold_band() else { return };
    let records = scratch("wave4-brainstorming-design");
    let workflow = manifest_dir().join("../../../installer/gold-band/workflows/brainstorming.json");
    let stdout = text(&prove(&workflow, &records, &[], &gold_band).stdout);
    assert!(
        stdout.contains("after-explore-check-4.clean: expected exit 2, got 2 (paused/"),
        "{stdout}"
    );
    assert!(
        stdout.contains("after-design-approval.clean: expected exit 0, got 0 (completed/success)"),
        "{stdout}"
    );
}

#[test]
fn pr_composer_pauses_at_every_confirmation_and_creates_only_after_one() {
    let Some(gold_band) = gold_band() else { return };
    let records = scratch("pr-composer");
    let workflow = manifest_dir().join("../../../installer/gold-band/workflows/pr-composer.json");
    let output = prove(&workflow, &records, &[], &gold_band);
    let stdout = text(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{stdout}\n{}",
        text(&output.stderr)
    );
    // The first segment pauses at the first confirmation check, never reaching the publish gate.
    for scenario in [
        "clean",
        "open-questions",
        "after-pr-links-check.clean",
        "after-pr-confirm-1.open-questions",
        "after-pr-confirm-2.open-questions",
    ] {
        assert!(
            stdout.contains(&format!("{scenario}: expected exit 2, got 2 (paused/")),
            "{scenario}: {stdout}"
        );
    }
    // The push-and-create gate runs only in a segment that starts after a confirmation check.
    for round in 1..=3 {
        assert!(
            stdout.contains(&format!(
                "after-pr-confirm-{round}.clean: expected exit 0, got 0 (completed/success)"
            )),
            "{round}: {stdout}"
        );
    }
    assert!(
        stdout.contains("after-pr-confirm-1.gate-fail-pr-composer-publish: expected exit 1, got 1 (completed/failure)"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("\n  gate-fail-pr-composer-publish:"),
        "the publish gate is reachable before a confirmation: {stdout}"
    );
    assert!(stdout.contains("gold-band-proof: passed"), "{stdout}");
}

/// The critique stage templates over the runner (issue #458).
const CRITIQUE_STAGES: [&str; 8] = [
    "decisions",
    "spec",
    "tickets",
    "ticket-review",
    "feature-review",
    "adr-review",
    "decision-recovery",
    "architecture-review",
];

#[test]
fn every_critique_stage_template_ends_in_its_expected_state() {
    let Some(gold_band) = gold_band() else { return };
    for stage in CRITIQUE_STAGES {
        let workflow = manifest_dir().join(format!(
            "../../../installer/gold-band/workflows/critique-{stage}.json"
        ));
        let records = scratch(&format!("critique-{stage}"));
        let output = prove(&workflow, &records, &[], &gold_band);
        let stdout = text(&output.stdout);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{stage}: {stdout}\n{}",
            text(&output.stderr)
        );
        assert!(
            stdout.contains("clean: expected exit 0, got 0 (completed/success)"),
            "{stage}: {stdout}"
        );
        assert!(
            stdout.contains("needs-human: expected exit 2, got 2 (paused/"),
            "{stage}: {stdout}"
        );
        // A refused run-start (the allowlist refusal) and a refused runner
        // command each end the run in failure, as does every other gate.
        for step in ["start", "prepare", "fan-out", "refute", "record"] {
            assert!(
                stdout.contains(&format!(
                    "gate-fail-critique-{stage}-{step}: expected exit 1, got 1 (completed/failure)"
                )),
                "{stage} {step}: {stdout}"
            );
        }
        assert!(
            stdout.contains("gold-band-proof: passed"),
            "{stage}: {stdout}"
        );
    }
}

#[test]
fn a_run_that_expects_success_from_a_failing_gate_fails_the_proof() {
    let Some(gold_band) = gold_band() else { return };
    let records = scratch("workflow-b-wrong");
    let output = prove(
        &workflow_b(),
        &records,
        &["--scenario", "gate-fail-test:gate-fail=test:0"],
        &gold_band,
    );
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stdout));
    assert!(
        text(&output.stderr).contains("gate-fail-test: expected exit 0, got 1"),
        "{}",
        text(&output.stderr)
    );
}

#[test]
fn a_failing_run_ends_with_exit_1() {
    let Some(gold_band) = gold_band() else { return };
    let records = scratch("fixture");
    let output = prove(
        &fixture(),
        &records,
        &["--scenario", "fails:needs-human:1"],
        &gold_band,
    );
    let stdout = text(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{stdout}\n{}",
        text(&output.stderr)
    );
    assert!(
        stdout.contains("fails: expected exit 1, got 1 (completed/failure)"),
        "{stdout}"
    );
}

#[test]
fn a_scenario_that_exits_differently_than_expected_fails_the_proof() {
    let Some(gold_band) = gold_band() else { return };
    let records = scratch("wrong");
    let output = prove(
        &fixture(),
        &records,
        &["--scenario", "fails:needs-human:0"],
        &gold_band,
    );
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stdout));
    let refused = harness(
        &[
            "check",
            "--workflow",
            fixture().to_str().expect("utf8"),
            "--record-dir",
            records.to_str().expect("utf8"),
        ],
        None,
    );
    assert_eq!(refused.status.code(), Some(5));
    assert!(
        text(&refused.stderr).contains("failed"),
        "{}",
        text(&refused.stderr)
    );
}

#[test]
fn a_passing_proof_allows_a_real_run_and_a_changed_workflow_is_refused() {
    let Some(gold_band) = gold_band() else { return };
    let records = scratch("allowed");
    let proved = prove(&template(), &records, &[], &gold_band);
    assert_eq!(proved.status.code(), Some(0), "{}", text(&proved.stdout));
    let workflow = template();
    let args = [
        "check",
        "--workflow",
        workflow.to_str().expect("utf8"),
        "--record-dir",
        records.to_str().expect("utf8"),
    ];
    assert_eq!(harness(&args, None).status.code(), Some(0));
    // The same workflow content under another path is the same proof; other content is not.
    let changed = records.join("changed.json");
    let original = std::fs::read_to_string(template()).expect("template");
    std::fs::write(&changed, original.replace("0.1", "0.1 ")).expect("write");
    let refused = harness(
        &[
            "check",
            "--workflow",
            changed.to_str().expect("utf8"),
            "--record-dir",
            records.to_str().expect("utf8"),
        ],
        None,
    );
    assert_eq!(refused.status.code(), Some(5), "{}", text(&refused.stderr));
}

#[test]
fn without_a_binary_the_proof_is_skipped_with_its_reason_and_a_real_run_is_refused() {
    let records = scratch("skipped");
    let workflow = template();
    let args = [
        "prove",
        "--workflow",
        workflow.to_str().expect("utf8"),
        "--record-dir",
        records.to_str().expect("utf8"),
    ];
    let output = harness(&args, None);
    let stdout = text(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("gold-band-proof: skipped: no Gold Band binary supplied"),
        "{stdout}"
    );
    let refused = harness(
        &[
            "run",
            "--workflow",
            template().to_str().expect("utf8"),
            "--record-dir",
            records.to_str().expect("utf8"),
        ],
        None,
    );
    assert_eq!(refused.status.code(), Some(5));
    assert!(
        text(&refused.stderr).contains("was skipped"),
        "{}",
        text(&refused.stderr)
    );
}

#[test]
fn every_checked_in_workflow_passes_the_workflow_lint() {
    let dir = manifest_dir().join("../../../installer/gold-band/workflows");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("the workflows directory")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert!(!files.is_empty());
    let mut args = vec!["--instance".to_string(), "gold-band-personal".to_string()];
    args.extend(files);
    assert_eq!(workflow_lint::run(args.into_iter().map(Into::into)), 0);
}
