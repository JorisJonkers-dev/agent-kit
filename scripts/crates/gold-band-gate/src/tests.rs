use super::*;
use std::cell::RefCell;
use std::path::PathBuf;

/// 2026-11-11T09:30:00Z.
const NOW: u64 = 1_794_389_400;
const STAMP: &str = "2026-11-11T09:30:00Z";

/// A host whose command ends as scripted, after writing `output`.
struct Fake {
    outcome: Outcome,
    output: Vec<u8>,
    ran: RefCell<Vec<(Vec<String>, Duration)>>,
    out: RefCell<String>,
    err: RefCell<String>,
}

fn fake(outcome: Outcome, output: &str) -> Fake {
    Fake {
        outcome,
        output: output.as_bytes().to_vec(),
        ran: RefCell::default(),
        out: RefCell::default(),
        err: RefCell::default(),
    }
}

impl Clock for Fake {
    fn unix_seconds(&self) -> u64 {
        NOW
    }
}

impl Host for Fake {
    fn execute(&self, argv: &[String], timeout: Duration, output: &Path) -> Outcome {
        self.ran.borrow_mut().push((argv.to_vec(), timeout));
        // The wrapper cleared the directory and made the file before this.
        assert_eq!(fs::read(output).unwrap(), b"");
        fs::write(output, &self.output).unwrap();
        self.outcome.clone()
    }
    fn print(&self, stderr: bool, text: &str) {
        let target = if stderr { &self.err } else { &self.out };
        target.borrow_mut().push_str(text);
    }
}

fn dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gb-gate-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn call(host: &dyn Host, args: &[&str]) -> i32 {
    run_with(args.iter().map(OsString::from), host)
}

fn wrapper(host: &Fake, dir: &Path, extra: &[&str]) -> i32 {
    let mut args = vec![
        "run",
        "--run-id",
        "run-9",
        "--attempt",
        "3",
        "--timeout-secs",
        "30",
        "--report-dir",
        dir.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    call(host, &args)
}

fn report_of(dir: &Path) -> Report {
    gold_band_report::read(&dir.join(REPORT_FILE)).unwrap()
}

// ---- run -------------------------------------------------------------

#[test]
fn a_green_command_writes_a_report_with_its_output_hash() {
    let d = dir("green");
    let host = fake(Outcome::Exited(0), "built\n");
    assert_eq!(wrapper(&host, &d, &["--", "cargo", "build", "--locked"]), 0);
    let report = report_of(&d);
    assert_eq!(report.run_id, "run-9");
    assert_eq!(report.attempt, 3);
    assert_eq!(report.command, "cargo build --locked");
    assert_eq!(report.exit_code, 0);
    assert_eq!(report.output_hash, sha256_hex(b"built\n"));
    assert_eq!(report.timestamp, STAMP);
    assert_eq!(report.failed_check, None);
    assert_eq!(fs::read(d.join(OUTPUT_FILE)).unwrap(), b"built\n");
    let ran = host.ran.borrow();
    assert_eq!(ran[0].0, ["cargo", "build", "--locked"]);
    assert_eq!(ran[0].1, Duration::from_secs(30));
    assert_eq!(
        *host.out.borrow(),
        "gold-band-gate: cargo build --locked exit 0\n"
    );
    assert_eq!(*host.err.borrow(), "");
}

#[test]
fn a_red_command_exits_with_its_own_code_and_the_report_says_so() {
    let d = dir("red");
    let host = fake(Outcome::Exited(101), "error[E0308]\n");
    assert_eq!(wrapper(&host, &d, &["--", "cargo", "test"]), 101);
    let report = report_of(&d);
    assert_eq!(report.exit_code, 101);
    assert_eq!(report.failed_check, None);
}

#[test]
fn a_timeout_is_exit_124_with_a_failed_report_and_a_note_in_the_output() {
    let d = dir("timeout");
    let host = fake(Outcome::TimedOut, "partial\n");
    assert_eq!(wrapper(&host, &d, &["--", "sleep", "30"]), EXIT_TIMEOUT);
    let report = report_of(&d);
    assert_eq!(report.exit_code, 124);
    assert_eq!(report.failed_check.as_deref(), Some("timeout"));
    let output = fs::read_to_string(d.join(OUTPUT_FILE)).unwrap();
    assert!(output.starts_with("partial\n") && output.contains("timed out after 30 seconds"));
    assert_eq!(report.output_hash, sha256_hex(output.as_bytes()));
    assert!(host.out.borrow().contains("exit 124 (timeout)"));
}

#[test]
fn a_command_that_cannot_start_is_exit_127_with_a_failed_report() {
    let d = dir("nostart");
    let host = fake(Outcome::CannotStart("No such file".to_string()), "");
    assert_eq!(
        wrapper(&host, &d, &["--", "no-such-gate"]),
        EXIT_CANNOT_START
    );
    let report = report_of(&d);
    assert_eq!(report.exit_code, 127);
    assert_eq!(report.failed_check.as_deref(), Some("cannot-start"));
    let output = fs::read_to_string(d.join(OUTPUT_FILE)).unwrap();
    assert!(output.contains("cannot start: No such file"));
    assert_eq!(report.output_hash, sha256_hex(output.as_bytes()));
}

#[test]
fn the_previous_attempts_report_and_output_are_cleared_first() {
    let d = dir("clear");
    fs::write(d.join(REPORT_FILE), "old report").unwrap();
    fs::write(d.join(OUTPUT_FILE), "old output").unwrap();
    // The fake asserts the output file is empty when the command starts.
    let host = fake(Outcome::Exited(0), "new\n");
    assert_eq!(wrapper(&host, &d, &["--", "true"]), 0);
    assert_eq!(fs::read(d.join(OUTPUT_FILE)).unwrap(), b"new\n");
    assert_eq!(report_of(&d).output_hash, sha256_hex(b"new\n"));
}

#[test]
fn a_report_directory_that_is_created_when_missing() {
    let d = dir("mkdir").join("deep").join("reports");
    let host = fake(Outcome::Exited(0), "");
    assert_eq!(wrapper(&host, &d, &["--", "true"]), 0);
    assert!(d.join(REPORT_FILE).is_file());
}

#[test]
fn output_beyond_the_cap_is_cut_on_disk_and_in_the_hash() {
    let d = dir("cap");
    let mut host = fake(Outcome::Exited(0), "");
    host.output = vec![b'x'; MAX_OUTPUT_BYTES as usize + 5];
    assert_eq!(wrapper(&host, &d, &["--", "noisy"]), 0);
    let kept = fs::read(d.join(OUTPUT_FILE)).unwrap();
    assert_eq!(kept.len() as u64, MAX_OUTPUT_BYTES);
    assert_eq!(report_of(&d).output_hash, sha256_hex(&kept));
}

#[test]
fn a_report_directory_that_is_a_file_is_exit_2() {
    let d = dir("notdir");
    let blocker = d.join("file");
    fs::write(&blocker, "x").unwrap();
    let host = fake(Outcome::Exited(0), "");
    assert_eq!(wrapper(&host, &blocker, &["--", "true"]), 2);
    assert!(host.err.borrow().contains("cannot prepare"));
    assert!(
        host.ran.borrow().is_empty(),
        "the command ran without a report directory"
    );
}

#[test]
fn a_stale_report_that_cannot_be_removed_stops_the_attempt() {
    let d = dir("unremovable");
    // A directory where the report file belongs cannot be removed as a file.
    fs::create_dir_all(d.join(REPORT_FILE)).unwrap();
    let host = fake(Outcome::Exited(0), "");
    assert_eq!(wrapper(&host, &d, &["--", "true"]), 2);
    assert!(host.err.borrow().contains("cannot prepare"));
    assert!(host.ran.borrow().is_empty());
}

#[test]
fn a_report_that_cannot_be_written_is_exit_2() {
    let d = dir("nowrite");
    let host = fake(Outcome::Exited(0), "");
    // The command makes a directory where the report goes.
    struct Blocker(Fake);
    impl Clock for Blocker {
        fn unix_seconds(&self) -> u64 {
            NOW
        }
    }
    impl Host for Blocker {
        fn execute(&self, argv: &[String], t: Duration, output: &Path) -> Outcome {
            fs::create_dir_all(output.parent().unwrap().join(REPORT_FILE)).unwrap();
            self.0.execute(argv, t, output)
        }
        fn print(&self, stderr: bool, text: &str) {
            self.0.print(stderr, text);
        }
    }
    let blocker = Blocker(host);
    assert_eq!(wrapper_dyn(&blocker, &d), 2);
    assert!(blocker.0.err.borrow().contains("cannot write"));
}

fn wrapper_dyn(host: &dyn Host, dir: &Path) -> i32 {
    call(
        host,
        &[
            "run",
            "--run-id",
            "r",
            "--attempt",
            "1",
            "--timeout-secs",
            "5",
            "--report-dir",
            dir.to_str().unwrap(),
            "--",
            "true",
        ],
    )
}

#[test]
fn an_output_file_that_vanishes_is_exit_2() {
    let d = dir("vanish");
    struct Vanisher(Fake);
    impl Clock for Vanisher {
        fn unix_seconds(&self) -> u64 {
            NOW
        }
    }
    impl Host for Vanisher {
        fn execute(&self, _argv: &[String], _t: Duration, output: &Path) -> Outcome {
            fs::remove_file(output).unwrap();
            Outcome::Exited(0)
        }
        fn print(&self, stderr: bool, text: &str) {
            self.0.print(stderr, text);
        }
    }
    let host = Vanisher(fake(Outcome::Exited(0), ""));
    assert_eq!(wrapper_dyn(&host, &d), 2);
    assert!(host.0.err.borrow().contains("cannot read"));
}

#[test]
fn a_timeout_note_that_cannot_be_appended_is_exit_2() {
    let d = dir("noappend");
    struct Vanisher(Fake);
    impl Clock for Vanisher {
        fn unix_seconds(&self) -> u64 {
            NOW
        }
    }
    impl Host for Vanisher {
        fn execute(&self, _argv: &[String], _t: Duration, output: &Path) -> Outcome {
            fs::remove_file(output).unwrap();
            Outcome::TimedOut
        }
        fn print(&self, stderr: bool, text: &str) {
            self.0.print(stderr, text);
        }
    }
    let host = Vanisher(fake(Outcome::Exited(0), ""));
    assert_eq!(wrapper_dyn(&host, &d), 2);
    assert!(host.0.err.borrow().contains("cannot write"));
}

#[test]
fn bad_run_usage_is_exit_2_with_the_usage_text() {
    let d = dir("usage");
    let dir_arg = d.to_str().unwrap();
    let base = [
        "--run-id",
        "r",
        "--attempt",
        "1",
        "--timeout-secs",
        "5",
        "--report-dir",
        dir_arg,
    ];
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec![], "the subcommands are run and verify"),
        (vec!["check"], "the subcommands are run and verify"),
        (vec!["run", "--bogus", "x"], "unexpected argument --bogus"),
        (vec!["run", "stray"], "unexpected argument stray"),
        (vec!["run", "--run-id"], "--run-id needs a value"),
        (
            vec!["run", "--run-id", "a", "--run-id", "b"],
            "--run-id given twice",
        ),
        (
            vec![
                "run",
                "--attempt",
                "1",
                "--timeout-secs",
                "5",
                "--report-dir",
                dir_arg,
                "--",
                "x",
            ],
            "--run-id is required",
        ),
        (
            vec![
                "run",
                "--run-id",
                "",
                "--attempt",
                "1",
                "--timeout-secs",
                "5",
                "--report-dir",
                dir_arg,
                "--",
                "x",
            ],
            "--run-id must not be empty",
        ),
        (
            vec![
                "run",
                "--run-id",
                "r",
                "--attempt",
                "x",
                "--timeout-secs",
                "5",
                "--report-dir",
                dir_arg,
                "--",
                "x",
            ],
            "--attempt must be a non-negative integer",
        ),
        (
            vec![
                "run",
                "--run-id",
                "r",
                "--attempt",
                "99999999999",
                "--timeout-secs",
                "5",
                "--report-dir",
                dir_arg,
                "--",
                "x",
            ],
            "--attempt is too large",
        ),
        (
            vec![
                "run",
                "--run-id",
                "r",
                "--attempt",
                "1",
                "--timeout-secs",
                "0",
                "--report-dir",
                dir_arg,
                "--",
                "x",
            ],
            "--timeout-secs must be between",
        ),
        (
            vec![
                "run",
                "--run-id",
                "r",
                "--attempt",
                "1",
                "--timeout-secs",
                "604801",
                "--report-dir",
                dir_arg,
                "--",
                "x",
            ],
            "--timeout-secs must be between",
        ),
        (
            vec![
                "run",
                "--run-id",
                "r",
                "--attempt",
                "1",
                "--report-dir",
                dir_arg,
                "--",
                "x",
            ],
            "--timeout-secs is required",
        ),
    ];
    for (args, why) in cases {
        let host = fake(Outcome::Exited(0), "");
        assert_eq!(call(&host, &args), 2, "{args:?}");
        let err = host.err.borrow();
        assert!(
            err.contains(why) && err.contains("usage:"),
            "{args:?}: {err}"
        );
        assert!(host.ran.borrow().is_empty());
    }
    // A command is required after the double dash.
    let host = fake(Outcome::Exited(0), "");
    let mut args = vec!["run"];
    args.extend(base);
    assert_eq!(call(&host, &args), 2);
    args.push("--");
    assert_eq!(call(&host, &args), 2);
    assert!(host.err.borrow().contains("a command is required after --"));
    assert!(!d.join(REPORT_FILE).exists());
}

#[test]
fn flag_like_words_after_the_double_dash_belong_to_the_command() {
    let d = dir("dashes");
    let host = fake(Outcome::Exited(0), "");
    assert_eq!(
        wrapper(&host, &d, &["--", "tool", "--run-id", "--", "x"]),
        0
    );
    assert_eq!(report_of(&d).command, "tool --run-id -- x");
    assert_eq!(host.ran.borrow()[0].0, ["tool", "--run-id", "--", "x"]);
}

// ---- verify ----------------------------------------------------------

const COMMAND: &str = "cargo test --locked";

/// A directory holding a genuine green report and its output.
fn gated(tag: &str) -> PathBuf {
    let d = dir(tag);
    let host = fake(Outcome::Exited(0), "ok\n");
    assert_eq!(wrapper(&host, &d, &["--", "cargo", "test", "--locked"]), 0);
    d
}

fn verify_args(d: &Path) -> Vec<String> {
    [
        "verify",
        "--run-id",
        "run-9",
        "--attempt",
        "3",
        "--command",
        COMMAND,
        "--report-dir",
        d.to_str().unwrap(),
    ]
    .map(String::from)
    .to_vec()
}

fn checked(d: &Path, extra: &[&str]) -> (i32, Fake) {
    let host = fake(Outcome::Exited(0), "");
    let mut args = verify_args(d);
    args.extend(extra.iter().map(|s| s.to_string()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    (call(&host, &refs), host)
}

fn rewrite(d: &Path, edit: impl Fn(Report) -> Report) {
    let report = edit(report_of(d));
    fs::write(d.join(REPORT_FILE), format!("{}\n", report.to_json())).unwrap();
}

fn problems(host: &Fake) -> Vec<String> {
    host.err.borrow().lines().map(str::to_string).collect()
}

#[test]
fn a_genuine_report_passes_and_is_printed_verbatim() {
    let d = gated("v-ok");
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 0, "{}", host.err.borrow());
    assert_eq!(
        *host.out.borrow(),
        fs::read_to_string(d.join(REPORT_FILE)).unwrap()
    );
    assert_eq!(*host.err.borrow(), "");
}

#[test]
fn verify_writes_nothing() {
    let d = gated("v-readonly");
    let before: Vec<_> = {
        let mut names: Vec<_> = fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    let report = fs::read(d.join(REPORT_FILE)).unwrap();
    let _ = checked(&d, &[]);
    rewrite(&d, |r| Report {
        run_id: "other".into(),
        ..r
    });
    let _ = checked(&d, &[]);
    let mut after: Vec<_> = fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    after.sort();
    assert_eq!(before, after);
    assert_ne!(report, fs::read(d.join(REPORT_FILE)).unwrap());
}

#[test]
fn a_missing_report_fails_as_missing() {
    let d = dir("v-missing");
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 1);
    assert_eq!(*host.out.borrow(), "");
    let lines = problems(&host);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("gold-band-gate: missing: no gate report at "));
}

#[test]
fn a_report_from_another_run_fails_as_other_run() {
    let d = gated("v-other-run");
    rewrite(&d, |r| Report {
        run_id: "run-8".into(),
        ..r
    });
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 1);
    assert_eq!(*host.out.borrow(), "");
    let lines = problems(&host);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("gold-band-gate: other-run: "),
        "{lines:?}"
    );
}

#[test]
fn a_report_from_another_attempt_fails_as_other_attempt() {
    let d = gated("v-other-attempt");
    rewrite(&d, |r| Report { attempt: 2, ..r });
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 1);
    assert!(problems(&host)[0].starts_with("gold-band-gate: other-attempt: "));
}

#[test]
fn a_report_for_another_command_fails_as_command_mismatch() {
    let d = gated("v-command");
    rewrite(&d, |r| Report {
        command: "true".into(),
        ..r
    });
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 1);
    let lines = problems(&host);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("gold-band-gate: command-mismatch: "));
}

#[test]
fn rewritten_output_fails_as_hash_mismatch() {
    let d = gated("v-hash");
    fs::write(d.join(OUTPUT_FILE), "all green, honest\n").unwrap();
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 1);
    let lines = problems(&host);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("gold-band-gate: hash-mismatch: "));
}

#[test]
fn a_missing_output_file_fails_as_hash_mismatch() {
    let d = gated("v-nooutput");
    fs::remove_file(d.join(OUTPUT_FILE)).unwrap();
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 1);
    assert!(
        problems(&host)[0].contains("hash-mismatch: ")
            && problems(&host)[0].contains("cannot be read")
    );
}

#[test]
fn an_oversized_output_file_fails_as_hash_mismatch() {
    let d = gated("v-bigoutput");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(d.join(OUTPUT_FILE))
        .unwrap();
    file.set_len(MAX_OUTPUT_BYTES + 1).unwrap();
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 1);
    assert!(
        problems(&host)[0].contains("hash-mismatch: ")
            && problems(&host)[0].contains("larger than")
    );
}

#[test]
fn an_old_report_fails_as_stale_by_default_and_by_max_age() {
    let d = gated("v-stale");
    // Fresh at the stamp, an hour later still inside the default.
    let (code, _) = checked(&d, &["--now", "2026-11-11T10:30:00Z"]);
    assert_eq!(code, 0);
    let (code, host) = checked(&d, &["--now", "2026-11-11T10:30:01Z"]);
    assert_eq!(code, 1);
    assert!(problems(&host)[0].starts_with("gold-band-gate: stale: "));
    assert!(problems(&host)[0].contains("more than 3600 seconds old"));
    let (code, host) = checked(
        &d,
        &["--now", "2026-11-11T09:31:00Z", "--max-age-secs", "30"],
    );
    assert_eq!(code, 1);
    assert!(problems(&host)[0].contains("more than 30 seconds old"));
}

#[test]
fn a_report_from_the_future_fails_as_stale() {
    let d = gated("v-future");
    let (code, _) = checked(&d, &["--now", "2026-11-11T09:29:00Z"]);
    assert_eq!(code, 0, "a minute of clock skew is tolerated");
    let (code, host) = checked(&d, &["--now", "2026-11-11T09:28:59Z"]);
    assert_eq!(code, 1);
    assert!(problems(&host)[0].contains("stale: ") && problems(&host)[0].contains("in the future"));
}

#[test]
fn the_clock_is_the_hosts_when_now_is_not_given() {
    let d = gated("v-clock");
    // The fake clock is the report's own second.
    assert_eq!(checked(&d, &[]).0, 0);
    rewrite(&d, |r| Report {
        timestamp: "2026-11-11T07:00:00Z".into(),
        ..r
    });
    assert_eq!(checked(&d, &[]).0, 1);
}

#[test]
fn a_failed_command_report_fails_naming_the_code() {
    let d = dir("v-red");
    let host = fake(Outcome::Exited(101), "boom\n");
    assert_eq!(
        wrapper(&host, &d, &["--", "cargo", "test", "--locked"]),
        101
    );
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 1);
    assert_eq!(*host.out.borrow(), "");
    let lines = problems(&host);
    assert_eq!(
        lines,
        ["gold-band-gate: failed-command: the command exited 101"]
    );
}

#[test]
fn a_timed_out_report_fails_naming_the_timeout() {
    let d = dir("v-timeout");
    let host = fake(Outcome::TimedOut, "");
    assert_eq!(
        wrapper(&host, &d, &["--", "cargo", "test", "--locked"]),
        124
    );
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 1);
    assert_eq!(
        problems(&host),
        ["gold-band-gate: failed-command: the command exited 124 (the command timed out)"]
    );
}

#[test]
fn a_cannot_start_report_names_its_failed_check() {
    let d = dir("v-nostart");
    let host = fake(Outcome::CannotStart("nope".into()), "");
    assert_eq!(
        wrapper(&host, &d, &["--", "cargo", "test", "--locked"]),
        127
    );
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 1);
    assert_eq!(
        problems(&host),
        ["gold-band-gate: failed-command: the command exited 127 (cannot-start)"]
    );
}

#[test]
fn a_zero_exit_that_still_carries_a_failed_check_fails() {
    let d = gated("v-inconsistent");
    rewrite(&d, |r| Report {
        failed_check: Some("timeout".into()),
        ..r
    });
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 1);
    assert_eq!(
        problems(&host),
        ["gold-band-gate: failed-command: the command exited 0 (timeout)"]
    );
}

#[test]
fn every_problem_is_listed_one_per_line() {
    let d = gated("v-many");
    rewrite(&d, |r| Report {
        run_id: "x".into(),
        attempt: 7,
        command: "y".into(),
        exit_code: 1,
        ..r
    });
    fs::remove_file(d.join(OUTPUT_FILE)).unwrap();
    let (code, host) = checked(&d, &["--now", "2030-01-01T00:00:00Z"]);
    assert_eq!(code, 1);
    let checks: Vec<String> = problems(&host)
        .iter()
        .map(|l| l.split(": ").nth(1).unwrap().to_string())
        .collect();
    assert_eq!(
        checks,
        [
            "other-run",
            "other-attempt",
            "command-mismatch",
            "stale",
            "hash-mismatch",
            "failed-command"
        ]
    );
}

#[test]
fn an_untrusted_report_that_is_malformed_is_exit_2() {
    let bad: Vec<(&str, Vec<u8>)> = vec![
        ("not json", b"nope".to_vec()),
        ("not utf8", vec![0xff, 0xfe, 0x00]),
        ("array", b"[1,2]".to_vec()),
        ("empty", Vec::new()),
        (
            "string attempt",
            format!("{{\"run_id\":\"run-9\",\"attempt\":\"3\",\"command\":\"{COMMAND}\",\"exit_code\":0,\"output_hash\":\"{}\",\"timestamp\":\"{STAMP}\"}}", sha256_hex(b"ok\n")).into_bytes(),
        ),
        (
            "float exit code",
            format!("{{\"run_id\":\"run-9\",\"attempt\":3,\"command\":\"{COMMAND}\",\"exit_code\":0.0,\"output_hash\":\"{}\",\"timestamp\":\"{STAMP}\"}}", sha256_hex(b"ok\n")).into_bytes(),
        ),
        (
            "short hash",
            format!("{{\"run_id\":\"run-9\",\"attempt\":3,\"command\":\"{COMMAND}\",\"exit_code\":0,\"output_hash\":\"abc\",\"timestamp\":\"{STAMP}\"}}").into_bytes(),
        ),
        (
            "bad timestamp",
            format!("{{\"run_id\":\"run-9\",\"attempt\":3,\"command\":\"{COMMAND}\",\"exit_code\":0,\"output_hash\":\"{}\",\"timestamp\":\"yesterday\"}}", sha256_hex(b"ok\n")).into_bytes(),
        ),
    ];
    for (name, content) in bad {
        let d = gated("v-bad");
        fs::write(d.join(REPORT_FILE), content).unwrap();
        let (code, host) = checked(&d, &[]);
        assert_eq!(code, 2, "{name}");
        assert_eq!(*host.out.borrow(), "", "{name}");
        assert!(host.err.borrow().contains("gold-band-gate:"), "{name}");
    }
}

#[test]
fn a_huge_report_is_refused_without_being_read_whole() {
    let d = gated("v-huge");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(d.join(REPORT_FILE))
        .unwrap();
    file.set_len(MAX_REPORT_BYTES + 1).unwrap();
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 2);
    assert!(host.err.borrow().contains("larger than"));
}

#[test]
fn a_report_that_cannot_be_read_is_exit_2_not_missing() {
    let d = dir("v-unreadable");
    // A directory where the report file belongs: it exists, it cannot be read.
    fs::create_dir_all(d.join(REPORT_FILE)).unwrap();
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 2);
    assert!(host.err.borrow().contains("cannot read"));
}

#[test]
fn unknown_report_fields_are_ignored_but_printed_verbatim_only_when_it_passes() {
    let d = gated("v-extra");
    let text = fs::read_to_string(d.join(REPORT_FILE)).unwrap();
    let extended = text.replace("}\n", ",\"note\":\"x\"}\n");
    fs::write(d.join(REPORT_FILE), &extended).unwrap();
    let (code, host) = checked(&d, &[]);
    assert_eq!(code, 0);
    assert_eq!(*host.out.borrow(), extended);
}

#[test]
fn bad_verify_usage_is_exit_2() {
    let d = dir("v-usage");
    let dir_arg = d.to_str().unwrap();
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec!["verify", "--"], "unexpected argument --"),
        (
            vec!["verify", "--timeout-secs", "5"],
            "unexpected argument --timeout-secs",
        ),
        (
            vec![
                "verify",
                "--run-id",
                "r",
                "--attempt",
                "1",
                "--report-dir",
                dir_arg,
            ],
            "--command is required",
        ),
        (
            vec![
                "verify",
                "--run-id",
                "r",
                "--attempt",
                "1",
                "--command",
                "c",
                "--report-dir",
                dir_arg,
                "--max-age-secs",
                "-1",
            ],
            "--max-age-secs must be a non-negative integer",
        ),
        (
            vec![
                "verify",
                "--run-id",
                "r",
                "--attempt",
                "1",
                "--command",
                "c",
                "--report-dir",
                dir_arg,
                "--now",
                "soon",
            ],
            "--now must be RFC 3339 UTC",
        ),
    ];
    for (args, why) in cases {
        let host = fake(Outcome::Exited(0), "");
        assert_eq!(call(&host, &args), 2, "{args:?}");
        assert!(
            host.err.borrow().contains(why),
            "{args:?}: {}",
            host.err.borrow()
        );
    }
}

// ---- the real host -----------------------------------------------------

fn real(d: &Path, timeout: &str, command: &[&str]) -> (i32, Report, String) {
    let mut args = vec![
        "run",
        "--run-id",
        "r",
        "--attempt",
        "1",
        "--timeout-secs",
        timeout,
        "--report-dir",
        d.to_str().unwrap(),
        "--",
    ];
    args.extend_from_slice(command);
    let code = run(args.iter().map(OsString::from));
    let output = fs::read_to_string(d.join(OUTPUT_FILE)).unwrap();
    (code, report_of(d), output)
}

#[test]
fn the_real_host_captures_stdout_and_stderr_in_order() {
    let d = dir("real-order");
    let (code, report, output) = real(
        &d,
        "30",
        &["sh", "-c", "echo one; echo two >&2; echo three"],
    );
    assert_eq!(code, 0);
    assert_eq!(output, "one\ntwo\nthree\n");
    assert_eq!(report.output_hash, sha256_hex(output.as_bytes()));
    assert_eq!(report.command, "sh -c echo one; echo two >&2; echo three");
}

#[test]
fn the_real_host_returns_the_commands_own_exit_code() {
    let d = dir("real-exit");
    let (code, report, _) = real(&d, "30", &["sh", "-c", "exit 7"]);
    assert_eq!((code, report.exit_code), (7, 7));
}

#[test]
fn the_real_host_maps_a_signal_death_to_128_plus_the_signal() {
    let d = dir("real-signal");
    let (code, report, _) = real(&d, "30", &["sh", "-c", "kill -9 $$"]);
    assert_eq!((code, report.exit_code), (137, 137));
}

#[test]
fn the_real_host_kills_a_hung_command_at_the_deadline() {
    let d = dir("real-hung");
    let started = Instant::now();
    let (code, report, output) = real(&d, "1", &["sleep", "30"]);
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the wrapper hung with the command"
    );
    assert_eq!(code, 124);
    assert_eq!(report.exit_code, 124);
    assert_eq!(report.failed_check.as_deref(), Some("timeout"));
    assert!(output.contains("timed out after 1 seconds"));
}

#[test]
fn the_real_host_kills_the_whole_process_group_of_a_hung_command() {
    let d = dir("real-group");
    // The shell backgrounds a long sleep that holds the output file open.
    let started = Instant::now();
    let (code, _, _) = real(&d, "1", &["sh", "-c", "sleep 30 & wait"]);
    assert_eq!(code, 124);
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn the_real_host_reports_a_missing_command_as_cannot_start() {
    let d = dir("real-missing");
    let (code, report, output) = real(&d, "30", &["/no/such/gate-command"]);
    assert_eq!(code, 127);
    assert_eq!(report.failed_check.as_deref(), Some("cannot-start"));
    assert!(output.contains("cannot start: "));
}

#[test]
fn the_real_host_reports_a_non_executable_command_as_cannot_start() {
    let d = dir("real-noexec");
    let script = d.join("gate.sh");
    fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    let (code, report, _) = real(&d, "30", &[script.to_str().unwrap()]);
    assert_eq!(code, 127);
    assert_eq!(report.failed_check.as_deref(), Some("cannot-start"));
}

#[test]
fn the_real_host_cannot_start_into_an_output_file_that_is_gone() {
    let d = dir("real-gone");
    let outcome = RealHost.execute(
        &["true".to_string()],
        Duration::from_secs(5),
        &d.join("absent"),
    );
    assert!(matches!(outcome, Outcome::CannotStart(_)));
}

#[test]
fn the_real_host_clock_and_printing_work() {
    assert!(RealHost.unix_seconds() > NOW);
    RealHost.print(false, "");
    RealHost.print(true, "");
}

#[test]
fn a_real_gate_then_a_real_copy_round_trips() {
    let d = dir("real-roundtrip");
    let (code, _, _) = real(&d, "30", &["true"]);
    assert_eq!(code, 0);
    let code = run([
        "verify",
        "--run-id",
        "r",
        "--attempt",
        "1",
        "--command",
        "true",
        "--report-dir",
        d.to_str().unwrap(),
    ]
    .map(OsString::from));
    assert_eq!(code, 0);
}
