//! Tests for the `batch` command: the plan is checked as a whole before anything runs, its metered estimates are
//! reserved together, and its independent delegations run in parallel, each through the single-delegation path.

use super::machine::{now_ms, Machine, DEFAULTS};
use crate::adapters::fs::SystemFiles;
use crate::batch_command::BatchCommand;
use crate::error::GatewayError;
use crate::json::{parse, Value};
use crate::ports::{Clock, Files, FsError, LockTimings, Process, ProcessCommand, ProcessResult};
use crate::run_command::RunCommand;
use crate::slots::ClientSlots;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const CLAUDE_ANSWER: &str = r#"{"type":"result","result":"done\nSUMMARY: fine","usage":{"input_tokens":10,"output_tokens":5}}"#;

fn answer(stdout: &str) -> Result<ProcessResult, String> {
    Ok(ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code: 0, timed_out: false })
}

/// A machine with the clients a batch is tested against installed and answering, and a temporary directory of its own.
fn machine() -> Machine {
    let mut machine = Machine::new(DEFAULTS);
    machine.process.install("claude");
    machine.process.install("hermes");
    machine.process.answer("claude", answer(CLAUDE_ANSWER));
    machine.process.answer("hermes", answer(CLAUDE_ANSWER));
    // Hermes runs on a provider, and this machine allows the one the tests name.
    machine.write_override(r#"{"clients":{"hermes":{"providers":["anthropic"]}}}"#);
    let tmp = format!("{}/tmp", machine.sandbox.text());
    std::fs::create_dir_all(&tmp).expect("tmp");
    machine.runtime.env.insert("TMPDIR".to_string(), tmp);
    machine
}

/// Runs one batch over the machine's runtime.
fn batch(machine: &Machine, input: &str) -> Result<Value, GatewayError> {
    let runs = RunCommand { runtime: &machine.runtime };
    BatchCommand { runtime: &machine.runtime, runs: &runs }.run(&parse(input).expect("JSON"))
}

/// A plan of the delegations given, in the workflow `w`.
fn plan(delegations: &[String]) -> String {
    format!(r#"{{"workflow":"w","delegations":[{}]}}"#, delegations.join(","))
}

/// One read-only delegation of the plan, with the delegations it waits for.
fn delegation(id: &str, client: &str, model: &str, depends_on: &[&str]) -> String {
    let depends = if depends_on.is_empty() {
        String::new()
    } else {
        format!(r#","depends_on":[{}]"#, depends_on.iter().map(|id| format!("\"{id}\"")).collect::<Vec<_>>().join(","))
    };
    format!(
        r#"{{"id":"{id}"{depends},"client":"{client}","model":"{model}","mode":"read-only","prompt":"TASK-{id}","escalate":false}}"#
    )
}

fn claude(id: &str, depends_on: &[&str]) -> String {
    delegation(id, "claude", "claude-sonnet-5-5", depends_on)
}

fn hermes(id: &str, depends_on: &[&str]) -> String {
    delegation(id, "hermes", "hermes-hard", depends_on).replace(r#""mode""#, r#""provider":"anthropic","mode""#)
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).unwrap_or_else(|| panic!("no {key} in {value:?}"))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    field(value, key).as_str().unwrap_or_default()
}

fn number(value: &Value, key: &str) -> f64 {
    match field(value, key) {
        Value::Num(number) => *number,
        other => panic!("{key} is not a number: {other:?}"),
    }
}

fn delegations(result: &Value) -> Vec<Value> {
    match field(result, "delegations") {
        Value::Arr(items) => items.clone(),
        other => panic!("delegations is not a list: {other:?}"),
    }
}

fn statuses(reports: &[Value]) -> Vec<&str> {
    reports.iter().map(|report| text(report, "status")).collect()
}

/// The run directories the machine left behind.
fn run_directories(machine: &Machine) -> Vec<String> {
    let path = format!("{}/state/agent-kit/model-gateway/runs", machine.sandbox.text());
    std::fs::read_dir(path).map(|entries| entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default()
}

#[test]
fn a_plan_with_a_cycle_is_refused_with_every_problem_and_launches_nothing() {
    let machine = machine();
    let error = batch(&machine, &plan(&[claude("a", &["b"]), claude("b", &["a"])])).expect_err("a cycle");

    assert_eq!(error.code, "plan-refused");
    let seen: Vec<String> = error.problems.iter().map(|problem| format!("{}:{}", problem.id, problem.code)).collect();
    assert_eq!(seen, vec!["a:cycle", "b:cycle"]);
    assert_eq!(
        error.message,
        "the plan is refused, and nothing was launched: a: is part of a dependency cycle, so it could never start; b: is part of a dependency cycle, so it could never start"
    );
    assert!(machine.ledger_text().is_empty(), "{}", machine.ledger_text());
    assert!(run_directories(&machine).is_empty());
}

#[test]
fn a_plan_that_waits_for_a_delegation_it_does_not_hold_is_refused() {
    let machine = machine();
    let error = batch(&machine, &plan(&[claude("a", &["nope"])])).expect_err("an unknown dependency");

    assert_eq!(error.code, "plan-refused");
    assert_eq!(error.problems.len(), 1);
    assert_eq!(error.problems[0].code, "unknown-dependency");
    assert_eq!(error.problems[0].id, "a");
    assert_eq!(error.problems[0].message, "depends_on names nope, which is not in the plan");
}

#[test]
fn a_plan_with_more_delegations_than_the_machine_allows_is_refused_first() {
    let machine = machine();
    machine.write_override(r#"{"settings":{"max_batch_delegations":1}}"#);
    let error = batch(&machine, &plan(&[claude("a", &[]), claude("b", &[])])).expect_err("too many");

    assert_eq!(error.code, "plan-refused");
    assert_eq!(error.problems.len(), 1);
    assert_eq!(error.problems[0].code, "too-many-delegations");
    assert_eq!(error.problems[0].id, "(plan)");
    assert!(machine.ledger_text().is_empty());
}

#[test]
fn a_plan_whose_estimates_do_not_fit_the_caps_is_refused_and_holds_nothing() {
    let machine = machine();
    machine.write_override(
        r#"{"clients":{"hermes":{"providers":["anthropic"]}},"settings":{"metered_personal_caps":{"per_day_usd":0.0001,"per_workflow_usd":0.0001}}}"#,
    );
    let error = batch(&machine, &plan(&[hermes("a", &[]), hermes("b", &[])])).expect_err("the caps");

    assert_eq!(error.code, "plan-refused");
    assert_eq!(error.problems.len(), 2);
    assert!(error.problems.iter().all(|problem| problem.code == "cap-exceeded"), "{:?}", error.problems);
    assert_eq!(error.problems.iter().map(|problem| problem.id.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
    assert!(machine.reservations().is_empty(), "{}", machine.ledger_text());
    assert!(run_directories(&machine).is_empty());
}

#[test]
fn a_delegation_that_fails_for_good_skips_only_what_depends_on_it_to_the_end_of_the_chain() {
    let machine = machine();
    machine.process.answer("claude", Err("spawn claude ENOENT".to_string()));
    let result = batch(&machine, &plan(&[claude("a", &[]), claude("b", &["a"]), claude("c", &["b"]), claude("d", &[])])).expect("a result");

    let reports = delegations(&result);
    assert_eq!(statuses(&reports), vec!["failed", "skipped", "skipped", "failed"]);
    assert_eq!(text(&reports[1], "skipped_because"), "a");
    assert_eq!(text(&reports[2], "skipped_because"), "a");
    assert_eq!(text(&reports[0], "reason"), "launch-failed");
    assert_eq!(text(&result, "status"), "failed");
}

#[test]
fn the_reservation_of_a_delegation_that_never_ran_is_given_back() {
    let machine = machine();
    machine.process.answer("claude", Err("spawn claude ENOENT".to_string()));
    let result = batch(&machine, &plan(&[claude("a", &[]), hermes("b", &["a"])])).expect("a result");

    let reports = delegations(&result);
    assert_eq!(statuses(&reports), vec!["failed", "skipped"]);
    assert_eq!(text(&reports[1], "skipped_because"), "a");
    assert!(machine.ledger_text().contains(r#""reason":"not-run""#), "{}", machine.ledger_text());
    assert!(number(&result, "reservations_released_usd") > 0.0, "{result:?}");
}

#[test]
fn a_delegation_that_threw_is_reported_failed_with_what_its_attempts_cost() {
    let mut machine = machine();
    machine.runtime.clock = Box::new(Ticking(AtomicI64::new(now_ms())));
    machine.runtime.lock = LockTimings { retry_ms: 1, stale_ms: 600_000_000, timeout_ms: 50 };
    let lock = format!("{}/state/agent-kit/model-gateway/workflows.json.lock", machine.sandbox.text());
    machine.write_at(&lock, "");

    let result = batch(&machine, &plan(&[claude("a", &[])])).expect("a result");

    let reports = delegations(&result);
    assert_eq!(statuses(&reports), vec!["failed"]);
    assert_eq!(text(&reports[0], "reason"), "unexpected");
    assert!(text(&reports[0], "summary").contains("workflows lock"), "{:?}", reports[0]);
    assert_eq!(number(&reports[0], "cost_usd"), 0.0);
    assert_eq!(text(&result, "status"), "failed");
}

#[test]
fn a_ledger_that_is_lost_mid_batch_still_returns_the_batch() {
    let mut machine = machine();
    let switch = Arc::new(Switch::default());
    machine.runtime.files = Box::new(FlakyLedger { inner: SystemFiles { cwd: machine.sandbox.text() }, switch: Arc::clone(&switch) });
    machine.runtime.process = Box::new(FlippingProcess { switch: Arc::clone(&switch) });

    let result = batch(&machine, &plan(&[claude("a", &[])])).expect("a result");

    let reports = delegations(&result);
    assert_eq!(statuses(&reports), vec!["failed"]);
    assert_eq!(text(&reports[0], "reason"), "ledger-unreadable");
    assert_eq!(field(&reports[0], "cost_usd"), &Value::Null);
    assert_eq!(field(&result, "cost_usd"), &Value::Null);
    assert_eq!(field(&result, "reservations_released_usd"), &Value::Null);
}

#[test]
fn a_delegation_whose_run_was_recorded_before_it_threw_reports_its_attempts_and_gives_its_reservation_back() {
    let mut machine = machine();
    // The first read of the ledger after the client ran (the run's settling) fails, and every later one works.
    let switch = Arc::new(Switch { flaky: AtomicUsize::new(1), ..Switch::default() });
    machine.runtime.files = Box::new(FlakyLedger { inner: SystemFiles { cwd: machine.sandbox.text() }, switch: Arc::clone(&switch) });
    machine.runtime.process = Box::new(FlippingProcess { switch: Arc::clone(&switch) });

    let result = batch(&machine, &plan(&[hermes("a", &[])])).expect("a result");

    let reports = delegations(&result);
    assert_eq!(statuses(&reports), vec!["failed"]);
    assert_ne!(text(&reports[0], "reason"), "ledger-unreadable", "{:?}", reports[0]);
    let attempts = field(&reports[0], "attempts");
    assert!(matches!(attempts, Value::Arr(list) if list.len() == 1), "{attempts:?}");
}

/// A client whose launch panics, as a bug deep in the run path would.
struct Panicking(AtomicUsize);

impl Process for Panicking {
    fn resolve(&self, binary: &str) -> Option<String> {
        (binary == "claude").then(|| format!("/bin/{binary}"))
    }
    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Ok(ProcessResult { stdout: String::new(), stderr: String::new(), exit_code: 0, timed_out: false })
    }
    fn run(&self, _command: &ProcessCommand) -> Result<ProcessResult, String> {
        // A panic carries a string slice, an owned string or anything else.
        match self.0.fetch_add(1, Ordering::SeqCst) {
            0 => panic!("the client exploded"),
            1 => panic!("{}", "the client exploded, owned".to_string()),
            _ => std::panic::panic_any(7_u8),
        }
    }
}

#[test]
fn a_delegation_that_panics_is_a_failed_delegation_and_never_a_lost_batch() {
    let mut machine = machine();
    machine.runtime.process = Box::new(Panicking(AtomicUsize::new(0)));
    // One at a time, so the three panics come in the order of the plan.
    machine.write_override(r#"{"settings":{"max_parallel_per_client":1}}"#);

    let result = batch(&machine, &plan(&[claude("a", &[]), claude("b", &[]), claude("c", &[]), claude("d", &["c"])])).expect("a result");

    let reports = delegations(&result);
    assert_eq!(statuses(&reports), vec!["failed", "failed", "failed", "skipped"]);
    assert!(reports[..3].iter().all(|report| text(report, "reason") == "unexpected"));
    let summaries: Vec<&str> = reports[..3].iter().map(|report| text(report, "summary")).collect();
    assert_eq!(summaries, vec!["the client exploded", "the client exploded, owned", "a delegation panicked"]);
    assert_eq!(text(&result, "status"), "failed");
}

#[test]
fn the_result_lists_every_delegation_in_plan_order_with_what_it_cost_and_how_the_batch_ended() {
    let machine = machine();
    let result = batch(&machine, &plan(&[claude("a", &[]), claude("b", &[])])).expect("a result");

    assert_eq!(text(&result, "status"), "ok");
    assert_eq!(text(&result, "workflow"), "w");
    assert!(text(&result, "batch_id").starts_with("batch-"), "{}", text(&result, "batch_id"));
    assert_eq!(number(&result, "cost_usd"), 0.0);
    assert_eq!(number(&result, "reservations_released_usd"), 0.0);

    let reports = delegations(&result);
    assert_eq!(statuses(&reports), vec!["done", "done"]);
    assert_eq!(text(&reports[0], "id"), "a");
    assert_eq!(text(&reports[0], "client"), "claude");
    assert_eq!(text(&reports[0], "model"), "claude-sonnet-5-5");
    assert_eq!(text(&reports[0], "summary"), "fine");
    assert!(text(&reports[0], "output_path").ends_with("output.json"), "{:?}", reports[0]);
    assert!(reports[0].as_object().and_then(|object| object.get("reason")).is_none(), "{:?}", reports[0]);

    // Every ledger line of an attempt carries the batch and the delegation it was one of.
    let ledger = machine.ledger_text();
    assert!(ledger.contains(r#""batch_id":"batch-"#), "{ledger}");
    assert!(ledger.contains(r#""delegation":"a""#), "{ledger}");
    assert!(ledger.contains(r#""delegation":"b""#), "{ledger}");
}

#[test]
fn a_dependent_is_launched_with_the_outputs_of_the_delegations_it_waits_for() {
    let machine = machine();
    let result = batch(&machine, &plan(&[claude("a", &[]), claude("b", &["a"])])).expect("a result");

    let reports = delegations(&result);
    assert_eq!(statuses(&reports), vec!["done", "done"]);
    // The dependent's prompt sits beside the output its result names.
    let output = text(&reports[1], "output_path");
    let run_dir = output.rsplit_once('/').map(|(dir, _)| dir).unwrap_or_default();
    let prompt = std::fs::read_to_string(format!("{run_dir}/prompt.md")).expect("the dependent's prompt");
    assert!(prompt.contains("Outputs of the delegations this one depends on"), "{prompt}");
    assert!(prompt.contains("- a: output at "), "{prompt}");
    assert!(prompt.contains("SUMMARY: fine") || prompt.contains("fine"), "{prompt}");
}

#[test]
fn at_most_the_parallel_limit_of_one_client_runs_at_once() {
    let mut machine = machine();
    let seen = Arc::new(Concurrency::default());
    machine.runtime.process = Box::new(SlowProcess {
        installed: vec!["claude".to_string()],
        answer: CLAUDE_ANSWER.to_string(),
        delay_ms: 120,
        seen: Arc::clone(&seen),
    });

    let result = batch(&machine, &plan(&[claude("a", &[]), claude("b", &[]), claude("c", &[]), claude("d", &[])])).expect("a result");

    assert_eq!(statuses(&delegations(&result)), vec!["done", "done", "done", "done"]);
    let peak = seen.peak.load(Ordering::SeqCst);
    assert!(peak <= 3, "the limit of three was passed: {peak}");
    assert!(peak >= 2, "the delegations did not run in parallel: {peak}");
}

#[test]
fn a_slot_is_given_to_the_attempt_that_asked_first() {
    let slots = ClientSlots::new(1);
    let held = slots.acquire("claude");
    let order: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for name in ["first", "second"] {
            let order = &order;
            let slots = &slots;
            scope.spawn(move || {
                let guard = slots.acquire("claude");
                order.lock().expect("the order").push(name);
                drop(guard);
            });
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
        drop(held);
    });
    assert_eq!(*order.lock().expect("the order"), vec!["first", "second"]);
}

/// A clock that moves on at every reading, so a wait with a deadline ends.
struct Ticking(AtomicI64);

impl Clock for Ticking {
    fn now_ms(&self) -> i64 {
        self.0.fetch_add(100, Ordering::SeqCst) + 100
    }
    fn monotonic_ms(&self) -> f64 {
        0.0
    }
}

/// Whether the ledger has been lost yet.
#[derive(Default)]
struct Switch {
    lost: AtomicBool,
    /// How many of the reads after the client ran still fail, when the ledger is not lost for good.
    flaky: AtomicUsize,
    /// Whether the client has run yet.
    ran: AtomicBool,
}

/// The real files, except that the ledger cannot be read once the client has run.
struct FlakyLedger {
    inner: SystemFiles,
    switch: Arc<Switch>,
}

impl Files for FlakyLedger {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        let flaky = self.switch.ran.load(Ordering::SeqCst)
            && self.switch.flaky.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1)).is_ok();
        if path.ends_with("ledger.jsonl") && (flaky || self.switch.lost.load(Ordering::SeqCst)) {
            return Err(FsError { code: "EIO".to_string(), message: "EIO: i/o error, read".to_string() });
        }
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
    fn stamp(&self, path: &str) -> Option<crate::ports::FileStamp> {
        self.inner.stamp(path)
    }
    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.inner.write_private(path, content)
    }
    fn write_text(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.inner.write_text(path, content)
    }
    fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, FsError> {
        self.inner.temp_dir(parent, prefix)
    }
    fn remove(&self, path: &str) -> Result<(), FsError> {
        self.inner.remove(path)
    }
}

/// A client that answers, and loses the ledger as it does.
struct FlippingProcess {
    switch: Arc<Switch>,
}

impl Process for FlippingProcess {
    fn resolve(&self, binary: &str) -> Option<String> {
        matches!(binary, "claude" | "hermes").then(|| format!("/bin/{binary}"))
    }
    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.switch.ran.store(true, Ordering::SeqCst);
        if self.switch.flaky.load(Ordering::SeqCst) == 0 {
            self.switch.lost.store(true, Ordering::SeqCst);
        }
        answer(CLAUDE_ANSWER)
    }
    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
        self.exec(&command.command, &args, command.timeout_ms)
    }
}

/// How many clients ran at once, and the most that ever did.
#[derive(Default)]
struct Concurrency {
    running: AtomicUsize,
    peak: AtomicUsize,
}

/// A client that takes its time, so the delegations that run together are seen together.
struct SlowProcess {
    installed: Vec<String>,
    answer: String,
    delay_ms: u64,
    seen: Arc<Concurrency>,
}

impl Process for SlowProcess {
    fn resolve(&self, binary: &str) -> Option<String> {
        self.installed.iter().any(|name| name == binary).then(|| format!("/bin/{binary}"))
    }
    fn exec(&self, command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        if !self.installed.iter().any(|name| name == command) {
            return Err(format!("cannot run {command}"));
        }
        let now = self.seen.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.seen.peak.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
        self.seen.running.fetch_sub(1, Ordering::SeqCst);
        answer(&self.answer)
    }
    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
        self.exec(&command.command, &args, command.timeout_ms)
    }
}
