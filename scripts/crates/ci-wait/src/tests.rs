//! Tests at the two seams: the `GitHub` and `Clock` traits (canned answers and
//! a fake clock) for the verdicts, and a tiny fake `gh` script for the real
//! client's request shape, deadline and failures.

use crate::{render, run, run_with, Clock, GhCli, GitHub, Job, Rerun, SystemClock, Verdict};
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::Duration;

#[path = "../../../test-support/fake_exe.rs"]
mod fake_exe;

const REPO: &str = "acme/kit";
const SHA: &str = "abc1234";
const FLAKY: &str = "flaky-e2e";

/// A GitHub that serves canned answers in call order and records every call.
struct Fake {
    gets: Vec<Result<String, String>>,
    posts: Vec<Result<String, String>>,
    calls: Vec<String>,
}

impl Fake {
    fn new(gets: Vec<Result<String, String>>) -> Fake {
        Fake {
            gets,
            posts: Vec::new(),
            calls: Vec::new(),
        }
    }

    fn with_posts(gets: Vec<Result<String, String>>, posts: Vec<Result<String, String>>) -> Fake {
        Fake {
            gets,
            posts,
            calls: Vec::new(),
        }
    }
}

/// The next answer: the queue's first, and the last one repeats.
fn next(queue: &mut Vec<Result<String, String>>) -> Result<String, String> {
    if queue.len() > 1 {
        queue.remove(0)
    } else {
        queue
            .first()
            .cloned()
            .unwrap_or_else(|| Err("no answer left".to_string()))
    }
}

impl GitHub for Fake {
    fn get(&mut self, path: &str) -> Result<String, String> {
        self.calls.push(format!("GET {path}"));
        next(&mut self.gets)
    }

    fn post(&mut self, path: &str) -> Result<String, String> {
        self.calls.push(format!("POST {path}"));
        next(&mut self.posts)
    }
}

/// A clock that advances by `step` on every sleep and records the pauses.
struct FakeClock {
    elapsed: Duration,
    step: Duration,
    sleeps: Vec<Duration>,
}

impl FakeClock {
    fn new(step: Duration) -> FakeClock {
        FakeClock {
            elapsed: Duration::ZERO,
            step,
            sleeps: Vec::new(),
        }
    }
}

impl Clock for FakeClock {
    fn elapsed(&self) -> Duration {
        self.elapsed
    }

    fn sleep(&mut self, d: Duration) {
        self.sleeps.push(d);
        self.elapsed += self.step;
    }
}

fn run_json(id: u64, name: &str, status: &str, conclusion: Option<&str>) -> String {
    let conclusion = match conclusion {
        Some(text) => format!("\"{text}\""),
        None => "null".to_string(),
    };
    format!(
        "{{\"id\": {id}, \"name\": \"{name}\", \"status\": \"{status}\", \"conclusion\": {conclusion}}}"
    )
}

fn page(runs: &[String]) -> String {
    format!(
        "{{\"total_count\": {}, \"check_runs\": [{}]}}",
        runs.len(),
        runs.join(", ")
    )
}

fn passing(id: u64, name: &str) -> String {
    run_json(id, name, "completed", Some("success"))
}

fn failed(id: u64, name: &str) -> String {
    run_json(id, name, "completed", Some("failure"))
}

fn repo_dir() -> String {
    format!("{}/tests/fixtures/repo", env!("CARGO_MANIFEST_DIR"))
}

fn check_path(page: u64) -> String {
    format!("GET repos/{REPO}/commits/{SHA}/check-runs?per_page=100&page={page}")
}

fn args_of(pairs: &[(&str, &str)]) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    for (name, value) in pairs {
        args.push(OsString::from(name));
        args.push(OsString::from(value));
    }
    args
}

fn base() -> Vec<OsString> {
    let dir = repo_dir();
    args_of(&[("--repo-dir", dir.as_str()), ("--sha", SHA)])
}

fn argv(extra: &[(&str, &str)]) -> Vec<OsString> {
    let mut args = base();
    args.extend(args_of(extra));
    args
}

fn run_case(args: Vec<OsString>, gh: &mut Fake, clock: &mut FakeClock) -> (i32, String, String) {
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = run_with(args, gh, clock, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).expect("stdout is UTF-8"),
        String::from_utf8(err).expect("stderr is UTF-8"),
    )
}

const GREEN_ONE_JOB: &str = "{\"verdict\":\"green\",\"repo\":\"acme/kit\",\"sha\":\"abc1234\",\"jobs\":[{\"name\":\"build\",\"status\":\"completed\",\"conclusion\":\"success\",\"reran\":false}],\"reruns\":[]}\n";

#[test]
fn two_identical_passing_looks_are_green() {
    let mut gh = Fake::new(vec![
        Ok(page(&[passing(1, "build")])),
        Ok(page(&[passing(1, "build")])),
    ]);
    let mut clock = FakeClock::new(Duration::from_secs(30));
    let (code, out, err) = run_case(argv(&[]), &mut gh, &mut clock);
    assert_eq!(code, 0);
    assert_eq!(err, "");
    assert_eq!(out, GREEN_ONE_JOB);
    assert_eq!(gh.calls, vec![check_path(1), check_path(1)]);
    assert_eq!(clock.sleeps, vec![Duration::from_secs(30)]);
}

#[test]
fn a_timeout_of_zero_looks_once() {
    let mut gh = Fake::new(vec![Ok(page(&[passing(1, "build")]))]);
    let mut clock = FakeClock::new(Duration::ZERO);
    let (code, out, _) = run_case(argv(&[("--timeout-secs", "0")]), &mut gh, &mut clock);
    assert_eq!(code, 1);
    assert!(out.contains("\"reason\":\"timed out\""), "{out}");
    assert_eq!(gh.calls, vec![check_path(1)]);
    assert!(clock.sleeps.is_empty());
}

#[test]
fn a_poll_of_zero_is_a_zero_pause() {
    let mut gh = Fake::new(vec![
        Ok(page(&[passing(1, "build")])),
        Ok(page(&[passing(1, "build")])),
    ]);
    let mut clock = FakeClock::new(Duration::ZERO);
    let (code, _, _) = run_case(argv(&[("--poll-secs", "0")]), &mut gh, &mut clock);
    assert_eq!(code, 0);
    assert_eq!(clock.sleeps, vec![Duration::ZERO]);
}

#[test]
fn a_failure_is_red_at_once() {
    let mut gh = Fake::new(vec![Ok(page(&[failed(1, "build")]))]);
    let mut clock = FakeClock::new(Duration::from_secs(30));
    let (code, out, _) = run_case(argv(&[]), &mut gh, &mut clock);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "{\"verdict\":\"red\",\"reason\":\"failed: build\",\"repo\":\"acme/kit\",\"sha\":\"abc1234\",\"jobs\":[{\"name\":\"build\",\"status\":\"completed\",\"conclusion\":\"failure\",\"reran\":false}],\"reruns\":[]}\n"
    );
    assert_eq!(gh.calls, vec![check_path(1)]);
}

#[test]
fn a_failure_that_is_not_listed_is_never_rerun() {
    let mut gh = Fake::new(vec![Ok(page(&[failed(1, "build")]))]);
    let mut clock = FakeClock::new(Duration::from_secs(30));
    let (code, _, _) = run_case(argv(&[]), &mut gh, &mut clock);
    assert_eq!(code, 1);
    assert!(gh.calls.iter().all(|call| call.starts_with("GET ")));
}

#[test]
fn a_listed_flaky_failure_is_rerun_once() {
    let mut gh = Fake::with_posts(
        vec![
            Ok(page(&[failed(1, FLAKY)])),
            Ok(page(&[failed(1, FLAKY)])),
            Ok(page(&[passing(2, FLAKY)])),
            Ok(page(&[passing(2, FLAKY)])),
        ],
        vec![Ok("{}".to_string())],
    );
    let mut clock = FakeClock::new(Duration::from_secs(30));
    let (code, out, _) = run_case(argv(&[]), &mut gh, &mut clock);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "{\"verdict\":\"green\",\"repo\":\"acme/kit\",\"sha\":\"abc1234\",\"jobs\":[{\"name\":\"flaky-e2e\",\"status\":\"completed\",\"conclusion\":\"success\",\"reran\":true}],\"reruns\":[{\"job\":\"flaky-e2e\",\"reason\":\"the runner pool is being drained\"}]}\n"
    );
    assert_eq!(
        gh.calls,
        vec![
            check_path(1),
            format!("POST repos/{REPO}/actions/jobs/1/rerun"),
            check_path(1),
            check_path(1),
            check_path(1),
        ]
    );
}

#[test]
fn a_rerun_that_fails_again_is_red() {
    let mut gh = Fake::with_posts(
        vec![Ok(page(&[failed(1, FLAKY)])), Ok(page(&[failed(2, FLAKY)]))],
        vec![Ok("{}".to_string())],
    );
    let mut clock = FakeClock::new(Duration::from_secs(30));
    let (code, out, _) = run_case(argv(&[]), &mut gh, &mut clock);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "{\"verdict\":\"red\",\"reason\":\"failed: flaky-e2e\",\"repo\":\"acme/kit\",\"sha\":\"abc1234\",\"jobs\":[{\"name\":\"flaky-e2e\",\"status\":\"completed\",\"conclusion\":\"failure\",\"reran\":true}],\"reruns\":[{\"job\":\"flaky-e2e\",\"reason\":\"the runner pool is being drained\"}]}\n"
    );
}

#[test]
fn a_rerun_that_cannot_be_posted_is_red() {
    let mut gh = Fake::with_posts(
        vec![Ok(page(&[failed(1, FLAKY)]))],
        vec![Err(format!(
            "gh api -X POST repos/{REPO}/actions/jobs/1/rerun: boom"
        ))],
    );
    let mut clock = FakeClock::new(Duration::from_secs(30));
    let (code, out, _) = run_case(argv(&[]), &mut gh, &mut clock);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "{\"verdict\":\"red\",\"reason\":\"rerun failed: flaky-e2e: gh api -X POST repos/acme/kit/actions/jobs/1/rerun: boom\",\"repo\":\"acme/kit\",\"sha\":\"abc1234\",\"jobs\":[{\"name\":\"flaky-e2e\",\"status\":\"completed\",\"conclusion\":\"failure\",\"reran\":false}],\"reruns\":[]}\n"
    );
}

#[test]
fn a_get_error_is_unreadable() {
    let mut gh = Fake::new(vec![Err(format!(
        "gh api -X GET repos/{REPO}/commits/{SHA}/check-runs?per_page=100&page=1: boom"
    ))]);
    let mut clock = FakeClock::new(Duration::from_secs(30));
    let (code, out, _) = run_case(argv(&[]), &mut gh, &mut clock);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "{\"verdict\":\"red\",\"reason\":\"unreadable: gh api -X GET repos/acme/kit/commits/abc1234/check-runs?per_page=100&page=1: boom\",\"repo\":\"acme/kit\",\"sha\":\"abc1234\",\"jobs\":[],\"reruns\":[]}\n"
    );
}

#[test]
fn no_checks_registered_times_out() {
    let mut gh = Fake::new(vec![Ok(page(&[]))]);
    let mut clock = FakeClock::new(Duration::ZERO);
    let (code, out, _) = run_case(argv(&[("--timeout-secs", "0")]), &mut gh, &mut clock);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "{\"verdict\":\"red\",\"reason\":\"no checks registered\",\"repo\":\"acme/kit\",\"sha\":\"abc1234\",\"jobs\":[],\"reruns\":[]}\n"
    );
}

#[test]
fn a_run_that_is_not_terminal_keeps_waiting() {
    let mut gh = Fake::new(vec![Ok(page(&[run_json(1, "build", "in_progress", None)]))]);
    let mut clock = FakeClock::new(Duration::ZERO);
    let (code, out, _) = run_case(argv(&[("--timeout-secs", "0")]), &mut gh, &mut clock);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "{\"verdict\":\"red\",\"reason\":\"timed out\",\"repo\":\"acme/kit\",\"sha\":\"abc1234\",\"jobs\":[{\"name\":\"build\",\"status\":\"in_progress\",\"conclusion\":null,\"reran\":false}],\"reruns\":[]}\n"
    );
}

#[test]
fn pages_are_read_until_total_count() {
    let first = format!(
        "{{\"total_count\": 2, \"check_runs\": [{}]}}",
        passing(1, "build")
    );
    let second = format!(
        "{{\"total_count\": 2, \"check_runs\": [{}]}}",
        passing(2, "lint")
    );
    let mut gh = Fake::new(vec![
        Ok(first.clone()),
        Ok(second.clone()),
        Ok(first),
        Ok(second),
    ]);
    let mut clock = FakeClock::new(Duration::from_secs(30));
    let (code, out, _) = run_case(argv(&[]), &mut gh, &mut clock);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "{\"verdict\":\"green\",\"repo\":\"acme/kit\",\"sha\":\"abc1234\",\"jobs\":[{\"name\":\"build\",\"status\":\"completed\",\"conclusion\":\"success\",\"reran\":false},{\"name\":\"lint\",\"status\":\"completed\",\"conclusion\":\"success\",\"reran\":false}],\"reruns\":[]}\n"
    );
    assert_eq!(
        gh.calls,
        vec![check_path(1), check_path(2), check_path(1), check_path(2)]
    );
}

#[test]
fn an_empty_page_stops_the_read() {
    let first = format!(
        "{{\"total_count\": 5, \"check_runs\": [{}]}}",
        passing(1, "build")
    );
    let empty = "{\"total_count\": 5, \"check_runs\": []}".to_string();
    let mut gh = Fake::new(vec![Ok(first), Ok(empty)]);
    let mut clock = FakeClock::new(Duration::ZERO);
    let (code, out, _) = run_case(argv(&[("--timeout-secs", "0")]), &mut gh, &mut clock);
    assert_eq!(code, 1);
    assert!(out.contains("\"reason\":\"timed out\""), "{out}");
    assert!(out.contains("\"jobs\":[{\"name\":\"build\""), "{out}");
    assert_eq!(gh.calls, vec![check_path(1), check_path(2)]);
}

#[test]
fn an_answer_that_is_not_the_promised_shape_is_unreadable() {
    let bodies = [
        "{not json".to_string(),
        "{\"total_count\": 1}".to_string(),
        "{\"total_count\": 1, \"check_runs\": [{\"name\": \"build\", \"status\": \"completed\", \"conclusion\": \"success\"}]}".to_string(),
        "{\"total_count\": 1, \"check_runs\": [{\"id\": 1, \"status\": \"completed\", \"conclusion\": \"success\"}]}".to_string(),
        "{\"total_count\": 1, \"check_runs\": [{\"id\": 1, \"name\": \"build\", \"conclusion\": \"success\"}]}".to_string(),
        "{\"total_count\": 1, \"check_runs\": [{\"id\": 1, \"name\": \"build\", \"status\": \"completed\", \"conclusion\": 7}]}".to_string(),
    ];
    for body in bodies {
        let mut gh = Fake::new(vec![Ok(body.clone())]);
        let mut clock = FakeClock::new(Duration::ZERO);
        let (code, out, _) = run_case(argv(&[]), &mut gh, &mut clock);
        assert_eq!(code, 1, "{body}");
        assert!(out.contains("\"reason\":\"unreadable: "), "{body}: {out}");
        assert!(out.contains("\"jobs\":[]"), "{body}: {out}");
    }
}

#[test]
fn the_repo_flag_overrides_the_tracker_repo() {
    let mut gh = Fake::new(vec![
        Ok(page(&[passing(1, "build")])),
        Ok(page(&[passing(1, "build")])),
    ]);
    let mut clock = FakeClock::new(Duration::from_secs(30));
    let (code, out, _) = run_case(argv(&[("--repo", "other/repo")]), &mut gh, &mut clock);
    assert_eq!(code, 0);
    assert!(out.contains("\"repo\":\"other/repo\""), "{out}");
    assert_eq!(
        gh.calls,
        vec![
            "GET repos/other/repo/commits/abc1234/check-runs?per_page=100&page=1".to_string(),
            "GET repos/other/repo/commits/abc1234/check-runs?per_page=100&page=1".to_string(),
        ]
    );
}

#[test]
fn bad_command_lines_are_usage() {
    let dir = repo_dir();
    let long = "a".repeat(65);
    let cases: Vec<Vec<OsString>> = vec![
        args_of(&[("--nope", "x")]),
        vec![OsString::from("--sha")],
        argv(&[("--repo", "acme/kit"), ("--repo", "acme/kit")]),
        args_of(&[("--sha", SHA)]),
        args_of(&[("--repo-dir", dir.as_str())]),
        args_of(&[("--repo-dir", dir.as_str()), ("--sha", "abc")]),
        args_of(&[("--repo-dir", dir.as_str()), ("--sha", "zzzzzzz")]),
        args_of(&[("--repo-dir", dir.as_str()), ("--sha", long.as_str())]),
        argv(&[("--repo", "acme")]),
        argv(&[("--repo", "./x")]),
        argv(&[("--timeout-secs", "soon")]),
        argv(&[("--poll-secs", "-1")]),
        vec![OsString::from(OsStr::from_bytes(b"--\xff"))],
        vec![
            OsString::from("--repo-dir"),
            OsString::from(dir.as_str()),
            OsString::from("--sha"),
            OsString::from(OsStr::from_bytes(b"\xff")),
        ],
    ];
    for args in cases {
        let mut gh = Fake::new(vec![Ok(page(&[]))]);
        let mut clock = FakeClock::new(Duration::ZERO);
        let (code, out, err) = run_case(args.clone(), &mut gh, &mut clock);
        assert_eq!(code, 2, "{args:?}");
        assert_eq!(out, "", "{args:?}");
        assert!(err.contains("usage: ci-wait"), "{args:?}: {err}");
        assert!(gh.calls.is_empty(), "{args:?}");
    }
}

#[test]
fn a_repository_without_a_setup_file_is_refused() {
    let dir = format!(
        "{}/tests/fixtures/repo-without-setup",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut gh = Fake::new(vec![Ok(page(&[]))]);
    let mut clock = FakeClock::new(Duration::ZERO);
    let args = args_of(&[("--repo-dir", dir.as_str()), ("--sha", SHA)]);
    let (code, out, err) = run_case(args, &mut gh, &mut clock);
    assert_eq!(code, 2);
    assert_eq!(out, "");
    assert!(err.starts_with("docs/agents/setup.yaml"), "{err}");
    assert!(gh.calls.is_empty());
}

#[test]
fn a_green_answer_omits_the_reason() {
    let answer = Verdict {
        green: true,
        reason: None,
        jobs: Vec::new(),
        reruns: Vec::new(),
    };
    assert_eq!(
        render(&answer, REPO, SHA),
        "{\"verdict\":\"green\",\"repo\":\"acme/kit\",\"sha\":\"abc1234\",\"jobs\":[],\"reruns\":[]}"
    );
}

#[test]
fn a_null_conclusion_prints_as_null() {
    let answer = Verdict {
        green: false,
        reason: Some("timed out".to_string()),
        jobs: vec![Job {
            name: "build".to_string(),
            status: "queued".to_string(),
            conclusion: None,
            reran: false,
        }],
        reruns: Vec::new(),
    };
    assert_eq!(
        render(&answer, REPO, SHA),
        "{\"verdict\":\"red\",\"reason\":\"timed out\",\"repo\":\"acme/kit\",\"sha\":\"abc1234\",\"jobs\":[{\"name\":\"build\",\"status\":\"queued\",\"conclusion\":null,\"reran\":false}],\"reruns\":[]}"
    );
}

#[test]
fn a_reason_with_quotes_is_escaped() {
    let answer = Verdict {
        green: false,
        reason: Some("failed: a \"quoted\" job".to_string()),
        jobs: Vec::new(),
        reruns: vec![Rerun {
            job: "a \"quoted\" job".to_string(),
            reason: "line\nbreak".to_string(),
        }],
    };
    let line = render(&answer, REPO, SHA);
    assert!(
        line.contains("\"reason\":\"failed: a \\\"quoted\\\" job\""),
        "{line}"
    );
    assert!(
        line.contains(
            "\"reruns\":[{\"job\":\"a \\\"quoted\\\" job\",\"reason\":\"line\\nbreak\"}]"
        ),
        "{line}"
    );
}

#[test]
fn run_without_arguments_is_usage() {
    assert_eq!(run(Vec::<OsString>::new()), 2);
}

#[test]
fn the_system_clock_measures_the_wait() {
    let mut clock = SystemClock::new();
    clock.sleep(Duration::from_millis(2));
    assert!(clock.elapsed() >= Duration::from_millis(2));
}

#[test]
fn default_is_a_system_clock() {
    assert!(SystemClock::default().elapsed() < Duration::from_secs(60));
}

/// A fake `gh` in a temporary directory, removed when the test ends.
struct Script {
    dir: PathBuf,
    path: PathBuf,
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Script {
    fn new(name: &str, body: &str) -> Script {
        let serial = fake_exe::lock();
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("ci-wait-script-{pid}-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gh");
        fake_exe::write_executable(&serial, &path, body);
        Script {
            dir,
            path,
            _serial: serial,
        }
    }

    fn client(&self, timeout: Duration) -> GhCli {
        GhCli::with(self.path.clone().into_os_string(), timeout)
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn gh_cli_asks_for_a_get_of_the_path() {
    let script = Script::new("args", "#!/bin/sh\nprintf '%s ' \"$@\"\n");
    let mut gh = script.client(Duration::from_secs(10));
    assert_eq!(
        gh.get("repos/acme/kit/commits/abc1234/check-runs"),
        Ok("api -X GET repos/acme/kit/commits/abc1234/check-runs ".to_string())
    );
}

#[test]
fn gh_cli_asks_for_a_post_of_the_path() {
    let script = Script::new("post-args", "#!/bin/sh\nprintf '%s ' \"$@\"\n");
    let mut gh = script.client(Duration::from_secs(10));
    assert_eq!(
        gh.post("repos/acme/kit/actions/jobs/1/rerun"),
        Ok("api -X POST repos/acme/kit/actions/jobs/1/rerun ".to_string())
    );
}

#[test]
fn gh_cli_returns_the_answer() {
    let script = Script::new(
        "answer",
        "#!/bin/sh\nprintf '%s\\n' '{\"total_count\": 0}'\n",
    );
    let mut gh = script.client(Duration::from_secs(10));
    assert_eq!(
        gh.get("repos/acme/kit/commits/abc1234/check-runs"),
        Ok("{\"total_count\": 0}\n".to_string())
    );
}

#[test]
fn gh_cli_reports_a_failure_with_the_method_and_path() {
    let script = Script::new("boom", "#!/bin/sh\necho 'boom' >&2\nexit 1\n");
    let mut gh = script.client(Duration::from_secs(10));
    assert_eq!(
        gh.get("repos/acme/kit/commits/abc1234/check-runs"),
        Err("gh api -X GET repos/acme/kit/commits/abc1234/check-runs: boom".to_string())
    );
    assert_eq!(
        gh.post("repos/acme/kit/actions/jobs/1/rerun"),
        Err("gh api -X POST repos/acme/kit/actions/jobs/1/rerun: boom".to_string())
    );
}

#[test]
fn gh_cli_kills_a_hung_call() {
    let script = Script::new("hung", "#!/bin/sh\nexec sleep 5\n");
    let mut gh = script.client(Duration::from_millis(100));
    assert_eq!(
        gh.get("repos/acme/kit/commits/abc1234/check-runs"),
        Err("gh timed out".to_string())
    );
}

#[test]
fn gh_cli_reports_a_missing_program() {
    let script = Script::new("missing", "#!/bin/sh\n");
    let missing = script.dir.join("no-such-gh");
    let mut gh = GhCli::with(missing.into_os_string(), Duration::from_secs(10));
    match gh.get("repos/acme/kit/commits/abc1234/check-runs") {
        Err(message) => assert!(message.contains("no-such-gh"), "{message}"),
        Ok(answer) => panic!("expected a failure, got {answer}"),
    }
}

#[test]
fn gh_cli_reads_non_utf8_output_lossily() {
    let script = Script::new("non-utf8", "#!/bin/sh\nprintf '\\377'\n");
    let mut gh = script.client(Duration::from_secs(10));
    assert_eq!(
        gh.get("repos/acme/kit/commits/abc1234/check-runs"),
        Ok("\u{fffd}".to_string())
    );
}

#[test]
fn default_is_the_real_gh_with_a_minute() {
    assert_eq!(
        format!("{:?}", GhCli::default()),
        format!("{:?}", GhCli::new())
    );
}
