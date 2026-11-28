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
use crate::ports::{Clock, FileStamp, Files, FsError, Git, GitRemote, LockTimings, ProcessResult};
use crate::run_command::{RunCommand, RunResult};
use crate::run::{RunMode, RunTokens};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

/// A git whose answers the test chooses; the repository it finds is a private one whose allowlist entry permits
/// every family.
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
        Ok(vec![GitRemote { name: "origin".into(), url: "git@github.com:acme/x.git".into() }])
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
    machine.allow_everything("github.com/acme/x");
    machine
}

fn with_tmp(mut machine: Machine) -> Machine {
    let tmp = format!("{}/tmp", machine.sandbox.text());
    machine.runtime.env.insert("TMPDIR".to_string(), tmp);
    machine
}

fn run(machine: &Machine, input: &str) -> Result<Value, GatewayError> {
    let _held = super::scenarios::EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    RunCommand { runtime: &machine.runtime, decision: None }.run(&parse(input).expect("JSON"))
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
fn a_provider_client_with_no_provider_configured_is_refused_whichever_provider_it_names() {
    let machine = machine();
    let hermes = |extra: &str| format!(r#"{{"workflow":"w","client":"hermes","mode":"read-only","model":"hermes-hard","prompt":"p","escalate":false{extra}}}"#);
    let error = run(&machine, &hermes("")).expect_err("no provider named");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("provider-not-allowed", "a hermes run must name its provider: this machine allows no provider"));
    machine.write_override(r#"{"clients":{"hermes":{"providers":[]}}}"#);
    let error = run(&machine, &hermes(r#","provider":"anthropic""#)).expect_err("no provider allowed");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("provider-not-allowed", "hermes may not run on anthropic: this machine allows no provider"));
}

/// The real files, except that nothing can be removed.
struct Unremovable(SystemFiles);

impl Files for Unremovable {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        self.0.read_text(path)
    }
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_atomic(path, content)
    }
    fn mkdir_all(&self, path: &str) -> Result<(), FsError> {
        self.0.mkdir_all(path)
    }
    fn create_exclusive(&self, path: &str) -> Result<bool, FsError> {
        self.0.create_exclusive(path)
    }
    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        self.0.remove_file(path)
    }
    fn modified_ms(&self, path: &str) -> Option<i64> {
        self.0.modified_ms(path)
    }
    fn exists(&self, path: &str) -> Result<bool, FsError> {
        self.0.exists(path)
    }
    fn append(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.append(path, content)
    }
    fn stamp(&self, path: &str) -> Option<FileStamp> {
        self.0.stamp(path)
    }
    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_private(path, content)
    }
    fn write_text(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_text(path, content)
    }
    fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, FsError> {
        self.0.temp_dir(parent, prefix)
    }
    fn remove(&self, _path: &str) -> Result<(), FsError> {
        Err(FsError { code: "EBUSY".into(), message: "EBUSY: resource busy or locked".into() })
    }
}

#[test]
fn a_throwaway_directory_that_cannot_be_removed_is_a_warning_on_a_run_that_finished() {
    let mut machine = with_tmp(machine());
    machine.runtime.files = Box::new(Unremovable(SystemFiles { cwd: machine.sandbox.text() }));
    let result = run(&machine, r#"{"workflow":"w","client":"claude","mode":"read-only","model":"claude-sonnet-5-5","prompt":"p","escalate":false}"#).expect("the run finishes");
    assert_eq!(text(&result, "status"), "ok", "{result:?}");
    let warning = text(&result, "cleanup_warning");
    assert!(warning.starts_with(&format!("cannot remove the throwaway worktree {}/tmp/", machine.sandbox.text())), "{warning}");
    assert!(warning.ends_with(": EBUSY: resource busy or locked"), "{warning}");
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
        exit_code: None,
        signal: Some(9),
        stderr_tail: Some("killed".into()),
        failure: Some(crate::run_failure::Failure { kind: crate::run_failure::FailureKind::Signalled, reason: "the client was ended by SIGKILL".into(), output_tail: None }),
        cleanup_warning: Some("w".into()),
        warnings: vec!["l".into()],
        fallback: None,
        retry: None,
        rewrite: None,
        routing: crate::routing::Routing::unseen("claude"),
    };
    assert!(!result.is_ok() && result.check_output_path().is_none());
    assert_eq!((result.family(), result.summary()), (Some("anthropic"), Some("s")));
    result.set_summary("t".into());
    // What the run said when no free copy was available is what the fallback reads.
    assert_eq!(result.free_refusal(), None);
    result.stderr_tail = Some("free limits [limiter refusal, no provider was called]: back in 90s".into());
    assert_eq!(result.free_refusal().as_deref(), Some("free limits [limiter refusal, no provider was called]: back in 90s"));
    result.stderr_tail = Some("killed".into());
    let keys: Vec<String> = match result.to_json() {
        Value::Obj(object) => object.entries().into_iter().map(|(key, _)| key.to_string()).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        keys.join(","),
        "client,cost_usd,family,mode,model,output_path,run_id,status,summary,tokens,workflow,reason,unavailable_until,exit_code,signal,stderr_tail,failure,attempt,attempt_of,cleanup_warning,warnings"
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
    let ((result, ending), _output, failure, _warnings, routing) = RunCommand { runtime: &machine.runtime, decision: None }.launch("foo", &input, &paths, "p", (None, false, Default::default()), &crate::ports::Unwatched);
    assert_eq!(ending, crate::ports::Ending::default());
    assert_eq!((failure.as_deref(), result.exit_code), (Some("launch-failed"), 1));
    assert_eq!(result.stderr, "foo is not a client a run can launch");
    assert_eq!(routing.capture, crate::routing::Capture::Hidden);
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
    let command = RunCommand { runtime: &machine.runtime, decision: None };

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

/// The shipped defaults, with the `cheap-research` profile listing one skill.
fn defaults_with_a_skill() -> String {
    let shipped = crate::SHIPPED_DEFAULTS;
    let profile = shipped.find("\"cheap-research\"").expect("the seed profile");
    let skills = profile + shipped[profile..].find("\"skills\": []").expect("its skills");
    format!("{}\"skills\": [\"lint\"]{}", &shipped[..skills], &shipped[skills + "\"skills\": []".len()..])
}

const PI_STREAM: &str = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Looked.\nSUMMARY: fine"}],"stopReason":"stop","usage":{"input":100,"output":40,"cacheRead":30,"cost":{"total":0.002}}}}"#;

/// A machine with Pi installed behind a launcher, its home holding the route's provider and a `lint` and an `other` skill.
fn pi_machine() -> Machine {
    let mut machine = Machine::new(&defaults_with_a_skill());
    machine.process.install("pi");
    machine.process.answer("pi", answer(PI_STREAM));
    let home = format!("{}/home/.pi/agent", machine.sandbox.text());
    std::fs::create_dir_all(format!("{home}/skills/lint")).expect("lint");
    std::fs::create_dir_all(format!("{home}/skills/other")).expect("other");
    std::fs::write(format!("{home}/skills/lint/SKILL.md"), "lint").expect("lint skill");
    std::fs::write(format!("{home}/skills/other/SKILL.md"), "other").expect("other skill");
    std::fs::write(
        format!("{home}/models.json"),
        r#"{"providers":{"litellm-paid":{"api":"openai-completions","apiKey":"literal","baseUrl":"https://litellm.example/v1","models":[{"id":"paid/ministral-8b"}]}}}"#,
    )
    .expect("models");
    machine.runtime.env.insert("AGENT_KIT_SANDBOX".to_string(), "host".to_string());
    machine.allow_everything("github.com/acme/x");
    machine
}

fn pi_run(extra: &str) -> String {
    format!(r#"{{"workflow":"w","client":"pi-paid","mode":"read-only","model":"paid/ministral-8b","prompt":"p","escalate":false,"profile":"cheap-research"{extra}}}"#)
}

#[test]
fn a_pi_run_under_a_profile_that_lists_a_skill_runs_and_writes_the_scope_on_its_ledger_line() {
    let machine = pi_machine();

    let result = run(&machine, &pi_run("")).expect("the run");

    assert_eq!(text(&result, "status"), "ok");
    let ledger = std::fs::read_to_string(format!("{}/state/agent-kit/model-gateway/ledger.jsonl", machine.sandbox.text())).expect("ledger");
    assert!(ledger.contains(r#""profile":"cheap-research","tool_scope":{"mcp":[],"plugins":[],"skills":["lint"]}"#), "{ledger}");
    // A request that narrows the skill away runs with the empty scope, and the line says so.
    let narrowed = run(&machine, &pi_run(r#","narrow":{"skills":[]}"#)).expect("the narrowed run");
    assert_eq!(text(&narrowed, "status"), "ok");
    let ledger = std::fs::read_to_string(format!("{}/state/agent-kit/model-gateway/ledger.jsonl", machine.sandbox.text())).expect("ledger");
    assert!(ledger.contains(r#""tool_scope":{"mcp":[],"plugins":[],"skills":[]}"#));
}

#[test]
fn a_pi_run_whose_profile_lists_a_skill_the_home_lacks_is_refused_before_anything_runs() {
    let machine = pi_machine();
    std::fs::remove_dir_all(format!("{}/home/.pi/agent/skills/lint", machine.sandbox.text())).expect("remove the skill");

    let error = run(&machine, &pi_run("")).expect_err("a skill the home lacks");

    assert_eq!(error.code, "profile-refused");
    assert!(error.message.starts_with("profile cheap-research cannot run on pi-paid: skill lint is not in Pi's home: looked for "), "{}", error.message);
    assert!(!std::path::Path::new(&format!("{}/state/agent-kit/model-gateway/ledger.jsonl", machine.sandbox.text())).exists());
}

fn pi_free_machine(cost: &str) -> Machine {
    let machine = pi_machine();
    std::fs::write(
        format!("{}/home/.pi/agent/models.json", machine.sandbox.text()),
        format!(r#"{{"providers":{{"litellm-paid":{{"api":"openai-completions","apiKey":"literal","baseUrl":"https://litellm.example/v1","models":[{{"id":"paid/ministral-8b"{cost}}}]}}}}}}"#),
    )
    .expect("models");
    machine
}

#[test]
fn a_free_only_run_on_a_model_that_is_not_free_is_refused_before_anything_runs() {
    for cost in ["", r#","cost":{"input":0,"output":0.2}"#] {
        let machine = pi_free_machine(cost);

        let error = run(&machine, &pi_run(r#","free_only":true"#)).expect_err("a model that is not free");

        assert_eq!(error.code, "free-only-refused");
        assert!(error.message.starts_with("pi-paid:paid/ministral-8b is not free"), "{}", error.message);
        assert!(!std::path::Path::new(&format!("{}/state/agent-kit/model-gateway/ledger.jsonl", machine.sandbox.text())).exists());
    }
}

#[test]
fn a_free_only_run_on_a_free_model_runs_and_a_run_that_does_not_ask_is_unchanged() {
    let free = pi_free_machine(r#","cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}"#);
    assert_eq!(text(&run(&free, &pi_run(r#","free_only":true"#)).expect("the free run"), "status"), "ok");
    let paid = pi_free_machine(r#","cost":{"input":1,"output":0}"#);
    assert_eq!(text(&run(&paid, &pi_run("")).expect("the run that did not ask"), "status"), "ok");
    let error = run(&paid, &pi_run(r#","free_only":"yes""#)).expect_err("not a flag");
    assert_eq!(error.code, "invalid-input");
}

fn with_vibe_catalogue(machine: &Machine, free: bool) {
    let dir = format!("{}/home/.cache/agent-kit", machine.sandbox.text());
    std::fs::create_dir_all(&dir).expect("cache");
    std::fs::write(
        format!("{dir}/litellm-catalogue.json"),
        format!(
            r#"{{"updated_at":"2026-11-03T11:17:01Z","providers":{{"vibe":{{"models":[{{"id":"vibe/ministral-8b","metadata":{{"status":"usable","free":{free},"modality":"chat"}}}}]}}}},"groups":{{"paid/ministral-8b":{{"modality":"chat","deployments":[{{"route":"vibe","upstream_id":"ministral-8b","order":1}}]}}}}}}"#
        ),
    )
    .expect("catalogue");
}

#[test]
fn a_free_only_run_follows_the_catalogues_free_flag_not_the_price_in_the_pi_file() {
    // Free by the catalogue with no price in models.json: accepted.
    let flagged = pi_free_machine("");
    with_vibe_catalogue(&flagged, true);
    assert_eq!(text(&run(&flagged, &pi_run(r#","free_only":true"#)).expect("the flagged run"), "status"), "ok");
    // Billed by the catalogue though models.json prices it at zero: refused.
    let billed = pi_free_machine(r#","cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}"#);
    with_vibe_catalogue(&billed, false);
    let error = run(&billed, &pi_run(r#","free_only":true"#)).expect_err("a billed model");
    assert_eq!(error.code, "free-only-refused");
}

const ZERO: &str = r#","cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}"#;

#[test]
fn openrouter_stays_refused_on_every_client() {
    let machine = machine();
    let other = |client: &str, model: &str, extra: &str| {
        format!(r#"{{"workflow":"w","client":"{client}","mode":"read-only","model":"{model}","prompt":"p","escalate":false{extra}}}"#)
    };
    let hermes = run(&machine, &other("hermes", "m", r#","provider":"openrouter""#)).expect_err("hermes on openrouter");
    assert_eq!(hermes.code, "provider-not-allowed");
    assert!(hermes.message.starts_with("openrouter is not allowed on hermes: a free model runs only as free/<model> on pi-free"), "{}", hermes.message);
    let named = run(&machine, &other("claude", "openrouter/free", "")).expect_err("a model named openrouter/");
    assert_eq!(named.code, "provider-not-allowed");
}

#[test]
fn the_free_client_runs_free_models_only() {
    let machine = pi_machine();
    std::fs::write(
        format!("{}/home/.pi/agent/models.json", machine.sandbox.text()),
        format!(r#"{{"providers":{{"litellm-free":{{"api":"openai-completions","apiKey":"literal","baseUrl":"https://litellm.example/v1","models":[{{"id":"ministral-8b"{ZERO}}},{{"id":"mistral-small"}}]}}}}}}"#),
    )
    .expect("models");
    let free = |model: &str| format!(r#"{{"workflow":"w","client":"pi-free","mode":"read-only","model":"{model}","prompt":"p","escalate":false}}"#);
    assert_eq!(text(&run(&machine, &free("ministral-8b")).expect("free"), "status"), "ok");
    assert_eq!(run(&machine, &free("mistral-small")).expect_err("billed").code, "free-only-refused");
}

fn events_of(machine: &Machine, result: &Value) -> String {
    std::fs::read_to_string(format!("{}/state/agent-kit/model-gateway/runs/{}/events.jsonl", machine.sandbox.text(), text(result, "run_id"))).expect("events")
}

#[test]
fn a_direct_go_model_runs_as_the_litellm_one_and_the_result_and_events_say_so() {
    let machine = with_tmp(machine());
    machine.process.install("opencode");
    machine.write_override(r#"{"clients": {"opencode": {"provider_cost_classes": {"litellm-opencode-go": "subsidised"}}}}"#);
    machine.process.answer("opencode", answer("opencode-go/glm-5.3-flash\nlitellm-opencode-go/glm-5.3-flash\n"));
    let result = run(&machine, r#"{"workflow":"w","client":"opencode","mode":"read-only","model":"opencode-go/glm-5.3-flash","prompt":"p","escalate":false}"#).expect("a result");
    assert_eq!(text(&result, "model"), "litellm-opencode-go/glm-5.3-flash");
    let rewritten = result.as_object().and_then(|object| object.get("rewritten")).and_then(Value::as_object).expect("rewritten");
    assert_eq!(rewritten.get("from").and_then(Value::as_str), Some("opencode-go/glm-5.3-flash"));
    assert_eq!(rewritten.get("to").and_then(Value::as_str), Some("litellm-opencode-go/glm-5.3-flash"));
    assert!(events_of(&machine, &result).contains(r#""type":"route.rewritten""#));
}

#[test]
fn a_direct_go_model_with_no_litellm_copy_is_refused_before_anything_runs() {
    let machine = with_tmp(machine());
    machine.process.install("opencode");
    machine.process.answer("opencode", answer("opencode-go/glm-5.3-flash\n"));
    let refused = run(&machine, r#"{"workflow":"w","client":"opencode","mode":"read-only","model":"opencode-go/glm-5.3-flash","prompt":"p","escalate":false}"#).expect_err("refused");
    assert_eq!(refused.code, "provider-not-allowed");
    assert!(refused.message.contains("litellm-opencode-go/glm-5.3-flash"));
}

#[test]
fn a_batch_plan_checks_a_direct_go_model_as_the_litellm_one() {
    let machine = with_tmp(machine());
    machine.process.install("opencode");
    machine.write_override(r#"{"clients": {"opencode": {"provider_cost_classes": {"litellm-opencode-go": "subsidised"}}}}"#);
    machine.process.answer("opencode", answer("litellm-opencode-go/glm-5.3-flash\n"));
    let input = crate::run::read_run_input(&parse(r#"{"workflow":"w","client":"opencode","mode":"read-only","model":"opencode-go/glm-5.3-flash","prompt":"p"}"#).expect("JSON")).expect("input");
    let checked = RunCommand { runtime: &machine.runtime, decision: None }.check(&input).expect("checked");
    assert_eq!(checked.family, "zhipu");
}

#[test]
fn the_free_fallback_an_attempt_is_handed_is_in_its_events_and_its_result() {
    let machine = with_tmp(machine());
    let fallback = crate::free_fallback::FallbackReport {
        from: "pi-free free/x".into(),
        to: "claude claude-sonnet-5-5".into(),
        reason: "limiter refusal, back in 90s".into(),
        from_run_id: "r0".into(),
        available_again_in_s: Some(90),
        available_at: Some("2026-11-04T10:01:30.000Z".into()),
        recommendation: "name tier-free".into(),
    };
    let input = crate::run::read_run_input(&parse(r#"{"workflow":"w","client":"claude","mode":"read-only","model":"claude-sonnet-5-5","prompt":"p","escalate":false}"#).expect("JSON")).expect("input");
    let options = super::super::run_command::RunOptions { link: None, start_check: None, reservation: None, tag: None, fallback: Some(fallback), retry: None };
    let result = {
        let _held = super::scenarios::EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        RunCommand { runtime: &machine.runtime, decision: None }.run_once(&input, &options).expect("a run")
    };
    let json = result.to_json();
    assert!(text(&json, "summary").starts_with("fell back from pi-free free/x to claude claude-sonnet-5-5: "));
    let object = json.as_object().expect("an object");
    assert_eq!(object.get("available_again_in_s"), Some(&Value::Num(90.0)));
    assert_eq!(object.get("recommendation").and_then(Value::as_str), Some("name tier-free"));
    assert!(object.get("fallback").is_some());
    let events = events_of(&machine, &json);
    assert!(events.contains(r#""type":"fallback""#) && events.contains("limiter refusal, back in 90s"));
}

#[test]
fn a_retry_an_attempt_is_handed_is_in_its_events_and_its_result() {
    let machine = with_tmp(machine());
    let report = crate::retry_wait::Waiter::new(crate::retry_wait::RetryWaitSettings::default())
        .next("r0", "429: every free deployment of x is limited; the first is back in 5s", 0)
        .expect("a wait")
        .report;
    let input = crate::run::read_run_input(&parse(r#"{"workflow":"w","client":"claude","mode":"read-only","model":"claude-sonnet-5-5","prompt":"p","escalate":false}"#).expect("JSON")).expect("input");
    let options = super::super::run_command::RunOptions { link: None, start_check: None, reservation: None, tag: None, fallback: None, retry: Some(report) };
    let result = {
        let _held = super::scenarios::EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        RunCommand { runtime: &machine.runtime, decision: None }.run_once(&input, &options).expect("a run")
    };
    let json = result.to_json();
    let retry = json.as_object().and_then(|object| object.get("retry")).and_then(Value::as_object).expect("the retry object");
    assert_eq!(retry.get("retry"), Some(&Value::Num(1.0)));
    assert_eq!(retry.get("of_run_id").and_then(Value::as_str), Some("r0"));
    let events = events_of(&machine, &json);
    assert!(events.contains(r#""type":"retry""#) && events.contains("back in 5s"), "{events}");
    // No routing was seen of a stub client, so the result has none to say.
    assert!(json.as_object().is_some_and(|object| object.get("routing").is_none()));
}

#[test]
fn a_result_that_was_watched_says_which_route_served_it() {
    let mut result = RunResult { routing: crate::routing::Routing::of(&[crate::routing::Seen { model_id: Some("d2".into()), attempted_fallbacks: Some(1), ..Default::default() }]), ..a_result() };
    let json = result.to_json();
    let routing = json.as_object().and_then(|object| object.get("routing")).and_then(Value::as_object).expect("the routing object");
    assert_eq!(routing.get("litellm_model_id").and_then(Value::as_str), Some("d2"));
    assert_eq!(routing.get("fallback_fired"), Some(&Value::Bool(true)));
    result.routing = crate::routing::Routing::unseen("claude");
    assert!(result.to_json().as_object().is_some_and(|object| object.get("routing").is_none()));
}

fn a_result() -> RunResult {
    RunResult {
        ok: true,
        run_id: "r".into(),
        workflow: "w".into(),
        client: "pi-free".into(),
        model: "free/m".into(),
        family: "x".into(),
        mode: RunMode::ReadOnly,
        tokens: RunTokens { cached: 0.0, input: 1.0, output: 2.0 },
        cost_usd: 0.0,
        output_path: "/o".into(),
        summary: "s".into(),
        reason: None,
        unavailable_until: None,
        write: None,
        attempt: None,
        attempt_of: None,
        exit_code: Some(0),
        signal: None,
        stderr_tail: Some(String::new()),
        failure: None,
        cleanup_warning: None,
        warnings: Vec::new(),
        fallback: None,
        retry: None,
        rewrite: None,
        routing: crate::routing::Routing::unseen("pi-free"),
    }
}
