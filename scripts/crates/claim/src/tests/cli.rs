//! The setup canary crate: the Rust workspace's proof of path.
//!
//! The standards repo's ADR-0014 puts all of agent-kit's Rust in one workspace
//! under `scripts/`. This member is deliberately trivial and std-only, with no
//! dependency of any kind.
//! `--version` proves the build end to end; the command also validates a path.
//! New Rust joins this workspace as a new member, never as its own Cargo
//! project.
//!
//! Tests for the claim command line: real bare repositories for git work,
//! recording stubs for GitHub and for the slug-shaped URL path.

use crate::cli::*;
use crate::gh::Gh;
use crate::gh::{is_slug, run_command, RealGh};
use crate::lease::{FixedClock, Lease};
use crate::store::Git;
use crate::store::{RealGit, Store};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    base: PathBuf,
    url: String,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "claim-cli-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let bare = base.join("remote.git");
        let setup = RealGit { dir: base.clone() };
        setup
            .git(&["init", "--bare", "-q", bare.to_str().unwrap()], "")
            .unwrap();
        setup
            .git(
                &[
                    "-C",
                    bare.to_str().unwrap(),
                    "config",
                    "core.hooksPath",
                    "/dev/null",
                ],
                "",
            )
            .unwrap();
        Self {
            url: bare.to_string_lossy().to_string(),
            base,
        }
    }

    fn scratch(&self, name: &str) -> RealGit {
        let dir = self.base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let git = RealGit { dir };
        git.git(&["init", "-q"], "").unwrap();
        git.git(&["config", "core.hooksPath", "/dev/null"], "")
            .unwrap();
        git
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

fn call(items: &[&str]) -> Vec<String> {
    args(items)
}

fn env(host: Option<&str>, run: Option<&str>) -> impl Fn(&str) -> Option<String> {
    let host = host.map(str::to_string);
    let run = run.map(str::to_string);
    move |name| match name {
        "HOSTNAME" => host.clone(),
        "CLAIM_RUN" => run.clone(),
        _ => None,
    }
}

fn lease_at(device: &str, run: &str, issue: u64, expires: u64) -> Lease {
    Lease {
        device: device.to_string(),
        run: run.to_string(),
        issue,
        expires,
    }
}

struct RecordingGh {
    calls: RefCell<Vec<Vec<String>>>,
    failure: Option<String>,
    fail_at: Option<usize>,
}

impl RecordingGh {
    fn new(failure: Option<&str>) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            failure: failure.map(str::to_string),
            fail_at: None,
        }
    }

    fn failing_at(index: usize, failure: &str) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            failure: Some(failure.to_string()),
            fail_at: Some(index),
        }
    }
}

impl Gh for RecordingGh {
    fn gh(&self, arguments: &[&str]) -> Result<(), String> {
        let mut calls = self.calls.borrow_mut();
        calls.push(call(arguments));
        let index = calls.len() - 1;
        drop(calls);
        match &self.failure {
            Some(message) if self.fail_at.is_none_or(|at| at == index) => Err(message.clone()),
            _ => Ok(()),
        }
    }
}

struct RecordingGit {
    calls: RefCell<Vec<Vec<String>>>,
    replies: RefCell<VecDeque<Result<String, String>>>,
}

impl RecordingGit {
    fn new(replies: Vec<Result<String, String>>) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            replies: RefCell::new(replies.into()),
        }
    }
}

impl Git for RecordingGit {
    fn git(&self, arguments: &[&str], _stdin: &str) -> Result<String, String> {
        self.calls.borrow_mut().push(call(arguments));
        if arguments == ["config", "user.name"] {
            return Ok("owner".to_string());
        }
        self.replies.borrow_mut().pop_front().unwrap()
    }
}

fn acquire_replies() -> Vec<Result<String, String>> {
    vec![
        Ok(String::new()),
        Ok("tree".to_string()),
        Ok(SHA.to_string()),
        Ok(String::new()),
    ]
}

fn release_replies(lease: &Lease) -> Vec<Result<String, String>> {
    vec![
        Ok(format!("{SHA}\trefs/claims/dev\n")),
        Ok(String::new()),
        Ok(lease.message()),
        Ok(String::new()),
    ]
}

fn missing_path(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

#[derive(Default)]
struct Sink(String);

impl std::io::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.push_str(&String::from_utf8_lossy(bytes));
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl std::ops::Deref for Sink {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}

#[test]
fn a_local_acquire_uses_environment_defaults_and_calls_no_gh() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let environment = env(Some("host-a"), Some("run-env"));
    let git = fixture.scratch("cli");
    let gh = RecordingGh::new(None);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&["acquire", fixture.url.as_str(), "task-a", "--issue", "11"]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        0
    );
    assert_eq!(out.trim_end().len(), 40);
    assert_eq!(err.0, "");
    assert!(gh.calls.borrow().is_empty());
    let store = Store {
        git: &git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    let (_, lease) = store.current("task-a").unwrap().unwrap();
    assert_eq!(lease, lease_at("host-a", "run-env", 11, 4_600));
}

#[test]
fn an_acquire_without_environment_uses_the_clock_and_unknown_device() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let environment = env(None, None);
    let git = fixture.scratch("cli");
    let gh = RecordingGh::new(None);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&["acquire", fixture.url.as_str(), "task-b", "--issue", "12"]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        0
    );
    assert_eq!(err.0, "");
    let store = Store {
        git: &git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    let (_, lease) = store.current("task-b").unwrap().unwrap();
    assert_eq!(lease, lease_at("unknown", "1000", 12, 4_600));
}

#[test]
fn a_slug_acquire_calls_gh_with_the_exact_arguments() {
    let git = RecordingGit::new(acquire_replies());
    let gh = RecordingGh::new(None);
    let environment = env(Some("ignored"), Some("ignored"));
    let clock = FixedClock(1_000);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&[
        "acquire",
        "acme/widget",
        "task-1",
        "--issue",
        "7",
        "--lease-seconds",
        "60",
        "--device",
        "agent-1",
        "--run",
        "run-9",
        "--owner",
        "ExtraToast",
        "--label",
        "status: in-progress",
    ]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        0
    );
    assert_eq!(out.0, format!("{SHA}\n"));
    assert_eq!(err.0, "");
    let git_calls = git.calls.borrow();
    assert_eq!(git_calls[0][1], "https://github.com/acme/widget.git");
    drop(git_calls);
    let expected = vec![
        call(&[
            "issue",
            "edit",
            "7",
            "--repo",
            "acme/widget",
            "--add-assignee",
            "ExtraToast",
            "--add-label",
            "status: in-progress",
        ]),
        call(&[
            "issue",
            "comment",
            "7",
            "--repo",
            "acme/widget",
            "--body",
            "Claimed by agent-1, run run-9, lease until 1970-01-01T00:17:40Z",
        ]),
    ];
    assert_eq!(gh.calls.borrow().as_slice(), expected.as_slice());
}

#[test]
fn a_slug_acquire_uses_the_default_owner_and_label() {
    let git = RecordingGit::new(acquire_replies());
    let gh = RecordingGh::new(None);
    let environment = env(None, None);
    let clock = FixedClock(1_000);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&[
        "acquire",
        "acme/widget",
        "task-2",
        "--issue",
        "8",
        "--device",
        "agent-2",
        "--run",
        "run-2",
    ]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        0
    );
    let expected = call(&[
        "issue",
        "edit",
        "8",
        "--repo",
        "acme/widget",
        "--add-assignee",
        "ExtraToast",
        "--add-label",
        "status: in-progress",
    ]);
    assert_eq!(gh.calls.borrow()[0], expected);
}

#[test]
fn an_active_holder_loses_the_acquire_and_reports_its_lease() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let environment = env(None, None);
    let git = fixture.scratch("cli");
    let gh = RecordingGh::new(None);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let first = args(&[
        "acquire",
        fixture.url.as_str(),
        "held",
        "--issue",
        "1",
        "--lease-seconds",
        "60",
        "--device",
        "first",
        "--run",
        "first-run",
    ]);
    assert_eq!(
        run_with(&first, &environment, &git, &gh, &clock, &mut out, &mut err),
        0
    );
    let mut out = Sink::default();
    let mut err = Sink::default();
    let second = args(&[
        "acquire",
        fixture.url.as_str(),
        "held",
        "--issue",
        "2",
        "--lease-seconds",
        "60",
        "--device",
        "second",
        "--run",
        "second-run",
    ]);
    assert_eq!(
        run_with(&second, &environment, &git, &gh, &clock, &mut out, &mut err),
        1
    );
    assert_eq!(out.0, "");
    assert_eq!(
        err.0,
        "held by first run first-run until 1970-01-01T00:17:40Z\n"
    );
    assert!(gh.calls.borrow().is_empty());
}

#[test]
fn an_acquire_runtime_error_is_reported() {
    let path = missing_path("claim-cli-absent-acquire");
    let environment = env(None, None);
    let git = RecordingGit::new(vec![Err("ls-remote failed".to_string())]);
    let gh = RecordingGh::new(None);
    let clock = FixedClock(1_000);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&[
        "acquire",
        path.to_str().unwrap(),
        "key",
        "--issue",
        "1",
        "--device",
        "d",
        "--run",
        "r",
    ]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        1
    );
    assert_eq!(out.0, "");
    assert!(err.starts_with("claim: "), "{}", err.0);
}

#[test]
fn a_gh_failure_stops_the_slug_acquire_and_is_reported() {
    let git = RecordingGit::new(acquire_replies());
    let gh = RecordingGh::new(Some("gh broke"));
    let environment = env(None, None);
    let clock = FixedClock(1_000);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&[
        "acquire",
        "acme/widget",
        "task-3",
        "--issue",
        "9",
        "--device",
        "agent-3",
        "--run",
        "run-3",
    ]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        1
    );
    assert_eq!(err.0, "claim: gh broke\n");
    assert_eq!(gh.calls.borrow().len(), 1);
}

#[test]
fn a_failing_acquire_comment_is_reported() {
    let git = RecordingGit::new(acquire_replies());
    let gh = RecordingGh::failing_at(1, "comment refused");
    let environment = env(None, None);
    let clock = FixedClock(1_000);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&[
        "acquire",
        "acme/widget",
        "task-4",
        "--issue",
        "10",
        "--device",
        "agent-4",
        "--run",
        "run-4",
    ]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        1
    );
    assert_eq!(err.0, "claim: comment refused\n");
    assert_eq!(gh.calls.borrow().len(), 2);
}

#[test]
fn renew_prints_the_new_sha_for_default_and_explicit_options() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let environment = env(Some("host-a"), Some("run-env"));
    let git = fixture.scratch("cli");
    let gh = RecordingGh::new(None);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let acquire = args(&[
        "acquire",
        fixture.url.as_str(),
        "renewed",
        "--issue",
        "3",
        "--device",
        "host-a",
        "--run",
        "run-env",
    ]);
    assert_eq!(
        run_with(
            &acquire,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        0
    );

    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&["renew", fixture.url.as_str(), "renewed"]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        0
    );
    assert_eq!(err.0, "");
    assert_eq!(out.trim_end().len(), 40);
    let store = Store {
        git: &git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    let (_, lease) = store.current("renewed").unwrap().unwrap();
    assert_eq!(lease, lease_at("host-a", "run-env", 3, 4_600));

    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&[
        "renew",
        fixture.url.as_str(),
        "renewed",
        "--lease-seconds",
        "120",
        "--device",
        "host-a",
        "--run",
        "run-env",
    ]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        0
    );
    assert_eq!(out.trim_end().len(), 40);
    let (_, lease) = store.current("renewed").unwrap().unwrap();
    assert_eq!(lease, lease_at("host-a", "run-env", 3, 1_120));
}

#[test]
fn a_renew_runtime_error_is_reported() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let environment = env(None, None);
    let git = fixture.scratch("cli");
    let gh = RecordingGh::new(None);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&["renew", fixture.url.as_str(), "absent"]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        1
    );
    assert_eq!(err.0, "claim: no claim\n");
}

#[test]
fn a_slug_release_calls_gh_with_and_without_a_comment() {
    let environment = env(None, None);
    let clock = FixedClock(1_000);
    let lease = lease_at("agent-1", "run-1", 9, 1_060);

    let git = RecordingGit::new(release_replies(&lease));
    let gh = RecordingGh::new(None);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&[
        "release",
        "acme/widget",
        "dev",
        "--issue",
        "9",
        "--label",
        "review",
        "--comment",
        "done",
    ]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        0
    );
    assert_eq!(out.0, "");
    assert_eq!(err.0, "");
    let expected = vec![
        call(&[
            "issue",
            "edit",
            "9",
            "--repo",
            "acme/widget",
            "--remove-label",
            "review",
        ]),
        call(&[
            "issue",
            "comment",
            "9",
            "--repo",
            "acme/widget",
            "--body",
            "done",
        ]),
    ];
    assert_eq!(gh.calls.borrow().as_slice(), expected.as_slice());

    let git = RecordingGit::new(release_replies(&lease));
    let gh = RecordingGh::new(None);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&["release", "acme/widget", "dev", "--issue", "9"]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        0
    );
    let expected = call(&[
        "issue",
        "edit",
        "9",
        "--repo",
        "acme/widget",
        "--remove-label",
        "status: in-progress",
    ]);
    assert_eq!(gh.calls.borrow().len(), 1);
    assert_eq!(gh.calls.borrow()[0], expected);
}

#[test]
fn a_non_slug_release_makes_no_gh_call() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let environment = env(None, None);
    let git = fixture.scratch("cli");
    let gh = RecordingGh::new(None);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let acquire = args(&[
        "acquire",
        fixture.url.as_str(),
        "dev",
        "--issue",
        "4",
        "--device",
        "d",
        "--run",
        "r",
    ]);
    assert_eq!(
        run_with(
            &acquire,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        0
    );
    let arguments = args(&["release", fixture.url.as_str(), "dev", "--issue", "4"]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        0
    );
    assert!(gh.calls.borrow().is_empty());
    let store = Store {
        git: &git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    assert!(store.current("dev").unwrap().is_none());
}

#[test]
fn a_release_gh_failure_is_reported() {
    let lease = lease_at("agent-1", "run-1", 9, 1_060);
    let git = RecordingGit::new(release_replies(&lease));
    let gh = RecordingGh::new(Some("edit refused"));
    let environment = env(None, None);
    let clock = FixedClock(1_000);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&["release", "acme/widget", "dev", "--issue", "9"]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        1
    );
    assert_eq!(err.0, "claim: edit refused\n");
}

#[test]
fn a_failing_release_comment_is_reported() {
    let lease = lease_at("agent-1", "run-1", 9, 1_060);
    let git = RecordingGit::new(release_replies(&lease));
    let gh = RecordingGh::failing_at(1, "comment refused");
    let environment = env(None, None);
    let clock = FixedClock(1_000);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&[
        "release",
        "acme/widget",
        "dev",
        "--issue",
        "9",
        "--comment",
        "done",
    ]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        1
    );
    assert_eq!(err.0, "claim: comment refused\n");
    assert_eq!(gh.calls.borrow().len(), 2);
}

#[test]
fn a_release_runtime_error_is_reported() {
    let path = missing_path("claim-cli-absent-release");
    let environment = env(None, None);
    let git = RecordingGit::new(vec![Err("ls-remote failed".to_string())]);
    let gh = RecordingGh::new(None);
    let clock = FixedClock(1_000);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&["release", path.to_str().unwrap(), "dev", "--issue", "1"]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        1
    );
    assert!(err.starts_with("claim: "), "{}", err.0);
}

#[test]
fn list_prints_active_and_expired_claims_sorted_by_key() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let environment = env(None, None);
    let git = fixture.scratch("cli");
    let gh = RecordingGh::new(None);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let zeta = args(&[
        "acquire",
        fixture.url.as_str(),
        "zeta",
        "--issue",
        "1",
        "--lease-seconds",
        "10",
        "--device",
        "dev-z",
        "--run",
        "run-z",
    ]);
    assert_eq!(
        run_with(&zeta, &environment, &git, &gh, &clock, &mut out, &mut err),
        0
    );
    let alpha = args(&[
        "acquire",
        fixture.url.as_str(),
        "alpha",
        "--issue",
        "2",
        "--lease-seconds",
        "120",
        "--device",
        "dev-a",
        "--run",
        "run-a",
    ]);
    assert_eq!(
        run_with(&alpha, &environment, &git, &gh, &clock, &mut out, &mut err),
        0
    );

    let later = FixedClock(1_100);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&["list", fixture.url.as_str()]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &later,
            &mut out,
            &mut err
        ),
        0
    );
    assert_eq!(err.0, "");
    assert_eq!(
        out.0,
        "alpha device=dev-a run=run-a issue=2 expires=1970-01-01T00:18:40Z active\n\
         zeta device=dev-z run=run-z issue=1 expires=1970-01-01T00:16:50Z expired\n"
    );
}

#[test]
fn a_list_runtime_error_is_reported() {
    let path = missing_path("claim-cli-absent-list");
    let environment = env(None, None);
    let git = RecordingGit::new(vec![Err("ls-remote failed".to_string())]);
    let gh = RecordingGh::new(None);
    let clock = FixedClock(1_000);
    let mut out = Sink::default();
    let mut err = Sink::default();
    let arguments = args(&["list", path.to_str().unwrap()]);
    assert_eq!(
        run_with(
            &arguments,
            &environment,
            &git,
            &gh,
            &clock,
            &mut out,
            &mut err
        ),
        1
    );
    assert!(err.starts_with("claim: "), "{}", err.0);
}

#[test]
fn every_argument_error_prints_usage_and_exits_two() {
    let environment = env(None, None);
    let git = RecordingGit::new(vec![]);
    let gh = RecordingGh::new(None);
    let clock = FixedClock(1_000);
    let cases = vec![
        args(&[]),
        args(&["nope"]),
        args(&["acquire"]),
        args(&["acquire", "repo"]),
        args(&["renew"]),
        args(&["release"]),
        args(&["list"]),
        args(&["list", "repo", "extra"]),
        args(&["list", "repo", "--device", "d"]),
        args(&["acquire", "repo", "key", "extra", "--issue", "1"]),
        args(&["renew", "repo", "key", "extra"]),
        args(&["acquire", "repo", "key"]),
        args(&["release", "repo", "key"]),
        args(&["release", "repo", "key", "--issue", "1", "--bogus", "x"]),
        args(&["acquire", "repo", "key", "--issue"]),
        args(&["acquire", "repo", "key", "--issue", "--device", "d"]),
        args(&["acquire", "repo", "key", "--issue", "abc"]),
        args(&[
            "acquire",
            "repo",
            "key",
            "--issue",
            "1",
            "--lease-seconds",
            "abc",
        ]),
        args(&["renew", "repo", "key", "--lease-seconds", "abc"]),
        args(&["release", "repo", "key", "--issue", "abc"]),
        args(&["acquire", "repo", "key", "--issue", "1", "--bogus", "x"]),
        args(&["renew", "repo", "key", "--issue", "1"]),
    ];
    for arguments in cases {
        let mut out = Sink::default();
        let mut err = Sink::default();
        assert_eq!(
            run_with(
                &arguments,
                &environment,
                &git,
                &gh,
                &clock,
                &mut out,
                &mut err
            ),
            2,
            "{arguments:?}"
        );
        assert_eq!(out.0, "", "{arguments:?}");
        assert_eq!(err.0, USAGE, "{arguments:?}");
    }
}

#[test]
fn is_slug_accepts_only_one_clean_owner_name_pair() {
    assert!(is_slug("owner/name"));
    assert!(is_slug("owner/repo.rs"));
    for bad in [
        "",
        "owner",
        "/name",
        "owner/",
        "a/b/c",
        "own er/name",
        "owner/na me",
        "owner\t/name",
        "owner:tag/name",
        "owner/name:tag",
        ".owner/name",
        "owner/.name",
    ] {
        assert!(!is_slug(bad), "{bad}");
    }
}

#[cfg(unix)]
static PATH_LOCK: Mutex<()> = Mutex::new(());

#[cfg(unix)]
struct PathGuard {
    original: Option<OsString>,
}

#[cfg(unix)]
impl Drop for PathGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }
}

#[cfg(unix)]
fn fake_bin() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "claim-gh-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = r#"#!/bin/sh
if [ "$1" = "fail" ]; then
  printf 'gh refused\n' >&2
  exit 7
fi
exit 0
"#;
    let path = dir.join("gh");
    std::fs::write(&path, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

#[cfg(unix)]
fn with_path(dir: &std::path::Path, action: impl FnOnce()) {
    let original = std::env::var_os("PATH");
    let _guard = PathGuard {
        original: original.clone(),
    };
    let mut paths = vec![dir.to_path_buf()];
    if let Some(original) = original {
        paths.extend(std::env::split_paths(&original));
    }
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    action();
}

#[test]
#[cfg(unix)]
fn real_gh_returns_success_and_the_stderr_of_a_failure() {
    let _lock = PATH_LOCK.lock().unwrap();
    let dir = fake_bin();
    with_path(&dir, || {
        let gh = RealGh;
        assert_eq!(gh.gh(&["ok"]), Ok(()));
        assert_eq!(gh.gh(&["fail"]), Err("gh refused".to_string()));
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_command_that_cannot_start_reports_a_spawn_failure() {
    let error = run_command(&mut Command::new("claim-no-such-gh-command-545b0c")).unwrap_err();
    assert!(error.starts_with("gh spawn:"), "{error}");
}
