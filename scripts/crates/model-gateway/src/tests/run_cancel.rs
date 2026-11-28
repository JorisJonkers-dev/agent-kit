//! Tests for `cancel`: the client's process group and sandbox are ended, and the log ends with exactly one finish.

use super::machine::{Machine, DEFAULTS};
use crate::json::{parse, Object, Value};
use crate::ports::{LaunchWatch, Process, ProcessCommand, ProcessResult};
use crate::run_cancel::{cancel, cancel_with, text, CancelWaits};
use crate::run_events::{RunEvents, RunIdentity, RunStart};
use crate::stall::cancel_reason;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A process port that records the signals and the commands it is given, and whose gateway and client groups the
/// test decides the life of.
struct Recorder {
    signals: Mutex<Vec<(u32, i32)>>,
    execs: Mutex<Vec<String>>,
    gateway_dead: AtomicBool,
    group_alive: AtomicBool,
    stubborn: bool,
}

impl Recorder {
    fn new(stubborn: bool) -> Recorder {
        Recorder { signals: Mutex::new(Vec::new()), execs: Mutex::new(Vec::new()), gateway_dead: AtomicBool::new(false), group_alive: AtomicBool::new(true), stubborn }
    }

    fn sent(&self) -> Vec<(u32, i32)> {
        self.signals.lock().expect("signals").iter().copied().filter(|(_, signal)| *signal != 0).collect()
    }
}

impl Process for Recorder {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }
    fn exec(&self, command: &str, args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.execs.lock().expect("execs").push(format!("{command} {}", args.join(" ")));
        Ok(ProcessResult { stdout: String::new(), stderr: String::new(), exit_code: 0, timed_out: false })
    }
    fn run(&self, _command: &ProcessCommand) -> Result<ProcessResult, String> {
        Err("no".into())
    }
    fn signal_group(&self, pid: u32, signal: i32) -> bool {
        self.signals.lock().expect("signals").push((pid, signal));
        match signal {
            0 => self.group_alive.load(Ordering::SeqCst),
            15 if self.stubborn => true,
            _ => {
                self.group_alive.store(false, Ordering::SeqCst);
                true
            }
        }
    }
    fn is_alive(&self, _pid: u32) -> bool {
        !self.gateway_dead.load(Ordering::SeqCst)
    }
}

fn waits() -> CancelWaits {
    CancelWaits { grace_ms: 150, finish_ms: 400, poll_ms: 5 }
}

fn input(run: &str) -> Object {
    match parse(&format!(r#"{{"run":"{run}"}}"#)).expect("JSON") {
        Value::Obj(object) => object,
        other => panic!("{other:?}"),
    }
}

fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).unwrap_or_else(|| panic!("no {key} in {value:?}"))
}

/// The recorder, shared between the machine and the test.
struct Shared(Arc<Recorder>);

impl Process for Shared {
    fn resolve(&self, binary: &str) -> Option<String> {
        self.0.resolve(binary)
    }
    fn exec(&self, command: &str, args: &[&str], timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.0.exec(command, args, timeout_ms)
    }
    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.0.run(command)
    }
    fn signal_group(&self, pid: u32, signal: i32) -> bool {
        self.0.signal_group(pid, signal)
    }
    fn is_alive(&self, pid: u32) -> bool {
        self.0.is_alive(pid)
    }
}

/// A machine that holds the recorder as its process port.
fn running(recorder: Recorder) -> (Machine, String, Arc<Recorder>) {
    let mut machine = Machine::new(DEFAULTS);
    let recorder = Arc::new(recorder);
    machine.runtime.process = Box::new(Shared(Arc::clone(&recorder)));
    (machine, "r-cancel".to_string(), recorder)
}

fn start<'a>(machine: &'a Machine, id: &str, with_client: bool) -> RunEvents<'a> {
    let dir = format!("{}/state/agent-kit/model-gateway/runs/{id}", machine.sandbox.text());
    std::fs::create_dir_all(&dir).expect("run dir");
    let identity = RunIdentity { run_id: id.into(), workflow: "w".into(), client: "pi".into(), model: "free/hy4-flash".into(), family: "tencent".into() };
    let index = crate::locations::run_index_path(&machine.runtime.env).expect("index path");
    let events = RunEvents::start(machine.runtime.files.as_ref(), machine.runtime.clock.as_ref(), &dir, identity, &RunStart { mode: "read-only", time_limit_s: 900.0 }).indexed(Some(&index));
    if with_client {
        events.spawned(4242);
        events.sandbox("mg-9-9");
    }
    events
}

fn log_kinds(machine: &Machine, id: &str) -> Vec<String> {
    let path = format!("{}/state/agent-kit/model-gateway/runs/{id}/events.jsonl", machine.sandbox.text());
    std::fs::read_to_string(path).expect("the log").lines().filter_map(|line| parse(line).ok()).filter_map(|event| event.as_object()?.get("type")?.as_str().map(str::to_string)).collect()
}

#[test]
fn a_cancel_ends_the_clients_group_and_its_sandbox_and_the_runs_gateway_writes_the_one_finish() {
    let (machine, id, rec) = running(Recorder::new(false));
    let events = start(&machine, &id, true);
    let finished = std::thread::scope(|scope| {
        // The run's own gateway: once the client's group is gone it writes the finish, as the result does.
        scope.spawn(|| {
            while rec.sent().is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            events.finish(parse(r#"{"status":"cancelled","reason":"cancelled"}"#).expect("JSON"));
        });
        cancel_with(&input("r-can"), &machine.runtime, waits()).expect("a cancel")
    });
    assert_eq!((get(&finished, "cancelled"), get(&finished, "finished"), get(&finished, "state")), (&Value::Bool(true), &Value::Bool(true), &Value::str("cancelled")));
    assert_eq!(rec.sent(), [(4242, 15)]);
    assert_eq!(rec.execs.lock().expect("execs").as_slice(), ["sbx rm --force mg-9-9"]);
    let kinds = log_kinds(&machine, &id);
    assert_eq!(kinds.iter().filter(|kind| *kind == "run.finished").count(), 1, "{kinds:?}");
    assert_eq!(kinds.iter().filter(|kind| *kind == "cancel").count(), 1, "{kinds:?}");
    let dir = format!("{}/state/agent-kit/model-gateway/runs/{id}", machine.sandbox.text());
    assert_eq!(cancel_reason(machine.runtime.files.as_ref(), &dir).as_deref(), Some("cancelled by the owner"));
    drop(events);
    // Cancelling it again finds it finished.
    let again = cancel(&input(&id), &machine.runtime).expect("again");
    assert_eq!((get(&again, "cancelled"), text(&again)), (&Value::Bool(false), format!("{id}: cancelled (the run had already finished)\n")));
    assert_eq!(log_kinds(&machine, &id).iter().filter(|kind| *kind == "run.finished").count(), 1);
}

#[test]
fn a_client_that_ignores_sigterm_is_killed_after_the_grace() {
    let (machine, id, rec) = running(Recorder::new(true));
    let events = start(&machine, &id, true);
    // The gateway is gone, so the cancel writes the finish itself, after the kill.
    rec.gateway_dead.store(true, Ordering::SeqCst);
    let answer = cancel_with(&input(&id), &machine.runtime, waits()).expect("a cancel");
    assert_eq!(rec.sent(), [(4242, 15), (4242, 9)]);
    assert_eq!((get(&answer, "finished"), get(&answer, "state")), (&Value::Bool(true), &Value::str("cancelled")));
    assert_eq!(text(&answer), format!("{id}: cancelled (the run's gateway was gone, so cancel wrote the finish)\n"));
    std::mem::forget(events);
}

#[test]
fn a_run_whose_gateway_is_gone_gets_exactly_one_finish_from_the_cancel_and_one_without_a_client_is_not_signalled() {
    let (machine, id, rec) = running(Recorder::new(false));
    let events = start(&machine, &id, false);
    rec.gateway_dead.store(true, Ordering::SeqCst);
    let answer = cancel_with(&input(&id), &machine.runtime, waits()).expect("a cancel");
    assert_eq!(get(&answer, "state"), &Value::str("cancelled"));
    assert!(rec.sent().is_empty() && rec.execs.lock().expect("execs").is_empty());
    let kinds = log_kinds(&machine, &id);
    assert_eq!(kinds.iter().filter(|kind| *kind == "run.finished").count(), 1, "{kinds:?}");
    // The finish says why, and the index says the run ended.
    let finish = std::fs::read_to_string(format!("{}/state/agent-kit/model-gateway/runs/{id}/events.jsonl", machine.sandbox.text())).expect("log");
    assert!(finish.lines().last().expect("a line").contains("the run's gateway was gone, so cancel wrote this finish"), "{finish}");
    let runs = crate::run_status::indexed_runs(&machine.runtime).expect("runs");
    assert_eq!(runs[0].finished.as_ref().map(|(status, _, _)| status.as_str()), Some("cancelled"));
    std::mem::forget(events);
}

#[test]
fn a_gateway_that_is_still_running_but_has_not_finished_yet_is_reported_as_such() {
    let (machine, id, _) = running(Recorder::new(false));
    let events = start(&machine, &id, true);
    let answer = cancel_with(&input(&id), &machine.runtime, CancelWaits { grace_ms: 50, finish_ms: 60, poll_ms: 5 }).expect("a cancel");
    assert_eq!((get(&answer, "cancelled"), get(&answer, "finished")), (&Value::Bool(true), &Value::Bool(false)));
    assert!(text(&answer).contains("(the run's gateway is still running and has not written the finish yet)"), "{}", text(&answer));
    // The owner's marker is there for the run's result to read.
    let dir = format!("{}/state/agent-kit/model-gateway/runs/{id}", machine.sandbox.text());
    assert_eq!(cancel_reason(machine.runtime.files.as_ref(), &dir).as_deref(), Some("cancelled by the owner"));
    drop(events);
}

#[test]
fn a_cancel_names_a_run_that_is_there() {
    let (machine, _, _) = running(Recorder::new(false));
    assert_eq!(cancel(&Object::new(), &machine.runtime).expect_err("no run").code, "invalid-input");
    assert_eq!(cancel(&input("nope"), &machine.runtime).expect_err("unknown").code, "unknown-run");
    // The default waits are the runner's own.
    let defaults = CancelWaits::default();
    assert_eq!((defaults.grace_ms, defaults.finish_ms), (crate::adapters::process::KILL_GRACE_MS, 15_000));
}

#[test]
fn a_finish_another_gateway_has_claimed_is_not_written_twice() {
    let (machine, id, rec) = running(Recorder::new(false));
    let events = start(&machine, &id, false);
    rec.gateway_dead.store(true, Ordering::SeqCst);
    // The lost-run recovery of another gateway claimed the log first.
    let dir = format!("{}/state/agent-kit/model-gateway/runs/{id}", machine.sandbox.text());
    std::fs::write(format!("{dir}/events.lost"), "").expect("claim");
    let answer = cancel_with(&input(&id), &machine.runtime, waits()).expect("a cancel");
    assert_eq!(get(&answer, "finished"), &Value::Bool(false));
    assert_eq!(text(&answer), format!("{id}: running (the run's gateway was gone and another gateway claimed the finish)\n"));
    assert!(!log_kinds(&machine, &id).iter().any(|kind| kind == "run.finished"));
    std::mem::forget(events);
}
