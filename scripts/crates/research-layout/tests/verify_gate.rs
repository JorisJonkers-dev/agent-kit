//! Workflow A's verifier behind the gate shim (issue #456): the built
//! `research-layout verify` run through `gold-band-gate run`, and its report
//! read back by `gold-band-gate verify`, the copier. A run whose quote is on
//! its fetched page passes both; a quote that is not on its fetched page
//! fails the gate and the copier refuses the report, so the run ends before
//! the acceptance node.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_research-layout");
const PAGE: &str = "Retries back off exponentially.\nJitter spreads them out.\n";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.org"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A research repository on its base commit, the fetch shim's output for one
/// page, and the run's merge with one note quoting `quote` from that page.
fn run_dir(tag: &str, quote: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("research-gate-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let papers = root.join("repo/papers");
    fs::create_dir_all(papers.join("notes")).unwrap();
    fs::write(papers.join("catalog.json"), "[]").unwrap();
    fs::write(papers.join("README.md"), "# Papers\n").unwrap();
    fs::write(papers.join("notes/README.md"), "# Notes\n").unwrap();
    assert!(Command::new(BIN)
        .arg("index")
        .arg(&papers)
        .status()
        .unwrap()
        .success());
    git(&root.join("repo"), &["init", "-q", "-b", "main"]);
    git(&root.join("repo"), &["add", "-A"]);
    git(&root.join("repo"), &["commit", "-q", "-m", "base"]);
    let fetched = root.join("pages");
    fs::create_dir_all(&fetched).unwrap();
    fs::write(fetched.join("retries.txt"), PAGE).unwrap();
    let sha = gold_band_report::sha256_hex(PAGE.as_bytes());
    fs::write(
        fetched.join("fetch-log.json"),
        format!(r#"{{"sources":[{{"id":"retries","url":"https://example.org/r","file":"retries.txt","sha256":"{sha}"}}]}}"#),
    )
    .unwrap();
    fs::create_dir_all(papers.join("resilience/retries")).unwrap();
    fs::write(papers.join("resilience/retries/retries.txt"), PAGE).unwrap();
    fs::write(papers.join("resilience/retries/README.md"), "# Retries\n").unwrap();
    fs::write(
        papers.join("catalog.json"),
        r#"[{"id":"retries","title":"Retries","theme":"resilience/retries","tier":"C","files":["resilience/retries/retries.txt"],"url":"https://example.org/r"}]"#,
    )
    .unwrap();
    fs::write(
        papers.join("notes/2026-11-14-retries.md"),
        format!("---\nmin_tier: C\ncites: [retries]\n---\n\n> {quote}\n\n-- resilience/retries/retries.txt\n"),
    )
    .unwrap();
    assert!(Command::new(BIN)
        .arg("index")
        .arg(&papers)
        .status()
        .unwrap()
        .success());
    (papers, fetched, root.join("reports"))
}

fn gate(args: Vec<String>) -> i32 {
    gold_band_gate::run(args.into_iter().map(OsString::from))
}

/// Runs the verifier through the gate wrapper, then the copier: both exit codes.
fn verify_through_the_gate(papers: &Path, fetched: &Path, reports: &Path) -> (i32, i32) {
    let command = [
        BIN.to_string(),
        "verify".to_string(),
        "--papers".to_string(),
        papers.display().to_string(),
        "--fetched".to_string(),
        fetched.display().to_string(),
        "--base".to_string(),
        "main".to_string(),
    ];
    let report_dir = reports.display().to_string();
    let mut run = vec![
        "run",
        "--run-id",
        "proof-run",
        "--attempt",
        "1",
        "--timeout-secs",
        "60",
        "--report-dir",
        &report_dir,
        "--",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    run.extend(command.iter().cloned());
    let gate_exit = gate(run);
    let copy_exit = gate(
        [
            "verify",
            "--run-id",
            "proof-run",
            "--attempt",
            "1",
            "--command",
            &command.join(" "),
            "--report-dir",
            &report_dir,
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
    );
    (gate_exit, copy_exit)
}

#[test]
fn a_quote_on_its_fetched_page_passes_the_gate_and_the_copier() {
    let (papers, fetched, reports) = run_dir("pass", "Retries back off exponentially.");
    assert_eq!(verify_through_the_gate(&papers, &fetched, &reports), (0, 0));
}

#[test]
fn a_quote_not_on_its_fetched_page_fails_the_gate_and_the_copier_refuses_it() {
    let (papers, fetched, reports) = run_dir("fail", "Retries never back off.");
    assert_eq!(verify_through_the_gate(&papers, &fetched, &reports), (1, 1));
    let output = fs::read_to_string(reports.join("gate-output.txt")).unwrap();
    assert!(
        output.contains("research-layout: resilience/retries/retries.txt: quote-not-found"),
        "{output}"
    );
}
