//! Tests for the `run` command through fakes for the client program and for git, so the branches the
//! node goldens do not reach (a worktree that cannot be added or removed, a workflow that cannot be
//! joined, a client that cannot start, a writing run) are pinned too.

use super::machine::{Machine, DEFAULTS};
use crate::adapters::clock::SystemClock;
use crate::adapters::fs::SystemFiles;
use crate::adapters::git::{SystemGit, TIMEOUT_MS};
use crate::error::GatewayError;
use crate::escalation_run::{AttemptError, AttemptedRun};
use crate::json::{parse, Value};
use crate::ports::{Clock, Files, Git, GitRemote, LockTimings, ProcessResult};
use crate::run_command::{RunCommand, RunResult};
use crate::run::{RunMode, RunTokens};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

/// A git whose answers the test chooses; the repository it finds belongs to nedap, which permits Claude Code.
struct FakeGit {
    common: Result<String, String>,
    add: Result<(), String>,
    remove: Result<(), String>,
}

impl Git for FakeGit {
    fn root(&self, path: &str) -> Result<Option<String>, String> {
        Ok(Some(path.to_string()))
    }
    fn remotes(&self, _path: &str) -> Result<Vec<GitRemote>, String> {
        Ok(vec![GitRemote { name: "origin".into(), url: "git@github.com:nedap/x.git".into() }])
    }
    fn remove_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        self.remove.clone()
    }
    fn delete_branch(&self, _repo: &str, _branch: &str) -> Result<(), String> {
        Ok(())
    }
    fn common_dir(&self, _path: &str) -> Result<String, String> {
        self.common.clone()
    }
    fn add_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        self.add.clone()
    }

    fn git_dir(&self, _path: &str) -> Result<String, String> {
        unreachable!("the test asks for no git directory")
    }

    fn create_worktree(&self, _repo: &str, _path: &str, _branch: &str, _base: &str) -> Result<(), String> {
        unreachable!("the test creates no worktree")
    }

    fn stage_all(&self, _path: &str) -> Result<(), String> {
        unreachable!("the test stages nothing")
    }

    fn staged_files(&self, _path: &str, _base: Option<&str>) -> Result<Vec<String>, String> {
        unreachable!("the test lists nothing")
    }

    fn list_worktrees(&self, _repo: &str) -> Result<Vec<String>, String> {
        unreachable!("the test watches no worktree")
    }

    fn status(&self, _path: &str) -> Result<String, String> {
        unreachable!("the test reads no status")
    }
}

fn git(common: Result<String, String>, add: Result<(), String>, remove: Result<(), String>) -> Box<FakeGit> {
    Box::new(FakeGit { common, add, remove })
}

const CLAUDE_ANSWER: &str = r#"{"type":"result","result":"done\nSUMMARY: fine","usage":{"input_tokens":10,"output_tokens":5}}"#;

fn answer(stdout: &str) -> Result<ProcessResult, String> {
    Ok(ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code: 0, timed_out: false })
}

/// A machine with the claude client installed and answering, and a repository in its work directory.
fn machine() -> Machine {
    let machine = Machine::new(DEFAULTS);
    machine.process.install("claude");
    machine.process.install("vibe");
    machine.process.answer("claude", answer(CLAUDE_ANSWER));
    std::fs::create_dir_all(format!("{}/tmp", machine.sandbox.text())).expect("tmp");
    machine
}

fn with_tmp(mut machine: Machine) -> Machine {
    let tmp = format!("{}/tmp", machine.sandbox.text());
    machine.runtime.env.insert("TMPDIR".to_string(), tmp);
    machine
}

fn run(machine: &Machine, input: &str) -> Result<Value, GatewayError> {
    let _held = super::scenarios::EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    RunCommand { runtime: &machine.runtime }.run(&parse(input).expect("JSON"))
}

fn repo_run(extra: &str) -> String {
    format!(r#"{{"workflow":"w","client":"claude","mode":"read-only","model":"claude-sonnet-5-5","prompt":"p","repo":"/repo","escalate":false{extra}}}"#)
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.as_object().and_then(|object| object.get(key)).and_then(Value::as_str).unwrap_or_default()
}

#[test]
fn a_client_the_configuration_does_not_declare_is_refused() {
    let machine = machine();
    let error = run(&machine, r#"{"workflow":"w","client":"agy","mode":"read-only","model":"m","prompt":"p","escalate":false}"#).expect_err("unknown");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("unknown-model", "the gateway knows no client agy"));
}

#[test]
fn a_throwaway_worktree_that_cannot_be_added_is_a_worktree_failure_and_leaves_no_directory() {
    let mut machine = with_tmp(machine());
    machine.runtime.git = git(Ok(format!("{}/common", machine.sandbox.text())), Err("no room".into()), Ok(()));
    let error = run(&machine, &repo_run("")).expect_err("worktree failure");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("worktree-failed", "/repo: cannot add a throwaway worktree: no room"));
    let left: Vec<_> = std::fs::read_dir(format!("{}/tmp", machine.sandbox.text())).expect("tmp").flatten().collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn a_throwaway_worktree_that_cannot_be_removed_is_a_warning_on_a_run_that_finished() {
    let mut machine = with_tmp(machine());
    machine.runtime.git = git(Ok(format!("{}/common", machine.sandbox.text())), Ok(()), Err("busy".into()));
    let result = run(&machine, &repo_run("")).expect("the run finishes");
    assert_eq!(text(&result, "status"), "ok");
    assert!(text(&result, "cleanup_warning").starts_with("cannot remove the throwaway worktree "), "{result:?}");
    assert!(text(&result, "cleanup_warning").ends_with(": busy"));
    let saved = std::fs::read_to_string(format!("{}/common/model-gateway/runs/{}/result.json", machine.sandbox.text(), text(&result, "run_id"))).expect("result.json");
    assert!(saved.contains("cleanup_warning"));
}

#[test]
fn a_repository_whose_common_directory_cannot_be_read_is_unreadable() {
    let mut machine = with_tmp(machine());
    machine.runtime.git = git(Err("bad".into()), Ok(()), Ok(()));
    let error = run(&machine, &repo_run("")).expect_err("unreadable");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("repository-unreadable", "/repo: bad"));
}

#[test]
fn a_client_that_cannot_start_is_a_failed_run_with_the_platforms_words() {
    let machine = with_tmp(machine());
    machine.process.answer("claude", Err("spawn claude ENOENT".to_string()));
    let result = run(&machine, r#"{"workflow":"w","client":"claude","mode":"read-only","model":"claude-sonnet-5-5","prompt":"p","escalate":false}"#).expect("a result");
    assert_eq!((text(&result, "status"), text(&result, "reason")), ("failed", "launch-failed"));
    let stderr = std::fs::read_to_string(format!("{}/state/agent-kit/model-gateway/runs/{}/stderr.txt", machine.sandbox.text(), text(&result, "run_id"))).expect("stderr");
    assert_eq!(stderr, "spawn claude ENOENT");
}

/// A clock that moves 100 ms with every reading, so a wait with a deadline ends.
struct Ticking(AtomicI64);

impl Clock for Ticking {
    fn now_ms(&self) -> i64 {
        self.0.fetch_add(100, Ordering::SeqCst) + 100
    }
    fn monotonic_ms(&self) -> f64 {
        0.0
    }
}

#[test]
fn a_workflow_that_cannot_be_joined_gives_back_the_reservation_and_fails_with_the_platforms_words() {
    let mut machine = with_tmp(machine());
    machine.write_override(r#"{"clients":{"hermes":{"providers":["anthropic"]}}}"#);
    machine.process.install("hermes");
    machine.process.answer("hermes", answer("ok"));
    machine.runtime.clock = Box::new(Ticking(AtomicI64::new(super::machine::now_ms())));
    machine.runtime.lock = LockTimings { retry_ms: 1, stale_ms: 600_000_000, timeout_ms: 300 };
    let lock = format!("{}/state/agent-kit/model-gateway/workflows.json.lock", machine.sandbox.text());
    machine.write_at(&lock, "");
    let held = r#"{"workflow":"w","client":"hermes","mode":"read-only","model":"hermes-hard","provider":"anthropic","prompt":"p","escalate":false}"#;
    let error = run(&machine, held).expect_err("the lock is held");
    assert_eq!(error.code, "unexpected");
    assert!(error.message.contains("workflows lock"), "{}", error.message);
    assert!(machine.ledger_text().contains(r#""reason":"not-started""#), "{}", machine.ledger_text());
}

#[test]
fn a_run_result_says_what_the_escalation_asks_and_prints_every_optional_member_in_order() {
    let mut result = RunResult {
        ok: false,
        run_id: "r".into(),
        workflow: "w".into(),
        client: "claude".into(),
        model: "m".into(),
        family: "anthropic".into(),
        mode: RunMode::ReadOnly,
        tokens: RunTokens { cached: 0.0, input: 1.0, output: 2.0 },
        cost_usd: 0.5,
        output_path: "/o".into(),
        summary: "s".into(),
        reason: Some("exit-1".into()),
        unavailable_until: Some("2026-11-04T10:00:00.000Z".into()),
        write: None,
        attempt: Some(2),
        attempt_of: Some("r0".into()),
        cleanup_warning: Some("w".into()),
        warnings: vec!["l".into()],
    };
    assert!(!result.is_ok() && result.check_output_path().is_none());
    assert_eq!((result.family(), result.summary()), (Some("anthropic"), Some("s")));
    result.set_summary("t".into());
    let keys: Vec<String> = match result.to_json() {
        Value::Obj(object) => object.entries().into_iter().map(|(key, _)| key.to_string()).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        keys.join(","),
        "client,cost_usd,family,mode,model,output_path,run_id,status,summary,tokens,workflow,reason,unavailable_until,attempt,attempt_of,cleanup_warning,warnings"
    );
}

#[test]
fn a_platform_failure_is_what_went_wrong_and_a_refusal_stays_a_refusal() {
    assert_eq!(AttemptError::from(GatewayError::new("unexpected", "disk")), AttemptError::Other("disk".into()));
    let refused = GatewayError::new("no-grant", "m");
    assert_eq!(AttemptError::from(refused.clone()), AttemptError::Refused(refused));
}

#[test]
fn the_real_clock_reads_a_monotonic_time_that_does_not_go_back() {
    let first = SystemClock.monotonic_ms();
    assert!(SystemClock.monotonic_ms() >= first);
}

#[test]
fn the_real_files_write_text_make_temporary_directories_and_remove_what_is_there() {
    let sandbox = super::scenarios::Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    files.write_text("a/b/c.txt", "hi").expect("write");
    assert_eq!(files.read_text("a/b/c.txt"), Ok("hi".to_string()));
    let dir = files.temp_dir("tmpparent", "model-gateway-run-").expect_err("the parent is not there");
    assert_eq!(dir.code, "ENOENT");
    files.mkdir_all("tmpparent").expect("mkdir");
    let made = files.temp_dir("tmpparent", "model-gateway-run-").expect("a directory");
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&made).expect("made").permissions().mode() & 0o777, 0o700);
    }
    let name = made.rsplit('/').next().expect("a name");
    assert!(name.starts_with("model-gateway-run-") && name.len() == "model-gateway-run-".len() + 6 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    assert!(std::path::Path::new(&made).is_dir());
    files.remove("tmpparent").expect("a directory goes");
    files.remove("a/b/c.txt").expect("a file goes");
    files.remove("a/b/c.txt").expect("a missing path is already gone");
    // A path below a file cannot be looked at.
    files.write_text("plain", "x").expect("write");
    assert_eq!(files.remove("plain/below").expect_err("not a directory").code, "ENOTDIR");
}

#[test]
fn the_real_git_answers_for_the_common_directory_and_a_worktree_and_fails_in_words_outside_one() {
    let sandbox = super::scenarios::Sandbox::new();
    let env = HashMap::from([("PATH".to_string(), std::env::var("PATH").unwrap_or_default()), ("HOME".to_string(), sandbox.text())]);
    let real = SystemGit { env, timeout_ms: TIMEOUT_MS };
    let outside = format!("{}/outside", sandbox.text());
    std::fs::create_dir_all(&outside).expect("outside");
    assert!(real.common_dir(&outside).is_err());
    assert!(real.add_worktree(&outside, &format!("{outside}/wt")).is_err());
}

#[test]
fn the_real_git_adds_a_worktree_of_head_and_names_the_common_directory() {
    let sandbox = super::scenarios::Sandbox::new();
    let env = HashMap::from([("PATH".to_string(), std::env::var("PATH").unwrap_or_default()), ("HOME".to_string(), sandbox.text())]);
    let real = SystemGit { env, timeout_ms: TIMEOUT_MS };
    let repo = format!("{}/repo", sandbox.text());
    std::fs::create_dir_all(&repo).expect("repo");
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", "-C", &repo])
            .args(args)
            .env("HOME", sandbox.text())
            .status()
            .expect("git");
        assert!(status.success(), "{args:?}");
    };
    git(&["init", "-q"]);
    git(&["commit", "-q", "--allow-empty", "-m", "x"]);
    assert_eq!(real.common_dir(&repo).expect("common dir"), format!("{repo}/.git"));
    let tree = format!("{}/tree", sandbox.text());
    real.add_worktree(&repo, &tree).expect("added");
    assert!(std::path::Path::new(&tree).join(".git").exists());
}

#[test]
fn temporary_directories_go_where_tmpdir_names_else_tmp_and_a_trailing_slash_is_dropped() {
    // No TMPDIR: the system's own, which the run leaves empty again.
    let machine = machine();
    let result = run(&machine, r#"{"workflow":"w","client":"claude","mode":"read-only","model":"claude-sonnet-5-5","prompt":"p","escalate":false}"#).expect("a result");
    assert_eq!(text(&result, "status"), "ok");
    let mut slashed = with_tmp(self::machine());
    let tmp = format!("{}/tmp/", slashed.sandbox.text());
    slashed.runtime.env.insert("TMPDIR".to_string(), tmp);
    run(&slashed, r#"{"workflow":"w","client":"claude","mode":"read-only","model":"claude-sonnet-5-5","prompt":"p","escalate":false}"#).expect("a result");
    // The root names itself.
    let mut root = self::machine();
    root.runtime.env.insert("TMPDIR".to_string(), "///".to_string());
    root.runtime.env.insert("TMP".to_string(), "/never".to_string());
    let error = run(&root, r#"{"workflow":"w","client":"claude","mode":"read-only","model":"claude-sonnet-5-5","prompt":"p","escalate":false}"#);
    if let Err(error) = error {
        assert!(error.message.contains("mkdtemp '/model-gateway-"), "{}", error.message);
    }
}

#[test]
fn a_client_a_run_cannot_launch_is_a_launch_failure() {
    let machine = machine();
    let input = crate::run::read_run_input(&parse(r#"{"workflow":"w","client":"claude","mode":"read-only","model":"m","prompt":"p"}"#).expect("JSON")).expect("input");
    let input = crate::run::RunInput { client: "foo".into(), ..input };
    let paths = crate::launchers::LaunchPaths::default();
    let (result, _output, failure) = RunCommand { runtime: &machine.runtime }.launch("foo", &input, &paths, "p");
    assert_eq!((failure.as_deref(), result.exit_code), (Some("launch-failed"), 1));
    assert_eq!(result.stderr, "foo is not a client a run can launch");
}

#[test]
fn write_text_fails_in_the_platforms_words_below_a_file_and_on_a_directory() {
    let sandbox = super::scenarios::Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    files.write_text("plain", "x").expect("write");
    assert_eq!(files.write_text("plain/below", "y").expect_err("not a directory").code, "EEXIST");
    files.mkdir_all("dir").expect("dir");
    assert_eq!(files.write_text("dir", "y").expect_err("a directory").code, "EISDIR");
}

/// A metered-employer run's input, as a batch's dependent sends it.
fn codex_input(time_limit_s: u32) -> crate::run::RunInput {
    let raw = format!(r#"{{"workflow":"w","client":"codex","mode":"read-only","model":"gpt-5.5","prompt":"p","time_limit_s":{time_limit_s}}}"#);
    crate::run::read_run_input(&parse(&raw).expect("JSON")).expect("an input")
}

#[test]
fn a_dependent_that_starts_later_needs_its_grant_to_still_stand_and_is_held_to_what_is_left() {
    let machine = machine();
    let command = RunCommand { runtime: &machine.runtime };

    // Every grant lapsed, or there never was one.
    assert_eq!(command.within_grant(&codex_input(900)).expect_err("no grant").code, "no-grant");

    // Under a minute left on it.
    machine.write_grants(&[super::machine::grant(5.0, 30.0)]);
    let error = command.within_grant(&codex_input(900)).expect_err("expiring");
    assert_eq!(error.code, "grant-expiring");
    assert!(error.message.contains("has 30 seconds left, under the 60 a run needs"), "{}", error.message);

    // More time than the run asks for: it keeps its own limit.
    machine.write_grants(&[super::machine::grant(5.0, 7200.0)]);
    assert_eq!(command.within_grant(&codex_input(900)).expect("held").time_limit_s, 900.0);

    // Less than the run asks for: it is held to what is left.
    machine.write_grants(&[super::machine::grant(5.0, 120.0)]);
    assert_eq!(command.within_grant(&codex_input(900)).expect("held").time_limit_s, 120.0);
}
