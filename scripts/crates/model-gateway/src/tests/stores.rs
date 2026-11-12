//! Tests for the two stores the gateway keeps on disk: the machine-wide ledger
//! and the machine's grants file, each read and written under a lock. The
//! failure paths run against in-memory stubs of the files, git and the clock;
//! the real file system, a real lock file and a real repository run in a
//! sandbox directory.

use super::scenarios::{FixedClock, Sandbox, FIXED_NOW};
use crate::adapters::clock::SystemClock;
use crate::adapters::fs::SystemFiles;
use crate::adapters::git::{SystemGit, TIMEOUT_MS};
use crate::adapters::process::SystemProcess;
use crate::config::CostClass;
use crate::error::GatewayError;
use crate::grants::{grant_file_text, Grant};
use crate::grants_file::{GrantChange, GrantsStore};
use crate::json::compact;
use crate::ledger::{LedgerEntry, LedgerRecord, ReservationRecord, RunTokens, SettledRecord};
use crate::ledger_file::{parse_ledger_line, settle_stale, LedgerChange, LedgerStore};
use crate::ports::{Clock, FileStamp, Files, FsError, Git, GitRemote, LockTimings, Runtime};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

/// The ledger and the grants file a runtime with `HOME=/home/me` names.
const LEDGER: &str = "/home/me/.local/state/agent-kit/model-gateway/ledger.jsonl";
const LEDGER_LOCK: &str = "/home/me/.local/state/agent-kit/model-gateway/ledger.jsonl.lock";
const STATE_DIR: &str = "/home/me/.local/state/agent-kit/model-gateway";
const GRANTS: &str = "/home/me/.config/agent-kit/model-gateway-grants.json";
const GRANTS_LOCK: &str = "/home/me/.config/agent-kit/model-gateway-grants.json.lock";
const CONFIG_DIR: &str = "/home/me/.config/agent-kit";

/// The files, the lock and the failures a real file system can give, in memory.
#[derive(Default)]
struct MemoryState {
    files: RefCell<BTreeMap<String, String>>,
    dirs: RefCell<Vec<String>>,
    read_errors: RefCell<BTreeMap<String, FsError>>,
    exists_errors: RefCell<BTreeMap<String, FsError>>,
    mkdir_error: RefCell<Option<FsError>>,
    append_error: RefCell<Option<FsError>>,
    write_error: RefCell<Option<FsError>>,
    lock_present: Cell<bool>,
    lock_modified: Cell<Option<i64>>,
    appended: RefCell<Vec<(String, String)>>,
    made: RefCell<Vec<String>>,
    written: RefCell<Vec<(String, String)>>,
    released: Cell<usize>,
}

/// A handle on the in-memory files; every clone shares the same state.
#[derive(Clone, Default)]
struct MemoryFiles {
    state: Rc<MemoryState>,
}

impl MemoryFiles {
    fn with_file(path: &str, content: &str) -> MemoryFiles {
        let files = MemoryFiles::default();
        files.state.files.borrow_mut().insert(path.to_string(), content.to_string());
        files
    }

    fn with_dir(path: &str) -> MemoryFiles {
        let files = MemoryFiles::default();
        files.state.dirs.borrow_mut().push(path.to_string());
        files
    }

    fn text(&self, path: &str) -> Option<String> {
        self.state.files.borrow().get(path).cloned()
    }

    fn appended(&self) -> Vec<(String, String)> {
        self.state.appended.borrow().clone()
    }

    fn written(&self) -> Vec<(String, String)> {
        self.state.written.borrow().clone()
    }

    fn made(&self) -> Vec<String> {
        self.state.made.borrow().clone()
    }

    fn released(&self) -> usize {
        self.state.released.get()
    }

    fn fail_read(&self, path: &str, code: &str, message: &str) {
        self.state.read_errors.borrow_mut().insert(path.to_string(), FsError { code: code.to_string(), message: message.to_string() });
    }

    fn fail_exists(&self, path: &str, code: &str, message: &str) {
        self.state.exists_errors.borrow_mut().insert(path.to_string(), FsError { code: code.to_string(), message: message.to_string() });
    }
}

impl Files for MemoryFiles {
    fn write_text(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        unreachable!("the test does not write text")
    }

    fn temp_dir(&self, _parent: &str, _prefix: &str) -> Result<String, FsError> {
        unreachable!("the test makes no temporary directory")
    }

    fn remove(&self, _path: &str) -> Result<(), FsError> {
        unreachable!("the test removes nothing")
    }

    fn read_text(&self, path: &str) -> Result<String, FsError> {
        if let Some(error) = self.state.read_errors.borrow().get(path) {
            return Err(error.clone());
        }
        match self.state.files.borrow().get(path) {
            Some(text) => Ok(text.clone()),
            None => Err(FsError { code: "ENOENT".to_string(), message: format!("ENOENT: no such file or directory, open '{path}'") }),
        }
    }

    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        if let Some(error) = self.state.write_error.borrow().as_ref() {
            return Err(error.clone());
        }
        self.state.written.borrow_mut().push((path.to_string(), content.to_string()));
        self.state.files.borrow_mut().insert(path.to_string(), content.to_string());
        Ok(())
    }

    fn mkdir_all(&self, path: &str) -> Result<(), FsError> {
        if let Some(error) = self.state.mkdir_error.borrow().as_ref() {
            return Err(error.clone());
        }
        self.state.made.borrow_mut().push(path.to_string());
        self.state.dirs.borrow_mut().push(path.to_string());
        Ok(())
    }

    fn create_exclusive(&self, _path: &str) -> Result<bool, FsError> {
        if self.state.lock_present.get() {
            return Ok(false);
        }
        self.state.lock_present.set(true);
        Ok(true)
    }

    fn remove_file(&self, _path: &str) -> Result<(), FsError> {
        self.state.released.set(self.state.released.get() + 1);
        self.state.lock_present.set(false);
        Ok(())
    }

    fn modified_ms(&self, _path: &str) -> Option<i64> {
        self.state.lock_modified.get()
    }

    fn exists(&self, path: &str) -> Result<bool, FsError> {
        if let Some(error) = self.state.exists_errors.borrow().get(path) {
            return Err(error.clone());
        }
        Ok(self.state.files.borrow().contains_key(path) || self.state.dirs.borrow().iter().any(|dir| dir == path))
    }

    fn append(&self, path: &str, content: &str) -> Result<(), FsError> {
        if let Some(error) = self.state.append_error.borrow().as_ref() {
            return Err(error.clone());
        }
        self.state.appended.borrow_mut().push((path.to_string(), content.to_string()));
        self.state.files.borrow_mut().entry(path.to_string()).or_default().push_str(content);
        Ok(())
    }

    fn stamp(&self, _path: &str) -> Option<FileStamp> {
        None
    }

    fn write_private(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
}

/// git's answer about a repository, and every directory it was asked about.
#[derive(Clone)]
struct StubGit {
    root: Result<Option<String>, String>,
    asked: Rc<RefCell<Vec<String>>>,
}

impl StubGit {
    fn answering(root: Result<Option<String>, String>) -> StubGit {
        StubGit { root, asked: Rc::new(RefCell::new(Vec::new())) }
    }

    fn asked(&self) -> Vec<String> {
        self.asked.borrow().clone()
    }
}

impl Git for StubGit {
    fn common_dir(&self, _path: &str) -> Result<String, String> {
        unreachable!("the test asks for no common directory")
    }

    fn add_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        unreachable!("the test adds no worktree")
    }

    fn root(&self, path: &str) -> Result<Option<String>, String> {
        self.asked.borrow_mut().push(path.to_string());
        self.root.clone()
    }

    fn remotes(&self, _path: &str) -> Result<Vec<GitRemote>, String> {
        Ok(Vec::new())
    }

    fn remove_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        Ok(())
    }

    fn delete_branch(&self, _repo: &str, _branch: &str) -> Result<(), String> {
        Ok(())
    }
}

/// A clock that moves on at every reading, so a wait that never ends runs out at once.
struct TickingClock {
    now: Cell<i64>,
    step: i64,
}

impl Clock for TickingClock {
    fn monotonic_ms(&self) -> f64 {
        0.0
    }

    fn now_ms(&self) -> i64 {
        let value = self.now.get();
        self.now.set(value + self.step);
        value
    }
}

/// A runtime over the in-memory files and git, with `HOME=/home/me`.
fn memory_runtime(files: &MemoryFiles, git: StubGit, lock: LockTimings, clock: Box<dyn Clock>) -> Runtime {
    Runtime {
        clock,
        files: Box::new(files.clone()),
        lock,
        process: Box::new(SystemProcess { env: HashMap::new() }),
        git: Box::new(git),
        env: HashMap::from([("HOME".to_string(), "/home/me".to_string())]),
        defaults: String::new(),
        defaults_path: "defaults.json".to_string(),
        routing: crate::SHIPPED_ROUTING.to_string(),
    }
}

/// A runtime over a fresh sandbox, with the state home and the configuration
/// home inside it, and the real git on the host's PATH.
fn sandbox_runtime(sandbox: &Sandbox, lock: LockTimings) -> Runtime {
    let root = sandbox.text();
    std::fs::create_dir_all(format!("{root}/home")).expect("home");
    std::fs::create_dir_all(format!("{root}/state")).expect("state");
    let env = HashMap::from([
        ("HOME".to_string(), format!("{root}/home")),
        ("XDG_STATE_HOME".to_string(), format!("{root}/state")),
        ("PATH".to_string(), std::env::var("PATH").unwrap_or_default()),
    ]);
    Runtime {
        clock: Box::new(SystemClock),
        files: Box::new(SystemFiles { cwd: root.clone() }),
        lock,
        process: Box::new(SystemProcess { env: env.clone() }),
        git: Box::new(SystemGit { env: env.clone(), timeout_ms: TIMEOUT_MS }),
        env,
        defaults: crate::SHIPPED_DEFAULTS.to_string(),
        defaults_path: "defaults.json".to_string(),
        routing: crate::SHIPPED_ROUTING.to_string(),
    }
}

fn reservation(run_id: &str, run_dir: &str, estimate_usd: f64, expires_at: &str) -> ReservationRecord {
    ReservationRecord {
        run_id: run_id.to_string(),
        workflow: "spec-293".to_string(),
        client: "codex".to_string(),
        cost_class: CostClass::MeteredEmployer,
        model: "gpt-5.5".to_string(),
        estimate_usd,
        reserved_at: "2026-11-03T11:30:00.000Z".to_string(),
        expires_at: expires_at.to_string(),
        run_dir: run_dir.to_string(),
    }
}

fn settled(run_id: &str, cost_usd: f64) -> SettledRecord {
    SettledRecord {
        run_id: run_id.to_string(),
        workflow: "spec-293".to_string(),
        client: "codex".to_string(),
        cost_class: CostClass::MeteredEmployer,
        cost_usd,
        reserved_at: "2026-11-03T11:30:00.000Z".to_string(),
        settled_at: "2026-11-03T12:00:00.000Z".to_string(),
        reason: Some("expired".to_string()),
    }
}

fn reservation_line(run_id: &str, run_dir: &str, estimate_usd: f64, expires_at: &str) -> String {
    format!(
        r#"{{"kind":"reservation","run_id":"{run_id}","workflow":"spec-293","client":"codex","cost_class":"metered-employer","model":"gpt-5.5","estimate_usd":{estimate_usd},"reserved_at":"2026-11-03T11:30:00.000Z","expires_at":"{expires_at}","run_dir":"{run_dir}"}}"#
    )
}

fn settled_line(run_id: &str, cost_usd: f64) -> String {
    format!(
        r#"{{"kind":"settled","run_id":"{run_id}","workflow":"spec-293","client":"codex","cost_class":"metered-employer","cost_usd":{cost_usd},"reserved_at":"2026-11-03T11:30:00.000Z","settled_at":"2026-11-03T12:00:00.000Z","reason":"expired"}}"#
    )
}

fn grant() -> Grant {
    Grant {
        answer: "yes, up to five dollars".to_string(),
        cap_usd: 5.0,
        max_starts: None,
        client: "codex".to_string(),
        expires_at: "2026-11-05T10:52:15.000Z".to_string(),
        id: "grant-20261104T105215Z-abc123".to_string(),
        question: "may I spend up to $5 of nedap money on spec-293?".to_string(),
        workflow: "spec-293".to_string(),
        written_at: "2026-11-04T10:52:15.000Z".to_string(),
        ended_at: None,
    }
}

#[test]
fn parse_ledger_line_reads_a_run_line_leniently() {
    let full = r#"{"at":"2026-11-03T12:00:00.000Z","client":"vibe","cost_class":"subsidised","cost_usd":0.5,"model":"mistral-medium-3.5","outcome":"ok","reason":"timeout","unavailable_until":"2026-11-04T00:00:00.000Z","run_id":"one","tokens":{"cached":0,"input":10,"output":20},"workflow":"review"}"#;
    assert_eq!(
        parse_ledger_line(full),
        Some(LedgerEntry::Run(LedgerRecord {
            at: "2026-11-03T12:00:00.000Z".to_string(),
            client: "vibe".to_string(),
            cost_class: Some(CostClass::Subsidised),
            cost_usd: 0.5,
            model: "mistral-medium-3.5".to_string(),
            ok: true,
            reason: Some("timeout".to_string()),
            unavailable_until: Some("2026-11-04T00:00:00.000Z".to_string()),
            run_id: "one".to_string(),
            tokens: Some(RunTokens { input: 10.0, output: 20.0, cached: 0.0 }),
            workflow: "review".to_string(),
        }))
    );
    // A line that carries nothing the gateway reads is still a run, with every field empty.
    assert_eq!(
        parse_ledger_line("{}"),
        Some(LedgerEntry::Run(LedgerRecord {
            at: String::new(),
            client: String::new(),
            cost_class: None,
            cost_usd: 0.0,
            model: String::new(),
            ok: false,
            reason: None,
            unavailable_until: None,
            run_id: String::new(),
            tokens: None,
            workflow: String::new(),
        }))
    );
    // A cost class the gateway does not know, a cost that is not a number and an
    // outcome that is not `ok` are read as absent, not as a failure.
    let odd = r#"{"cost_class":"free","cost_usd":"0.5","outcome":"failed","tokens":{"input":1,"output":2}}"#;
    let Some(LedgerEntry::Run(record)) = parse_ledger_line(odd) else {
        panic!("a run line");
    };
    assert_eq!((record.cost_class, record.cost_usd, record.ok, record.tokens), (None, 0.0, false, None));
    // A `kind` that is not a string names no bookkeeping line, so the line is a run's own.
    let Some(LedgerEntry::Run(record)) = parse_ledger_line(r#"{"kind":7,"run_id":"two"}"#) else {
        panic!("a run line");
    };
    assert_eq!(record.run_id, "two");
}

#[test]
fn parse_ledger_line_refuses_a_line_that_is_not_a_json_object() {
    for line in ["", "not json", "{\"run_id\":", "[]", "null", "7", "\"text\""] {
        assert_eq!(parse_ledger_line(line), None, "{line:?}");
    }
}

#[test]
fn parse_ledger_line_reads_a_reservation_and_a_settlement() {
    assert_eq!(
        parse_ledger_line(&reservation_line("one", "/runs/one", 1.5, "2026-11-03T13:00:00.000Z")),
        Some(LedgerEntry::Reservation(reservation("one", "/runs/one", 1.5, "2026-11-03T13:00:00.000Z")))
    );
    assert_eq!(parse_ledger_line(&settled_line("one", 0.25)), Some(LedgerEntry::Settled(settled("one", 0.25))));
    // A settlement without a reason is one the gateway wrote for a run that cost nothing.
    let no_reason = r#"{"kind":"settled","run_id":"one","workflow":"spec-293","client":"codex","cost_class":"metered-employer","cost_usd":0,"reserved_at":"2026-11-03T11:30:00.000Z","settled_at":"2026-11-03T12:00:00.000Z"}"#;
    let Some(LedgerEntry::Settled(record)) = parse_ledger_line(no_reason) else {
        panic!("a settlement");
    };
    assert_eq!((record.cost_usd, record.reason), (0.0, None));
}

#[test]
fn parse_ledger_line_refuses_a_reservation_or_settlement_missing_a_field() {
    let fields = [
        ("run_id", "\"one\""),
        ("workflow", "\"spec-293\""),
        ("client", "\"codex\""),
        ("cost_class", "\"metered-employer\""),
        ("model", "\"gpt-5.5\""),
        ("estimate_usd", "1.5"),
        ("reserved_at", "\"2026-11-03T11:30:00.000Z\""),
        ("expires_at", "\"2026-11-03T13:00:00.000Z\""),
        ("run_dir", "\"/runs/one\""),
    ];
    let body = |skip: &str| -> String {
        let kept: Vec<String> = fields.iter().filter(|(key, _)| *key != skip).map(|(key, value)| format!("\"{key}\":{value}")).collect();
        format!("{{\"kind\":\"reservation\",{}}}", kept.join(","))
    };
    for (key, _) in fields {
        assert_eq!(parse_ledger_line(&body(key)), None, "without {key}");
    }
    // A field of the wrong type is as good as missing, and an estimate that is
    // not a finite number is no estimate at all.
    assert_eq!(parse_ledger_line(&body("run_id").replace("\"kind\":\"reservation\",", "\"kind\":\"reservation\",\"run_id\":7,")), None);
    assert_eq!(parse_ledger_line(&body("estimate_usd").replace("\"kind\":\"reservation\",", "\"kind\":\"reservation\",\"estimate_usd\":1e999,")), None);
    assert_eq!(parse_ledger_line(&body("cost_class").replace("\"kind\":\"reservation\",", "\"kind\":\"reservation\",\"cost_class\":\"free\",")), None);

    let settled_fields = [
        ("run_id", "\"one\""),
        ("workflow", "\"spec-293\""),
        ("client", "\"codex\""),
        ("cost_class", "\"metered-employer\""),
        ("cost_usd", "0.25"),
        ("reserved_at", "\"2026-11-03T11:30:00.000Z\""),
        ("settled_at", "\"2026-11-03T12:00:00.000Z\""),
    ];
    let settled_body = |skip: &str| -> String {
        let kept: Vec<String> = settled_fields.iter().filter(|(key, _)| *key != skip).map(|(key, value)| format!("\"{key}\":{value}")).collect();
        format!("{{\"kind\":\"settled\",{}}}", kept.join(","))
    };
    for (key, _) in settled_fields {
        assert_eq!(parse_ledger_line(&settled_body(key)), None, "without {key}");
    }
    assert_eq!(parse_ledger_line(&settled_body("cost_usd").replace("\"kind\":\"settled\",", "\"kind\":\"settled\",\"cost_usd\":\"0.25\",")), None);
}

#[test]
fn the_ledger_reads_every_line_it_can_and_skips_the_rest() {
    let text = format!(
        "{}\n\n   \n{{\"run_id\":\n{}\n[]\n{}\n",
        settled_line("one", 0.25),
        reservation_line("two", "/runs/two", 1.5, "2026-11-03T13:00:00.000Z"),
        settled_line("three", 0.5)
    );
    let files = MemoryFiles::with_file(LEDGER, &text);
    let runtime = memory_runtime(&files, StubGit::answering(Ok(None)), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    let entries = LedgerStore { runtime: &runtime }.read().expect("the ledger reads");
    assert_eq!(
        entries,
        vec![
            LedgerEntry::Settled(settled("one", 0.25)),
            LedgerEntry::Reservation(reservation("two", "/runs/two", 1.5, "2026-11-03T13:00:00.000Z")),
            LedgerEntry::Settled(settled("three", 0.5)),
        ]
    );
}

#[test]
fn a_ledger_that_is_not_there_holds_nothing() {
    let files = MemoryFiles::default();
    let runtime = memory_runtime(&files, StubGit::answering(Ok(None)), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    assert_eq!(LedgerStore { runtime: &runtime }.read(), Ok(Vec::new()));
}

#[test]
fn a_ledger_that_cannot_be_read_names_the_platform_error() {
    let files = MemoryFiles::default();
    files.fail_read(LEDGER, "EACCES", "EACCES: permission denied, open '/home/me/.local/state/agent-kit/model-gateway/ledger.jsonl'");
    let runtime = memory_runtime(&files, StubGit::answering(Ok(None)), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    assert_eq!(
        LedgerStore { runtime: &runtime }.read(),
        Err(GatewayError::new("unexpected", "EACCES: permission denied, open '/home/me/.local/state/agent-kit/model-gateway/ledger.jsonl'"))
    );
}

#[test]
fn an_update_appends_the_settlements_it_decides() {
    let files = MemoryFiles::with_file(LEDGER, &format!("{}\n", settled_line("one", 0.25)));
    let runtime = memory_runtime(&files, StubGit::answering(Ok(None)), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    let store = LedgerStore { runtime: &runtime };
    let result = store
        .update(|entries| {
            assert_eq!(entries.len(), 1, "the change sees the ledger as it is");
            LedgerChange { append: vec![settled("two", 0.5)], result: "decided" }
        })
        .expect("the update lands");
    assert_eq!(result, "decided");
    assert_eq!(files.made(), vec![STATE_DIR.to_string()]);
    assert_eq!(
        files.appended(),
        vec![(
            LEDGER.to_string(),
            "{\"client\":\"codex\",\"cost_class\":\"metered-employer\",\"cost_usd\":0.5,\"kind\":\"settled\",\"reason\":\"expired\",\"reserved_at\":\"2026-11-03T11:30:00.000Z\",\"run_id\":\"two\",\"settled_at\":\"2026-11-03T12:00:00.000Z\",\"workflow\":\"spec-293\"}\n"
                .to_string()
        )]
    );
    // The lock is taken and released, and the ledger now holds both lines.
    assert_eq!(files.released(), 1);
    let appended = files.appended();
    assert_eq!(files.text(LEDGER), Some(format!("{}\n{}", settled_line("one", 0.25), appended[0].1)));
}

#[test]
fn an_update_that_settles_nothing_appends_nothing() {
    let files = MemoryFiles::default();
    let runtime = memory_runtime(&files, StubGit::answering(Ok(None)), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    let result = LedgerStore { runtime: &runtime }.update(|entries| {
        assert!(entries.is_empty());
        LedgerChange { append: Vec::new(), result: 7 }
    });
    assert_eq!(result, Ok(7));
    assert!(files.appended().is_empty());
    assert_eq!(files.released(), 1);
}

#[test]
fn an_update_that_cannot_make_the_state_directory_names_the_platform_error() {
    let files = MemoryFiles::default();
    *files.state.mkdir_error.borrow_mut() = Some(FsError { code: "EACCES".to_string(), message: "EACCES: permission denied, mkdir '/home/me/.local'".to_string() });
    let runtime = memory_runtime(&files, StubGit::answering(Ok(None)), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    assert_eq!(
        LedgerStore { runtime: &runtime }.update(|_| LedgerChange { append: Vec::new(), result: () }),
        Err(GatewayError::new("unexpected", "EACCES: permission denied, mkdir '/home/me/.local'"))
    );
    assert_eq!(files.released(), 0);
}

#[test]
fn an_update_that_cannot_append_names_the_platform_error() {
    let files = MemoryFiles::default();
    *files.state.append_error.borrow_mut() = Some(FsError { code: "ENOSPC".to_string(), message: "ENOSPC: no space left on device, write".to_string() });
    let runtime = memory_runtime(&files, StubGit::answering(Ok(None)), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    assert_eq!(
        LedgerStore { runtime: &runtime }.update(|_| LedgerChange { append: vec![settled("one", 0.25)], result: () }),
        Err(GatewayError::new("unexpected", "ENOSPC: no space left on device, write"))
    );
    // The lock is released even though the append failed.
    assert_eq!(files.released(), 1);
}

#[test]
fn an_update_gives_up_when_another_process_holds_the_ledger_lock() {
    let files = MemoryFiles::default();
    files.state.lock_present.set(true);
    let runtime = memory_runtime(
        &files,
        StubGit::answering(Ok(None)),
        LockTimings { retry_ms: 0, stale_ms: 5_000, timeout_ms: 10 },
        Box::new(TickingClock { now: Cell::new(0), step: 1 }),
    );
    let error = LedgerStore { runtime: &runtime }.update(|_| LedgerChange { append: Vec::new(), result: () }).expect_err("the lock never came free");
    assert_eq!(error, GatewayError::new("unexpected", format!("{LEDGER_LOCK}: another gateway process held the ledger lock for 10 ms")));
    assert_eq!(files.released(), 0);
}

#[test]
fn an_update_breaks_a_lock_left_by_a_dead_writer() {
    let files = MemoryFiles::default();
    files.state.lock_present.set(true);
    files.state.lock_modified.set(Some(0));
    let runtime = memory_runtime(
        &files,
        StubGit::answering(Ok(None)),
        LockTimings { retry_ms: 0, stale_ms: 5_000, timeout_ms: 10_000 },
        Box::new(FixedClock(10_000)),
    );
    assert_eq!(LedgerStore { runtime: &runtime }.update(|_| LedgerChange { append: Vec::new(), result: "done" }), Ok("done"));
    // Once to break the stale lock, once to release the one it took.
    assert_eq!(files.released(), 2);
}

#[test]
fn settle_stale_settles_a_finished_run_and_an_expired_one() {
    let text = format!(
        "{}\n{}\n{}\n",
        reservation_line("finished", "/runs/finished", 1.0, "2026-11-03T13:00:00.000Z"),
        reservation_line("expired", "/runs/expired", 2.0, "2026-11-03T11:00:00.000Z"),
        reservation_line("waiting", "/runs/waiting", 3.0, "2026-11-03T13:00:00.000Z")
    );
    let files = MemoryFiles::with_file(LEDGER, &text);
    files.state.files.borrow_mut().insert("/runs/finished/result.json".to_string(), r#"{"cost_usd":0.25}"#.to_string());
    let runtime = memory_runtime(&files, StubGit::answering(Ok(None)), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    settle_stale(&runtime, FIXED_NOW).expect("the stale reservations settle");
    let appended = files.appended();
    assert_eq!(appended.len(), 1);
    assert_eq!(
        appended[0].1,
        format!(
            "{}\n{}\n",
            compact(&crate::caps::settled_record_json(&settled("finished", 0.25))),
            compact(&crate::caps::settled_record_json(&settled("expired", 2.0)))
        )
    );
    // The ledger now holds the settlements after the reservations it had.
    let entries = LedgerStore { runtime: &runtime }.read().expect("the ledger reads");
    assert_eq!(entries.len(), 5);
    assert_eq!(entries[3], LedgerEntry::Settled(settled("finished", 0.25)));
    assert_eq!(entries[4], LedgerEntry::Settled(settled("expired", 2.0)));
}

#[test]
fn settle_stale_leaves_a_reservation_whose_result_it_cannot_read() {
    let text = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        reservation_line("unreadable", "/runs/unreadable", 1.0, "2026-11-03T13:00:00.000Z"),
        reservation_line("torn", "/runs/torn", 1.5, "2026-11-03T13:00:00.000Z"),
        reservation_line("negative", "/runs/negative", 2.0, "2026-11-03T13:00:00.000Z"),
        reservation_line("text", "/runs/text", 3.0, "2026-11-03T13:00:00.000Z"),
        reservation_line("list", "/runs/list", 4.0, "2026-11-03T13:00:00.000Z"),
        reservation_line("infinite", "/runs/infinite", 5.0, "2026-11-03T13:00:00.000Z"),
        reservation_line("unlooked", "/runs/unlooked", 6.0, "2026-11-03T13:00:00.000Z")
    );
    let files = MemoryFiles::with_file(LEDGER, &text);
    {
        let mut held = files.state.files.borrow_mut();
        held.insert("/runs/torn/result.json".to_string(), "{\"cost_usd\":".to_string());
        held.insert("/runs/negative/result.json".to_string(), r#"{"cost_usd":-1}"#.to_string());
        held.insert("/runs/text/result.json".to_string(), r#"{"cost_usd":"1"}"#.to_string());
        held.insert("/runs/list/result.json".to_string(), "[]".to_string());
        held.insert("/runs/infinite/result.json".to_string(), r#"{"cost_usd":1e999}"#.to_string());
    }
    files.fail_read("/runs/unreadable/result.json", "EACCES", "EACCES: permission denied, open '/runs/unreadable/result.json'");
    files.fail_exists("/runs/unlooked/result.json", "EACCES", "EACCES: permission denied, access '/runs/unlooked/result.json'");
    let runtime = memory_runtime(&files, StubGit::answering(Ok(None)), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    settle_stale(&runtime, FIXED_NOW).expect("nothing settles, and nothing fails");
    assert!(files.appended().is_empty());
}

#[test]
fn settle_stale_reports_a_ledger_it_cannot_read() {
    let files = MemoryFiles::default();
    files.fail_read(LEDGER, "EISDIR", "EISDIR: illegal operation on a directory, read");
    let runtime = memory_runtime(&files, StubGit::answering(Ok(None)), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    assert_eq!(settle_stale(&runtime, FIXED_NOW), Err(GatewayError::new("unexpected", "EISDIR: illegal operation on a directory, read")));
}

#[test]
fn the_grants_are_read_from_a_file_outside_every_repository() {
    let files = MemoryFiles::with_file(GRANTS, &grant_file_text(&[grant()]));
    let git = StubGit::answering(Ok(None));
    let runtime = memory_runtime(&files, git.clone(), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    assert_eq!(GrantsStore { runtime: &runtime }.read(), Ok(vec![grant()]));
    // Nothing of the configuration directory exists yet, so git is asked about the root.
    assert_eq!(git.asked(), vec!["/".to_string()]);
}

#[test]
fn the_grants_ask_git_about_the_nearest_directory_that_exists() {
    let files = MemoryFiles::with_dir("/home/me/.config");
    let git = StubGit::answering(Ok(None));
    let runtime = memory_runtime(&files, git.clone(), LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    assert_eq!(GrantsStore { runtime: &runtime }.read(), Ok(Vec::new()));
    assert_eq!(git.asked(), vec!["/home/me/.config".to_string()]);
}

#[test]
fn a_grants_file_that_is_not_there_holds_none() {
    let files = MemoryFiles::with_dir(CONFIG_DIR);
    let git = StubGit::answering(Ok(None));
    let runtime = memory_runtime(&files, git, LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    assert_eq!(GrantsStore { runtime: &runtime }.read(), Ok(Vec::new()));
}

#[test]
fn a_grants_file_inside_a_repository_is_refused() {
    let files = MemoryFiles::with_dir(CONFIG_DIR);
    let git = StubGit::answering(Ok(Some("/home/me".to_string())));
    let runtime = memory_runtime(&files, git, LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    let store = GrantsStore { runtime: &runtime };
    let expected = GatewayError::new(
        "grants-in-repository",
        format!("{GRANTS} is inside the git repository /home/me: a grant lives in the machine configuration, never in a repository"),
    );
    assert_eq!(store.read(), Err(expected.clone()));
    assert_eq!(store.update(|_| Ok(GrantChange { grants: vec![grant()], grant: grant() })), Err(expected));
    assert!(files.written().is_empty());
}

#[test]
fn a_grants_file_whose_repository_git_cannot_name_is_refused() {
    let files = MemoryFiles::with_dir(CONFIG_DIR);
    let git = StubGit::answering(Err("fatal: detected dubious ownership in repository".to_string()));
    let runtime = memory_runtime(&files, git, LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    let store = GrantsStore { runtime: &runtime };
    let expected = GatewayError::new(
        "invalid-config",
        format!(
            "{GRANTS}: cannot tell whether {CONFIG_DIR} is inside a git repository, so no grant is read or written: fatal: detected dubious ownership in repository"
        ),
    );
    assert_eq!(store.read(), Err(expected.clone()));
    assert_eq!(store.update(|_| Ok(GrantChange { grants: vec![grant()], grant: grant() })), Err(expected));
}

#[test]
fn a_grants_file_that_cannot_be_read_names_the_platform_error() {
    let files = MemoryFiles::with_dir(CONFIG_DIR);
    files.fail_read(GRANTS, "EISDIR", "EISDIR: illegal operation on a directory, read");
    let git = StubGit::answering(Ok(None));
    let runtime = memory_runtime(&files, git, LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    assert_eq!(
        GrantsStore { runtime: &runtime }.read(),
        Err(GatewayError::new("invalid-config", format!("{GRANTS}: cannot read the grants: EISDIR: illegal operation on a directory, read")))
    );
}

#[test]
fn a_grants_file_that_is_not_json_names_the_file() {
    let files = MemoryFiles::with_file(GRANTS, "not json\n");
    let git = StubGit::answering(Ok(None));
    let runtime = memory_runtime(&files, git, LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    let error = GrantsStore { runtime: &runtime }.read().expect_err("the file is not JSON");
    assert_eq!(error.code, "invalid-config");
    assert!(error.message.starts_with(&format!("{GRANTS}: not JSON: ")), "{}", error.message);
}

#[test]
fn an_update_writes_the_grants_it_holds_and_reports_the_one_it_added() {
    let files = MemoryFiles::with_file(GRANTS, &grant_file_text(&[]));
    let git = StubGit::answering(Ok(None));
    let runtime = memory_runtime(&files, git, LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    let store = GrantsStore { runtime: &runtime };
    let written = grant();
    let changed = store
        .update(|held| {
            assert!(held.is_empty(), "the change sees the grants as they are");
            Ok(GrantChange { grants: vec![written.clone()], grant: written.clone() })
        })
        .expect("the grant is written");
    assert_eq!(changed, written);
    assert_eq!(files.made(), vec![CONFIG_DIR.to_string()]);
    assert_eq!(files.written(), vec![(GRANTS.to_string(), grant_file_text(&[written.clone()]))]);
    assert_eq!(files.released(), 1);
    assert_eq!(store.read(), Ok(vec![written]));
}

#[test]
fn an_update_that_cannot_make_the_configuration_directory_names_the_platform_error() {
    let files = MemoryFiles::default();
    *files.state.mkdir_error.borrow_mut() = Some(FsError { code: "EROFS".to_string(), message: "EROFS: read-only file system, mkdir '/home/me/.config'".to_string() });
    let git = StubGit::answering(Ok(None));
    let runtime = memory_runtime(&files, git, LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    assert_eq!(
        GrantsStore { runtime: &runtime }.update(|_| Ok(GrantChange { grants: vec![grant()], grant: grant() })),
        Err(GatewayError::new("unexpected", "EROFS: read-only file system, mkdir '/home/me/.config'"))
    );
    assert_eq!(files.released(), 0);
}

#[test]
fn an_update_that_cannot_write_names_the_platform_error() {
    let files = MemoryFiles::default();
    *files.state.write_error.borrow_mut() = Some(FsError { code: "ENOSPC".to_string(), message: "ENOSPC: no space left on device, write".to_string() });
    let git = StubGit::answering(Ok(None));
    let runtime = memory_runtime(&files, git, LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    assert_eq!(
        GrantsStore { runtime: &runtime }.update(|_| Ok(GrantChange { grants: vec![grant()], grant: grant() })),
        Err(GatewayError::new("unexpected", "ENOSPC: no space left on device, write"))
    );
    assert_eq!(files.released(), 1);
}

#[test]
fn an_update_gives_up_when_another_process_holds_the_grants_lock() {
    let files = MemoryFiles::default();
    files.state.lock_present.set(true);
    let git = StubGit::answering(Ok(None));
    let runtime = memory_runtime(
        &files,
        git,
        LockTimings { retry_ms: 0, stale_ms: 5_000, timeout_ms: 10 },
        Box::new(TickingClock { now: Cell::new(0), step: 1 }),
    );
    let error = GrantsStore { runtime: &runtime }
        .update(|_| Ok(GrantChange { grants: vec![grant()], grant: grant() }))
        .expect_err("the lock never came free");
    assert_eq!(error, GatewayError::new("unexpected", format!("{GRANTS_LOCK}: another gateway process held the grants lock for 10 ms")));
    assert_eq!(files.released(), 0);
}

#[test]
fn an_update_reports_what_the_change_refused() {
    let files = MemoryFiles::default();
    let git = StubGit::answering(Ok(None));
    let runtime = memory_runtime(&files, git, LockTimings::default(), Box::new(FixedClock(FIXED_NOW)));
    let refusal = GatewayError::new("invalid-input", "the owner said no");
    assert_eq!(GrantsStore { runtime: &runtime }.update(|_| Err(refusal.clone())), Err(refusal));
    assert!(files.written().is_empty());
    assert_eq!(files.released(), 1);
}

#[test]
fn a_real_ledger_skips_a_torn_line_and_a_blank_one() {
    let sandbox = Sandbox::new();
    let runtime = sandbox_runtime(&sandbox, LockTimings::default());
    let path = format!("{}/state/agent-kit/model-gateway/ledger.jsonl", sandbox.text());
    std::fs::create_dir_all(format!("{}/state/agent-kit/model-gateway", sandbox.text())).expect("state dir");
    std::fs::write(&path, format!("{}\n\n   \n{{\"run_id\":\n{}\n", settled_line("one", 0.25), settled_line("two", 0.5))).expect("write the ledger");
    let store = LedgerStore { runtime: &runtime };
    assert_eq!(store.read(), Ok(vec![LedgerEntry::Settled(settled("one", 0.25)), LedgerEntry::Settled(settled("two", 0.5))]));
    // An update appends to the same file, and the ledger reads both lines back.
    store.update(|_| LedgerChange { append: vec![settled("three", 0.75)], result: () }).expect("the settlement lands");
    let entries = store.read().expect("the ledger reads");
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[2], LedgerEntry::Settled(settled("three", 0.75)));
    assert!(!std::path::Path::new(&format!("{path}.lock")).exists(), "the lock is released");
}

#[test]
fn a_real_ledger_that_is_a_directory_fails_with_the_platform_error() {
    let sandbox = Sandbox::new();
    let runtime = sandbox_runtime(&sandbox, LockTimings::default());
    std::fs::create_dir_all(format!("{}/state/agent-kit/model-gateway/ledger.jsonl", sandbox.text())).expect("a directory where the ledger goes");
    let error = LedgerStore { runtime: &runtime }.read().expect_err("a directory is not a ledger");
    assert_eq!(error.code, "unexpected");
    assert!(error.message.starts_with("EISDIR: illegal operation on a directory"), "{}", error.message);
}

#[test]
fn a_real_lock_held_by_another_process_is_given_up_on() {
    let sandbox = Sandbox::new();
    let runtime = sandbox_runtime(&sandbox, LockTimings { retry_ms: 1, stale_ms: 5_000, timeout_ms: 20 });
    let lock = format!("{}/state/agent-kit/model-gateway/ledger.jsonl.lock", sandbox.text());
    std::fs::create_dir_all(format!("{}/state/agent-kit/model-gateway", sandbox.text())).expect("state dir");
    std::fs::write(&lock, "").expect("hold the lock");
    let error = LedgerStore { runtime: &runtime }
        .update(|_| LedgerChange { append: Vec::new(), result: () })
        .expect_err("the lock never came free");
    assert_eq!(error, GatewayError::new("unexpected", format!("{lock}: another gateway process held the ledger lock for 20 ms")));
    assert!(std::path::Path::new(&lock).exists(), "the lock it could not take is left alone");
}

#[test]
fn a_real_stale_lock_is_broken() {
    let sandbox = Sandbox::new();
    let runtime = sandbox_runtime(&sandbox, LockTimings { retry_ms: 1, stale_ms: 5_000, timeout_ms: 10_000 });
    let lock = format!("{}/state/agent-kit/model-gateway/ledger.jsonl.lock", sandbox.text());
    std::fs::create_dir_all(format!("{}/state/agent-kit/model-gateway", sandbox.text())).expect("state dir");
    std::fs::write(&lock, "").expect("hold the lock");
    let held = std::fs::File::options().write(true).open(&lock).expect("open the lock");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    held.set_times(std::fs::FileTimes::new().set_modified(old)).expect("age the lock");
    assert_eq!(LedgerStore { runtime: &runtime }.update(|_| LedgerChange { append: Vec::new(), result: "done" }), Ok("done"));
    assert!(!std::path::Path::new(&lock).exists(), "the stale lock is gone");
}

#[test]
fn settle_stale_reads_a_real_result_file() {
    let sandbox = Sandbox::new();
    let runtime = sandbox_runtime(&sandbox, LockTimings::default());
    let root = sandbox.text();
    let run_dir = format!("{root}/runs/one");
    std::fs::create_dir_all(&run_dir).expect("run dir");
    std::fs::write(format!("{run_dir}/result.json"), r#"{"cost_usd":0.4}"#).expect("result");
    std::fs::create_dir_all(format!("{root}/state/agent-kit/model-gateway")).expect("state dir");
    std::fs::write(
        format!("{root}/state/agent-kit/model-gateway/ledger.jsonl"),
        format!("{}\n", reservation_line("one", &run_dir, 1.0, "2026-11-03T13:00:00.000Z")),
    )
    .expect("ledger");
    settle_stale(&runtime, FIXED_NOW).expect("the finished run settles");
    let entries = LedgerStore { runtime: &runtime }.read().expect("the ledger reads");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1], LedgerEntry::Settled(settled("one", 0.4)));
}

#[test]
fn a_real_grants_file_outside_a_repository_is_written_and_read_back() {
    let sandbox = Sandbox::new();
    let runtime = sandbox_runtime(&sandbox, LockTimings::default());
    let store = GrantsStore { runtime: &runtime };
    assert_eq!(store.read(), Ok(Vec::new()));
    let written = grant();
    let changed = store
        .update(|held| {
            let mut grants = held.to_vec();
            grants.push(written.clone());
            Ok(GrantChange { grants, grant: written.clone() })
        })
        .expect("the grant is written");
    assert_eq!(changed, written);
    assert_eq!(store.read(), Ok(vec![written.clone()]));
    let path = format!("{}/home/.config/agent-kit/model-gateway-grants.json", sandbox.text());
    assert_eq!(std::fs::read_to_string(&path).expect("the grants file"), grant_file_text(&[written]));
    // Nothing but the grants file is left in the directory: the lock and the temporary file are gone.
    let left: Vec<String> = std::fs::read_dir(format!("{}/home/.config/agent-kit", sandbox.text()))
        .expect("the configuration directory")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, vec!["model-gateway-grants.json".to_string()]);
}

#[test]
fn a_real_grants_file_inside_a_repository_is_refused() {
    let sandbox = Sandbox::new();
    let root = sandbox.text();
    let status = std::process::Command::new("git").arg("-C").arg(&root).args(["init", "-q"]).status().expect("run git");
    assert!(status.success(), "git init");
    let runtime = sandbox_runtime(&sandbox, LockTimings::default());
    let error = GrantsStore { runtime: &runtime }.read().expect_err("a grant never lives in a repository");
    assert_eq!(error.code, "grants-in-repository");
    assert_eq!(
        error.message,
        format!("{root}/home/.config/agent-kit/model-gateway-grants.json is inside the git repository {root}: a grant lives in the machine configuration, never in a repository")
    );
}

#[test]
fn a_real_grants_file_that_is_a_directory_cannot_be_read() {
    let sandbox = Sandbox::new();
    let runtime = sandbox_runtime(&sandbox, LockTimings::default());
    let path = format!("{}/home/.config/agent-kit/model-gateway-grants.json", sandbox.text());
    std::fs::create_dir_all(&path).expect("a directory where the grants go");
    let error = GrantsStore { runtime: &runtime }.read().expect_err("a directory is not a grants file");
    assert_eq!(error.code, "invalid-config");
    assert_eq!(error.message, format!("{path}: cannot read the grants: EISDIR: illegal operation on a directory, read"));
}

#[test]
fn a_real_grants_file_of_another_version_or_not_json_names_the_file() {
    let sandbox = Sandbox::new();
    let runtime = sandbox_runtime(&sandbox, LockTimings::default());
    let path = format!("{}/home/.config/agent-kit/model-gateway-grants.json", sandbox.text());
    std::fs::create_dir_all(format!("{}/home/.config/agent-kit", sandbox.text())).expect("configuration directory");
    std::fs::write(&path, "{\"grants\":[],\"version\":2}\n").expect("write the grants");
    assert_eq!(
        GrantsStore { runtime: &runtime }.read(),
        Err(GatewayError::new("invalid-config", format!("{path}: version must be 1")))
    );
    std::fs::write(&path, "not json\n").expect("write the grants");
    let error = GrantsStore { runtime: &runtime }.read().expect_err("not JSON");
    assert!(error.message.starts_with(&format!("{path}: not JSON: ")), "{}", error.message);
}
