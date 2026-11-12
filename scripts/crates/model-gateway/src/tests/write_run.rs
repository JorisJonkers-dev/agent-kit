//! Tests for a writing run: the worktree it works in, the checkouts of the repository it watches, what it
//! stages and the check it runs, through fakes for git, for the client and for the check, so every refusal and
//! every failure of the gateway's own is pinned.

use super::machine::{grant, now_ms, Machine, DEFAULTS};
use crate::adapters::fs::SystemFiles;
use crate::error::GatewayError;
use crate::escalation_run::AttemptedRun;
use crate::json::{parse, Value};
use crate::ports::{Clock, FileStamp, Files, FsError, Git, GitRemote, Process, ProcessCommand, ProcessResult};
use crate::run::{RunMode, RunTokens};
use crate::run_command::{RunCommand, RunResult};
use crate::write_run::{CheckReport, WriteReport};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// A clock the test moves: the client's launch can put it past the moment a grant lapses.
#[derive(Clone)]
struct Movable(Rc<Cell<i64>>);

impl Clock for Movable {
    fn now_ms(&self) -> i64 {
        self.0.get()
    }
    fn monotonic_ms(&self) -> f64 {
        0.0
    }
}

/// What the fake git answers, and what it was asked.
#[derive(Default)]
struct GitState {
    common: Option<Result<String, String>>,
    git_dir: Option<Result<String, String>>,
    worktrees: Option<Result<Vec<String>, String>>,
    /// The status of each watched worktree, in the order it is read; the last answer repeats.
    statuses: HashMap<String, Vec<Result<String, String>>>,
    create: Option<Result<(), String>>,
    /// The brief the worktree is made with, as the repository keeps one.
    brief: Option<String>,
    /// Whether the brief is a directory instead of a file, which cannot be read.
    brief_is_dir: bool,
    stage: Option<Result<(), String>>,
    staged: Option<Result<Vec<String>, String>>,
    remove: Option<Result<(), String>>,
    branch: Option<Result<(), String>>,
    calls: Vec<String>,
}

/// A git whose answers the test chooses; the repository it finds belongs to nedap, which permits Codex and
/// Claude Code. A worktree it creates is a real directory, as git's own would be.
#[derive(Clone, Default)]
struct FakeGit {
    state: Rc<RefCell<GitState>>,
}

impl FakeGit {
    fn new(common: &str) -> FakeGit {
        let git = FakeGit::default();
        git.set(|state| {
            state.common = Some(Ok(common.to_string()));
            state.worktrees = Some(Ok(Vec::new()));
        });
        git
    }

    fn set(&self, change: impl FnOnce(&mut GitState)) {
        change(&mut self.state.borrow_mut());
    }

    fn calls(&self) -> Vec<String> {
        self.state.borrow().calls.clone()
    }
}

impl Git for FakeGit {
    fn root(&self, path: &str) -> Result<Option<String>, String> {
        // The machine's configuration is in no repository: a grant may live there.
        if path.contains("/.config/") {
            return Ok(None);
        }
        Ok(Some(path.to_string()))
    }

    fn remotes(&self, _path: &str) -> Result<Vec<GitRemote>, String> {
        Ok(vec![GitRemote { name: "origin".to_string(), url: "git@github.com:nedap/x.git".to_string() }])
    }

    fn remove_worktree(&self, repo: &str, path: &str) -> Result<(), String> {
        let mut state = self.state.borrow_mut();
        state.calls.push(format!("remove {repo} {path}"));
        state.remove.clone().unwrap_or(Ok(()))
    }

    fn delete_branch(&self, repo: &str, branch: &str) -> Result<(), String> {
        let mut state = self.state.borrow_mut();
        state.calls.push(format!("branch {repo} {branch}"));
        state.branch.clone().unwrap_or(Ok(()))
    }

    fn common_dir(&self, _path: &str) -> Result<String, String> {
        self.state.borrow().common.clone().expect("a common directory")
    }

    fn add_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        unreachable!("a writing run adds no throwaway worktree")
    }

    fn git_dir(&self, path: &str) -> Result<String, String> {
        let mut state = self.state.borrow_mut();
        state.calls.push(format!("git-dir {path}"));
        state.git_dir.clone().unwrap_or_else(|| Err("no git directory".to_string()))
    }

    fn create_worktree(&self, repo: &str, path: &str, branch: &str, base: &str) -> Result<(), String> {
        let mut state = self.state.borrow_mut();
        state.calls.push(format!("create {repo} {path} {branch} {base}"));
        std::fs::create_dir_all(path).expect("the worktree directory");
        if state.brief_is_dir {
            std::fs::create_dir_all(format!("{path}/codex-brief.md")).expect("a brief that is a directory");
        } else if let Some(brief) = &state.brief {
            std::fs::write(format!("{path}/codex-brief.md"), brief).expect("the brief");
        }
        state.create.clone().unwrap_or(Ok(()))
    }

    fn stage_all(&self, path: &str) -> Result<(), String> {
        let mut state = self.state.borrow_mut();
        state.calls.push(format!("stage {path}"));
        state.stage.clone().unwrap_or(Ok(()))
    }

    fn staged_files(&self, path: &str, base: Option<&str>) -> Result<Vec<String>, String> {
        let mut state = self.state.borrow_mut();
        state.calls.push(format!("staged {path} {}", base.unwrap_or("-")));
        state.staged.clone().unwrap_or(Ok(Vec::new()))
    }

    fn list_worktrees(&self, repo: &str) -> Result<Vec<String>, String> {
        let mut state = self.state.borrow_mut();
        state.calls.push(format!("list {repo}"));
        state.worktrees.clone().expect("a worktree listing")
    }

    fn status(&self, path: &str) -> Result<String, String> {
        let mut state = self.state.borrow_mut();
        state.calls.push(format!("status {path}"));
        let queue = state.statuses.get_mut(path).unwrap_or_else(|| panic!("the test reads no status of {path}"));
        if queue.len() > 1 {
            queue.remove(0)
        } else {
            queue[0].clone()
        }
    }
}

/// A process port that answers by program name, records every launch, and can move the clock on once the
/// client has run, so the check finds its grant lapsed.
#[derive(Clone, Default)]
struct FakeProcess {
    answers: Rc<RefCell<HashMap<String, Result<ProcessResult, String>>>>,
    commands: Rc<RefCell<Vec<ProcessCommand>>>,
    clock: Rc<Cell<i64>>,
    jump: Rc<Cell<Option<i64>>>,
}

impl FakeProcess {
    fn answer(&self, command: &str, result: Result<ProcessResult, String>) {
        self.answers.borrow_mut().insert(command.to_string(), result);
    }

    fn commands(&self) -> Vec<ProcessCommand> {
        self.commands.borrow().clone()
    }

    /// The one check the gateway ran.
    fn check(&self) -> ProcessCommand {
        self.commands().into_iter().find(|command| command.command == "sh").expect("a check")
    }

    /// The one launch of the client.
    fn launch(&self) -> ProcessCommand {
        self.commands().into_iter().find(|command| command.command != "sh").expect("a launch")
    }
}

impl Process for FakeProcess {
    fn resolve(&self, binary: &str) -> Option<String> {
        Some(format!("/bin/{binary}"))
    }

    fn exec(&self, command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.answers.borrow().get(command).cloned().unwrap_or_else(|| Err("no answer was stubbed".to_string()))
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.commands.borrow_mut().push(command.clone());
        if command.command != "sh" {
            if let Some(at) = self.jump.get() {
                self.clock.set(at);
                self.jump.set(None);
            }
        }
        self.exec(&command.command, &[], command.timeout_ms)
    }
}

/// The real files, with one path's write refused: the only way to make the gateway's own write fail.
struct RefusingFiles {
    inner: SystemFiles,
    refuse: String,
}

impl Files for RefusingFiles {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        self.inner.read_text(path)
    }
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.inner.write_atomic(path, content)
    }
    fn mkdir_all(&self, path: &str) -> Result<(), FsError> {
        self.inner.mkdir_all(path)
    }
    fn create_exclusive(&self, path: &str) -> Result<bool, FsError> {
        self.inner.create_exclusive(path)
    }
    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        self.inner.remove_file(path)
    }
    fn modified_ms(&self, path: &str) -> Option<i64> {
        self.inner.modified_ms(path)
    }
    fn exists(&self, path: &str) -> Result<bool, FsError> {
        self.inner.exists(path)
    }
    fn append(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.inner.append(path, content)
    }
    fn stamp(&self, path: &str) -> Option<FileStamp> {
        self.inner.stamp(path)
    }
    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.inner.write_private(path, content)
    }
    fn write_text(&self, path: &str, content: &str) -> Result<(), FsError> {
        if path.ends_with(&self.refuse) {
            return Err(FsError { code: "EACCES".to_string(), message: format!("EACCES: permission denied, open '{path}'") });
        }
        self.inner.write_text(path, content)
    }
    fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, FsError> {
        self.inner.temp_dir(parent, prefix)
    }
    fn remove(&self, path: &str) -> Result<(), FsError> {
        self.inner.remove(path)
    }
}

/// A machine whose git, client and check the test chooses, and whose clock it can move.
struct WriteMachine {
    machine: Machine,
    git: FakeGit,
    process: FakeProcess,
}

fn machine() -> WriteMachine {
    let mut machine = Machine::new(DEFAULTS);
    let root = machine.sandbox.text();
    let clock = Rc::new(Cell::new(now_ms()));
    let process = FakeProcess { clock: Rc::clone(&clock), ..FakeProcess::default() };
    let git = FakeGit::new(&format!("{root}/common"));
    machine.runtime.clock = Box::new(Movable(Rc::clone(&clock)));
    machine.runtime.process = Box::new(process.clone());
    machine.runtime.git = Box::new(git.clone());
    WriteMachine { machine, git, process }
}

impl WriteMachine {
    fn common(&self) -> String {
        format!("{}/common", self.machine.sandbox.text())
    }

    fn state(&self) -> String {
        format!("{}/state/agent-kit/model-gateway", self.machine.sandbox.text())
    }

    fn run(&self, input: &str) -> Result<Value, GatewayError> {
        RunCommand { runtime: &self.machine.runtime }.run(&parse(input).expect("JSON"))
    }

    /// The prompt the delegate was given.
    fn prompt(&self, result: &Value) -> String {
        let path = format!("{}/model-gateway/runs/{}/prompt.md", self.common(), text(result, "run_id"));
        std::fs::read_to_string(path).expect("the prompt")
    }

    /// The kept worktrees the run entered.
    fn kept(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(format!("{}/kept-worktrees.json", self.state())).expect("the kept list");
        match parse(&text).expect("JSON") {
            Value::Obj(object) => match object.get("worktrees") {
                Some(Value::Arr(entries)) => entries.clone(),
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }
}

const CLAUDE_ANSWER: &str = r#"{"type":"result","result":"done\nSUMMARY: fine","usage":{"input_tokens":10,"output_tokens":5}}"#;

const CODEX_ANSWER: &str = "{\"item\":{\"id\":\"item_0\",\"text\":\"done\\nSUMMARY: fine\",\"type\":\"agent_message\"},\"type\":\"item.completed\"}\n{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}";

fn answer(stdout: &str) -> Result<ProcessResult, String> {
    Ok(ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code: 0, timed_out: false })
}

fn exited(exit_code: i32, stdout: &str, stderr: &str) -> Result<ProcessResult, String> {
    Ok(ProcessResult { stdout: stdout.to_string(), stderr: stderr.to_string(), exit_code, timed_out: false })
}

fn claude_write(extra: &str) -> String {
    format!(
        r#"{{"workflow":"w","client":"claude","mode":"write","model":"claude-sonnet-5-5","prompt":"p","repo":"/repo","base":"main","check":"make test","escalate":false{extra}}}"#
    )
}

fn codex_write(extra: &str) -> String {
    format!(
        r#"{{"workflow":"w","client":"codex","mode":"write","model":"gpt-5.5","prompt":"p","repo":"/repo","base":"main","check":"make test","escalate":false{extra}}}"#
    )
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.as_object().and_then(|object| object.get(key)).and_then(Value::as_str).unwrap_or_default()
}

fn member<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object().and_then(|object| object.get(key))
}

fn staged(result: &Value) -> Vec<String> {
    match member(result, "staged_files") {
        Some(Value::Arr(files)) => files.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        other => panic!("{other:?}"),
    }
}

fn check(result: &Value) -> &Value {
    member(result, "check").expect("a check")
}

#[test]
fn a_writing_run_works_in_a_worktree_of_its_own_and_stages_what_its_delegate_left() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.process.answer("sh", exited(0, "all good\n", "warn\n"));
    write.git.set(|state| state.staged = Some(Ok(vec!["a.txt".to_string(), "b/c.txt".to_string()])));

    let result = write.run(&claude_write("")).expect("a result");

    let id = text(&result, "run_id").to_string();
    let worktree = format!("{}/model-gateway/worktrees/{id}", write.common());
    assert_eq!((text(&result, "status"), text(&result, "mode")), ("ok", "write"));
    assert_eq!(text(&result, "branch"), format!("gateway/{id}"));
    assert_eq!(text(&result, "worktree"), worktree);
    assert_eq!(staged(&result), vec!["a.txt".to_string(), "b/c.txt".to_string()]);
    assert_eq!(member(&result, "worktree_error"), None);
    assert_eq!(member(&result, "foreign_worktree"), None);
    // A writing run is never released, so it carries no cleanup warning.
    assert_eq!(member(&result, "cleanup_warning"), None);
    assert!(std::path::Path::new(&worktree).is_dir());

    let report = check(&result);
    let output_path = format!("{}/model-gateway/runs/{id}/check-output.txt", write.common());
    assert_eq!(text(report, "command"), "make test");
    assert_eq!(member(report, "exit_code"), Some(&Value::Num(0.0)));
    assert_eq!(text(report, "output_path"), output_path);
    assert_eq!(std::fs::read_to_string(&output_path).expect("the check's output"), "all good\nwarn\n");

    // The check runs in the worktree, with the mark that stops a delegate delegating on, held to the run's limit.
    let check = write.process.check();
    assert_eq!(check.args, vec!["-c".to_string(), "make test".to_string()]);
    assert_eq!(check.cwd.as_deref(), Some(worktree.as_str()));
    assert_eq!(check.env.get("AGENT_KIT_DELEGATION").map(String::as_str), Some("1"));
    assert_eq!(check.timeout_ms, Some(900_000));

    // The worktree is created off the caller's base, on a branch of its own, and staged against that base.
    let calls = write.git.calls();
    assert!(calls.contains(&format!("create /repo {worktree} gateway/{id} main")), "{calls:?}");
    assert!(calls.contains(&format!("stage {worktree}")), "{calls:?}");
    assert!(calls.contains(&format!("staged {worktree} main")), "{calls:?}");

    // It is entered in the kept list at once, in the node runner's member order.
    let kept = write.kept();
    assert_eq!(kept.len(), 1);
    let entry = kept[0].as_object().expect("an entry");
    let keys: Vec<&str> = entry.entries().into_iter().map(|(key, _)| key).collect();
    assert_eq!(keys.join(","), "at,branch,path,repo,run_id,workflow");
    assert_eq!(entry.get("at").and_then(Value::as_str), Some("2026-11-04T10:00:00.000Z"));
    assert_eq!(entry.get("branch").and_then(Value::as_str), Some(format!("gateway/{id}").as_str()));
    assert_eq!(entry.get("path").and_then(Value::as_str), Some(worktree.as_str()));
    assert_eq!(entry.get("repo").and_then(Value::as_str), Some("/repo"));
    assert_eq!(entry.get("run_id").and_then(Value::as_str), Some(id.as_str()));
    assert_eq!(entry.get("workflow").and_then(Value::as_str), Some("w"));

    // A writing delegate is told where it may edit, and that committing is not its to do.
    let prompt = write.prompt(&result);
    assert!(prompt.contains("Edit only files inside the working directory. Do not commit."), "{prompt}");
    assert!(!prompt.contains("commit it on the branch you are on"), "{prompt}");

    // The ledger line is the one every run leaves.
    assert!(write.machine.ledger_text().contains(r#""outcome":"ok""#), "{}", write.machine.ledger_text());
    assert!(write.machine.ledger_text().contains(r#""mode":"write""#), "{}", write.machine.ledger_text());
}

#[test]
fn a_codex_writing_run_gets_the_git_directories_it_may_write_and_the_brief_cut_at_the_limit() {
    let write = machine();
    write.machine.write_grants(&[grant(5.0, 600.0)]);
    write.process.answer("codex", answer(CODEX_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    let common = write.common();
    write.git.set(|state| {
        state.git_dir = Some(Ok(format!("{common}/repo/.git/worktrees/one")));
        state.brief = Some("x".repeat(20_001));
    });

    let result = write.run(&codex_write("")).expect("a result");

    assert_eq!(text(&result, "status"), "ok");
    let launch = write.process.launch();
    let dirs = vec![
        format!("{common}/repo/.git/worktrees/one"),
        format!("{common}/objects"),
        format!("{common}/refs"),
        format!("{common}/logs"),
    ];
    let added: Vec<String> = launch.args.windows(2).filter(|pair| pair[0] == "--add-dir").map(|pair| pair[1].clone()).collect();
    assert_eq!(added, dirs);
    // The brief comes first, cut at the limit, and the rule says the delegate commits its own change.
    let prompt = write.prompt(&result);
    assert!(prompt.starts_with(&"x".repeat(20_000)), "{}", &prompt[..40]);
    assert!(!prompt.starts_with(&"x".repeat(20_001)));
    assert!(prompt.contains("commit it on the branch you are on"), "{prompt}");
    // The grant holds the check to the time it leaves.
    assert_eq!(write.process.check().timeout_ms, Some(600_000));
}

#[test]
fn a_codex_run_whose_git_directories_cannot_be_read_gets_neither_the_directories_nor_the_brief() {
    let write = machine();
    write.machine.write_grants(&[grant(5.0, 600.0)]);
    write.process.answer("codex", answer(CODEX_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    write.git.set(|state| {
        state.git_dir = Some(Err("no git directory".to_string()));
        state.brief = Some("the brief".to_string());
    });

    let result = write.run(&codex_write("")).expect("a result");

    assert_eq!(text(&result, "status"), "ok");
    assert!(!write.process.launch().args.contains(&"--add-dir".to_string()));
    let prompt = write.prompt(&result);
    assert!(!prompt.contains("the brief"), "{prompt}");
    assert!(prompt.contains("Do not commit."), "{prompt}");
}

#[test]
fn a_codex_run_whose_git_directory_is_the_common_one_gets_neither() {
    let write = machine();
    write.machine.write_grants(&[grant(5.0, 600.0)]);
    write.process.answer("codex", answer(CODEX_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    let common = write.common();
    write.git.set(|state| state.git_dir = Some(Ok(common.clone())));

    let result = write.run(&codex_write("")).expect("a result");

    assert_eq!(text(&result, "status"), "ok");
    assert!(!write.process.launch().args.contains(&"--add-dir".to_string()));
    assert!(write.prompt(&result).contains("Do not commit."));
}

#[test]
fn a_codex_run_with_no_brief_gets_the_rule_but_no_brief() {
    let write = machine();
    write.machine.write_grants(&[grant(5.0, 600.0)]);
    write.process.answer("codex", answer(CODEX_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    let common = write.common();
    write.git.set(|state| state.git_dir = Some(Ok(format!("{common}/repo/.git/worktrees/one"))));

    let result = write.run(&codex_write("")).expect("a result");

    let prompt = write.prompt(&result);
    assert!(!prompt.contains("---"), "{prompt}");
    assert!(prompt.contains("commit it on the branch you are on"), "{prompt}");
}

#[test]
fn a_brief_that_cannot_be_read_is_no_brief() {
    let write = machine();
    write.machine.write_grants(&[grant(5.0, 600.0)]);
    write.process.answer("codex", answer(CODEX_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    let common = write.common();
    write.git.set(|state| {
        state.git_dir = Some(Ok(format!("{common}/repo/.git/worktrees/one")));
        state.brief_is_dir = true;
    });

    let result = write.run(&codex_write("")).expect("a result");

    let prompt = write.prompt(&result);
    assert!(!prompt.contains("---"), "{prompt}");
    assert!(prompt.contains("commit it on the branch you are on"), "{prompt}");
}

#[test]
fn a_worktree_that_cannot_be_created_is_a_worktree_failure_and_leaves_no_directory() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.git.set(|state| state.create = Some(Err("no room".to_string())));

    let error = write.run(&claude_write("")).expect_err("worktree failure");

    assert_eq!((error.code.as_str(), error.message.as_str()), ("worktree-failed", "/repo: cannot create a worktree off main: no room"));
    let worktrees = format!("{}/model-gateway/worktrees", write.common());
    let left: Vec<_> = std::fs::read_dir(&worktrees).expect("the worktrees directory").flatten().collect();
    assert!(left.is_empty(), "{left:?}");
    // Nothing was launched, staged or checked.
    assert!(write.process.commands().is_empty());
    assert!(write.machine.ledger().is_empty());
}

#[test]
fn a_kept_list_that_cannot_take_the_worktree_removes_it_and_its_branch_and_propagates() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.machine.write_at(&format!("{}/kept-worktrees.json", write.state()), "not json");

    let error = write.run(&claude_write("")).expect_err("the kept list refuses it");

    assert_eq!(error.code, "invalid-config");
    assert!(error.message.contains("not JSON"), "{}", error.message);
    let calls = write.git.calls();
    let removed = calls.iter().find(|call| call.starts_with("remove /repo ")).expect("the worktree is removed");
    let branch = calls.iter().find(|call| call.starts_with("branch /repo gateway/")).expect("the branch is removed");
    assert!(removed.contains("/model-gateway/worktrees/"), "{removed}");
    assert!(branch.contains("/model-gateway/worktrees/") || branch.starts_with("branch /repo gateway/"), "{branch}");
    assert!(write.process.commands().is_empty());
}

#[test]
fn a_kept_list_that_cannot_take_the_worktree_keeps_the_lists_own_error_when_git_cannot_undo_it() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.machine.write_at(&format!("{}/kept-worktrees.json", write.state()), "not json");
    write.git.set(|state| {
        state.remove = Some(Err("busy".to_string()));
        state.branch = Some(Err("checked out".to_string()));
    });

    let error = write.run(&claude_write("")).expect_err("the kept list refuses it");

    assert_eq!(error.code, "invalid-config");
}

#[test]
fn a_repository_whose_worktrees_cannot_be_listed_is_refused_before_anything_is_taken() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.git.set(|state| state.worktrees = Some(Err("bad listing".to_string())));

    let error = write.run(&claude_write("")).expect_err("the listing refuses the run");

    assert_eq!((error.code.as_str(), error.message.as_str()), ("worktree-failed", "/repo: cannot list the worktrees to watch: bad listing"));
    assert!(write.machine.ledger().is_empty());
    assert!(write.process.commands().is_empty());
}

#[test]
fn a_worktree_whose_status_cannot_be_read_is_refused() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.git.set(|state| {
        state.worktrees = Some(Ok(vec!["/other".to_string()]));
        state.statuses.insert("/other".to_string(), vec![Err("gone".to_string())]);
    });

    let error = write.run(&claude_write("")).expect_err("the status refuses the run");

    assert_eq!((error.code.as_str(), error.message.as_str()), ("worktree-failed", "/repo: cannot read the status of /other: gone"));
    assert!(write.machine.ledger().is_empty());
}

#[test]
fn the_gateways_own_worktrees_are_not_watched() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    let kept = format!("{}/model-gateway/worktrees", write.common());
    write.git.set(|state| {
        state.worktrees = Some(Ok(vec![kept.clone(), format!("{kept}/one"), "/other".to_string()]));
        state.statuses.insert("/other".to_string(), vec![Ok(String::new())]);
    });

    let result = write.run(&claude_write("")).expect("a result");

    assert_eq!(text(&result, "status"), "ok");
    let statuses: Vec<String> = write.git.calls().into_iter().filter(|call| call.starts_with("status ")).collect();
    // Read before the delegate runs and again when it ends, for the one checkout watched.
    assert_eq!(statuses, vec!["status /other".to_string(), "status /other".to_string()]);
}

#[test]
fn a_change_in_another_checkout_fails_the_run_and_names_it() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    write.git.set(|state| {
        state.worktrees = Some(Ok(vec!["/other".to_string()]));
        state.statuses.insert("/other".to_string(), vec![Ok(String::new()), Ok(" M file\n".to_string())]);
        state.staged = Some(Ok(vec!["a.txt".to_string()]));
    });

    let result = write.run(&claude_write("")).expect("a result");

    assert_eq!((text(&result, "status"), text(&result, "reason")), ("failed", "foreign-worktree-change"));
    assert_eq!(text(&result, "foreign_worktree"), "/other");
    assert_eq!(staged(&result), vec!["a.txt".to_string()]);
    // The delegate's own work is still staged, and no check runs beside a foreign change.
    assert_eq!(member(&result, "check"), None);
    assert!(write.git.calls().iter().any(|call| call == &format!("stage {}", text(&result, "worktree"))));
}

#[test]
fn a_foreign_change_is_reported_before_the_delegates_own_failure_and_leaves_nothing_staged() {
    let write = machine();
    write.process.answer("claude", exited(1, "", "boom"));
    write.git.set(|state| {
        state.worktrees = Some(Ok(vec!["/other".to_string()]));
        state.statuses.insert("/other".to_string(), vec![Ok(String::new()), Ok(" M file\n".to_string())]);
    });

    let result = write.run(&claude_write("")).expect("a result");

    assert_eq!((text(&result, "status"), text(&result, "reason")), ("failed", "foreign-worktree-change"));
    assert_eq!(staged(&result), Vec::<String>::new());
    assert!(!write.git.calls().iter().any(|call| call.starts_with("stage ")));
}

#[test]
fn a_checkout_that_cannot_be_read_at_the_end_counts_as_changed() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    write.git.set(|state| {
        state.worktrees = Some(Ok(vec!["/other".to_string()]));
        state.statuses.insert("/other".to_string(), vec![Ok(String::new()), Err("gone".to_string())]);
    });

    let result = write.run(&claude_write("")).expect("a result");

    assert_eq!(text(&result, "reason"), "foreign-worktree-change");
    assert_eq!(text(&result, "foreign_worktree"), "/other");
}

#[test]
fn a_worktree_that_cannot_be_staged_fails_the_run_with_stage_failed() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    write.git.set(|state| state.stage = Some(Err("no room".to_string())));

    let result = write.run(&claude_write("")).expect("a result");

    assert_eq!((text(&result, "status"), text(&result, "reason")), ("failed", "stage-failed"));
    assert_eq!(staged(&result), Vec::<String>::new());
    assert_eq!(text(&result, "worktree_error"), "no room");
    // The check still runs, and its report goes out beside the failure.
    assert_eq!(member(check(&result), "exit_code"), Some(&Value::Num(0.0)));
}

#[test]
fn a_worktree_that_cannot_be_listed_is_staged_but_unlisted() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    write.git.set(|state| state.staged = Some(Err("bad listing".to_string())));

    let result = write.run(&claude_write("")).expect("a result");

    assert_eq!((text(&result, "status"), text(&result, "reason")), ("failed", "stage-list-failed"));
    assert_eq!(member(&result, "staged_files"), Some(&Value::Null));
    assert_eq!(text(&result, "worktree_error"), "bad listing");
}

#[test]
fn a_check_that_fails_fails_the_run_with_check_failed() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.process.answer("sh", exited(2, "one failure\n", ""));
    write.git.set(|state| state.staged = Some(Ok(vec!["a.txt".to_string()])));

    let result = write.run(&claude_write("")).expect("a result");

    assert_eq!((text(&result, "status"), text(&result, "reason")), ("failed", "check-failed"));
    assert_eq!(member(check(&result), "exit_code"), Some(&Value::Num(2.0)));
    assert_eq!(staged(&result), vec!["a.txt".to_string()]);
    assert_eq!(member(&result, "worktree_error"), None);
    let output = format!("{}/model-gateway/runs/{}/check-output.txt", write.common(), text(&result, "run_id"));
    assert_eq!(std::fs::read_to_string(output).expect("the check's output"), "one failure\n");
}

#[test]
fn a_check_that_cannot_start_reports_127_and_the_platforms_words() {
    let write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.process.answer("sh", Err("spawn sh ENOENT".to_string()));

    let result = write.run(&claude_write("")).expect("a result");

    assert_eq!((text(&result, "status"), text(&result, "reason")), ("failed", "check-failed"));
    assert_eq!(member(check(&result), "exit_code"), Some(&Value::Num(127.0)));
    let output = format!("{}/model-gateway/runs/{}/check-output.txt", write.common(), text(&result, "run_id"));
    assert_eq!(std::fs::read_to_string(output).expect("the check's output"), "spawn sh ENOENT");
}

#[test]
fn a_check_whose_output_cannot_be_kept_still_reports_its_exit_code() {
    let mut write = machine();
    write.process.answer("claude", answer(CLAUDE_ANSWER));
    write.process.answer("sh", exited(0, "fine\n", ""));
    let root = write.machine.sandbox.text();
    write.machine.runtime.files = Box::new(RefusingFiles { inner: SystemFiles { cwd: root }, refuse: "check-output.txt".to_string() });

    let result = write.run(&claude_write("")).expect("a result");

    assert_eq!((text(&result, "status"), text(&result, "reason")), ("failed", "check-output-failed"));
    assert_eq!(member(check(&result), "exit_code"), Some(&Value::Num(0.0)));
    let output = format!("{}/model-gateway/runs/{}/check-output.txt", write.common(), text(&result, "run_id"));
    assert_eq!(text(&result, "worktree_error"), format!("EACCES: permission denied, open '{output}'"));
}

#[test]
fn a_check_that_would_outlive_the_grant_is_not_started() {
    let write = machine();
    write.machine.write_grants(&[grant(5.0, 600.0)]);
    write.process.answer("codex", answer(CODEX_ANSWER));
    write.git.set(|state| state.staged = Some(Ok(vec!["a.txt".to_string()])));
    // The delegate spends the whole grant.
    write.process.jump.set(Some(now_ms() + 600_000));

    let result = write.run(&codex_write("")).expect("a result");

    assert_eq!((text(&result, "status"), text(&result, "reason")), ("failed", "grant-expired"));
    assert_eq!(member(&result, "check"), None);
    assert_eq!(staged(&result), vec!["a.txt".to_string()]);
    assert!(!write.process.commands().iter().any(|command| command.command == "sh"));
}

#[test]
fn the_check_is_held_to_the_time_left_on_the_grant() {
    let write = machine();
    write.machine.write_grants(&[grant(5.0, 600.0)]);
    write.process.answer("codex", answer(CODEX_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    // The delegate spends half the grant, so the check gets what is left of it.
    write.process.jump.set(Some(now_ms() + 300_000));

    let result = write.run(&codex_write("")).expect("a result");

    assert_eq!(text(&result, "status"), "ok");
    assert_eq!(write.process.check().timeout_ms, Some(300_000));
}

#[test]
fn a_check_gets_at_least_a_second_of_what_the_grant_leaves() {
    let write = machine();
    write.machine.write_grants(&[grant(5.0, 600.0)]);
    write.process.answer("codex", answer(CODEX_ANSWER));
    write.process.answer("sh", exited(0, "", ""));
    // Half a second is left: the check still gets a second to start.
    write.process.jump.set(Some(now_ms() + 599_500));

    let result = write.run(&codex_write("")).expect("a result");

    assert_eq!(text(&result, "status"), "ok");
    assert_eq!(write.process.check().timeout_ms, Some(1_000));
}

#[test]
fn a_delegate_that_did_not_finish_leaves_its_worktree_unstaged_and_runs_no_check() {
    let write = machine();
    write.process.answer("claude", exited(1, "", "boom"));

    let result = write.run(&claude_write("")).expect("a result");

    assert_eq!((text(&result, "status"), text(&result, "reason")), ("failed", "exit-1"));
    assert_eq!(staged(&result), Vec::<String>::new());
    assert_eq!(member(&result, "check"), None);
    assert!(!write.git.calls().iter().any(|call| call.starts_with("stage ")));
    assert!(!write.process.commands().iter().any(|command| command.command == "sh"));
}

#[test]
fn a_writing_run_is_refused_for_a_model_the_catalogue_does_not_know_before_anything_is_taken() {
    let write = machine();
    let unknown = claude_write("").replace("claude-sonnet-5-5", "no-such-model");

    let error = write.run(&unknown).expect_err("refused");

    assert_eq!(error.code, "unknown-model");
    assert!(write.machine.ledger().is_empty());
    assert!(write.git.calls().is_empty());
}

#[test]
fn a_writing_run_result_prints_its_members_in_the_order_the_node_runner_writes_them() {
    let mut result = RunResult {
        ok: false,
        run_id: "r".into(),
        workflow: "w".into(),
        client: "claude".into(),
        model: "m".into(),
        family: "anthropic".into(),
        mode: RunMode::Write,
        tokens: RunTokens { cached: 0.0, input: 1.0, output: 2.0 },
        cost_usd: 0.5,
        output_path: "/o".into(),
        summary: "s".into(),
        reason: Some("check-failed".into()),
        unavailable_until: Some("2026-11-04T10:00:00.000Z".into()),
        write: Some(WriteReport {
            branch: "gateway/r".into(),
            worktree: "/w".into(),
            foreign_worktree: Some("/other".into()),
            check: Some(CheckReport { command: "make test".into(), exit_code: 1, output_path: "/run/check-output.txt".into() }),
            staged_files: None,
            worktree_error: Some("bad".into()),
        }),
        attempt: Some(2),
        attempt_of: Some("r0".into()),
        cleanup_warning: Some("w".into()),
        warnings: vec!["l".into()],
    };
    assert_eq!(result.check_output_path(), Some("/run/check-output.txt"));
    let keys: Vec<String> = match result.to_json() {
        Value::Obj(object) => object.entries().into_iter().map(|(key, _)| key.to_string()).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        keys.join(","),
        "client,cost_usd,family,mode,model,output_path,run_id,status,summary,tokens,workflow,reason,unavailable_until,branch,worktree,foreign_worktree,check,staged_files,worktree_error,attempt,attempt_of,cleanup_warning,warnings"
    );
    // A report with no foreign change, no check and no error prints only what it has.
    result.write = Some(WriteReport {
        branch: "gateway/r".into(),
        worktree: "/w".into(),
        foreign_worktree: None,
        check: None,
        staged_files: Some(vec!["a.txt".into()]),
        worktree_error: None,
    });
    assert_eq!(result.check_output_path(), None);
    let keys: Vec<String> = match result.to_json() {
        Value::Obj(object) => object.entries().into_iter().map(|(key, _)| key.to_string()).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        keys.join(","),
        "client,cost_usd,family,mode,model,output_path,run_id,status,summary,tokens,workflow,reason,unavailable_until,branch,worktree,staged_files,attempt,attempt_of,cleanup_warning,warnings"
    );
}
