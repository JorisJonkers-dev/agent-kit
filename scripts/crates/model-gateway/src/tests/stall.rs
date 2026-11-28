//! Tests for the stall policy: a client that stops writing gets a `stall` event at the threshold and a cancel at
//! three times it, and the settings, the policy and the cancel marker behind that.

use super::scenarios::{FixedClock, Sandbox, EXEC_LOCK};
use crate::adapters::fs::SystemFiles;
use crate::adapters::process::SystemProcess;
use crate::json::{parse, Value};
use crate::ports::{Process, ProcessCommand};
use crate::run_events::{RunEvents, RunIdentity, RunStart};
use crate::stall::{cancel_reason, mark_cancelled, merge, stall_data, StallPolicy, StallSettings, StallStep, CANCEL_FILE, DEFAULT_THRESHOLD_S};
use std::collections::HashMap;

fn process() -> SystemProcess {
    SystemProcess { env: HashMap::from([("PATH".to_string(), std::env::var("PATH").unwrap_or_default())]) }
}

fn value(text: &str) -> Value {
    parse(text).expect("JSON")
}

#[test]
fn the_threshold_is_three_minutes_unless_the_settings_say_otherwise() {
    let base = StallSettings::default();
    assert_eq!(base.threshold_s, DEFAULT_THRESHOLD_S);
    assert_eq!(merge(&base, None), Ok(base.clone()));
    assert_eq!(merge(&base, Some(&value(r#"{}"#))), Ok(base.clone()));
    assert_eq!(merge(&base, Some(&value(r#"{"threshold_s": 45}"#))), Ok(StallSettings { threshold_s: 45.0 }));
    for (raw, message) in [
        (r#"[]"#, "settings.stall must be an object"),
        (r#"{"limit": 1}"#, "settings.stall.limit is not a setting: it has threshold_s"),
        (r#"{"threshold_s": 0}"#, "settings.stall.threshold_s must be a number of seconds above 0"),
        (r#"{"threshold_s": "soon"}"#, "settings.stall.threshold_s must be a number of seconds above 0"),
    ] {
        let error = merge(&base, Some(&value(raw))).expect_err(raw);
        assert_eq!((error.code.as_str(), error.message.as_str()), ("invalid-config", message), "{raw}");
    }
}

#[test]
fn a_silence_is_a_stall_at_the_threshold_and_a_cancel_at_three_times_it() {
    let policy = StallPolicy::from_settings(&StallSettings { threshold_s: 10.0 });
    assert_eq!((policy.threshold_ms, policy.cancel_ms()), (10_000, 30_000));
    assert_eq!(policy.step(9_999, false, false), StallStep::Quiet);
    assert_eq!(policy.step(10_000, false, false), StallStep::Warn);
    assert_eq!(policy.step(10_000, true, false), StallStep::Quiet);
    assert_eq!(policy.step(29_999, true, false), StallStep::Quiet);
    assert_eq!(policy.step(30_000, true, false), StallStep::Cancel);
    assert_eq!(policy.step(30_000, false, false), StallStep::Cancel);
    assert_eq!(policy.step(99_000, true, true), StallStep::Quiet);
    let data = stall_data(12, &policy);
    assert_eq!(crate::json::compact(&Value::Obj(data)), r#"{"silent_s":12,"threshold_s":10,"cancel_at_s":30}"#);
}

#[test]
fn a_cancel_leaves_one_marker_that_says_why() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let dir = sandbox.text();
    assert_eq!(cancel_reason(&files, &dir), None);
    assert!(mark_cancelled(&files, &dir, "cancelled by the owner"));
    assert!(!mark_cancelled(&files, &dir, "cancelled by the stall policy"));
    assert_eq!(cancel_reason(&files, &dir).as_deref(), Some("cancelled by the owner"));
    std::fs::write(sandbox.path.join(CANCEL_FILE), "").expect("emptied");
    assert_eq!(cancel_reason(&files, &dir).as_deref(), Some(""));
}

fn identity() -> RunIdentity {
    RunIdentity { run_id: "r1".into(), workflow: "w".into(), client: "pi".into(), model: "m".into(), family: "f".into() }
}

fn events_of(sandbox: &Sandbox) -> Vec<Value> {
    std::fs::read_to_string(sandbox.path.join("r1/events.jsonl")).expect("the log").lines().map(value).collect()
}

fn kinds(events: &[Value]) -> Vec<String> {
    events.iter().filter_map(|event| event.as_object()?.get("type")?.as_str().map(str::to_string)).collect()
}

/// A log of a run in a directory of the sandbox, with the stall policy watching it.
fn watched<'a>(sandbox: &Sandbox, files: &'a SystemFiles, clock: &'a FixedClock, process: &'a dyn Process, policy: StallPolicy) -> RunEvents<'a> {
    std::fs::create_dir_all(sandbox.path.join("r1")).expect("run dir");
    RunEvents::start(files, clock, &format!("{}/r1", sandbox.text()), identity(), &RunStart { mode: "read-only", time_limit_s: 900.0 }).with_stall(policy, process)
}

#[test]
fn a_fake_harness_that_stops_writing_is_stalled_at_the_threshold_and_cancelled_at_three_times_it() {
    let _guard = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(1_795_000_000_000);
    let process = process();
    let events = watched(&sandbox, &files, &clock, &process, StallPolicy { threshold_ms: 300, check_ms: 25, kill_grace_ms: 5_000 });
    let started = std::time::Instant::now();
    let launched = events.beating(|| process.run_watched(&ProcessCommand::new("sh", &["-c", "echo hello; exec sleep 30"]), &events)).expect("the harness runs");
    let took = started.elapsed();
    // Silent from the start, so the stall is at 300 ms and the cancel at 900 ms; the whole 30 s are never waited.
    assert!(took >= std::time::Duration::from_millis(850) && took < std::time::Duration::from_secs(10), "{took:?}");
    assert_eq!(launched.ending.signal, Some(15));
    events.finish(crate::json::object(vec![("status", Value::str("cancelled"))]));
    let log = events_of(&sandbox);
    let kinds = kinds(&log);
    let at = |name: &str| kinds.iter().position(|kind| kind == name).unwrap_or_else(|| panic!("no {name} in {kinds:?}"));
    assert!(at("stall") < at("cancel") && at("cancel") < at("run.finished"), "{kinds:?}");
    assert_eq!(kinds.iter().filter(|kind| *kind == "stall").count(), 1);
    assert_eq!(kinds.iter().filter(|kind| *kind == "cancel").count(), 1);
    let stall = log[at("stall")].as_object().and_then(|event| event.get("data")).and_then(Value::as_object).cloned().expect("data");
    assert_eq!(stall.get("threshold_s"), Some(&Value::Num(0.0)));
    let why = cancel_reason(&files, &format!("{}/r1", sandbox.text())).expect("the marker");
    assert!(why.starts_with("cancelled by the stall policy: the client wrote nothing for"), "{why}");
}

#[test]
fn a_client_that_writes_again_is_not_cancelled() {
    let _guard = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(1_795_000_000_000);
    let process = process();
    let events = watched(&sandbox, &files, &clock, &process, StallPolicy { threshold_ms: 400, check_ms: 25, kill_grace_ms: 5_000 });
    let launched = events.beating(|| process.run_watched(&ProcessCommand::new("sh", &["-c", "echo a; sleep 0.7; echo b; sleep 0.7; echo c"]), &events)).expect("runs");
    assert_eq!((launched.result.exit_code, launched.ending.signal), (0, None));
    events.finish(crate::json::object(vec![("status", Value::str("ok"))]));
    let kinds = kinds(&events_of(&sandbox));
    assert!(kinds.iter().filter(|kind| *kind == "stall").count() >= 2, "each silence is its own stall: {kinds:?}");
    assert!(!kinds.iter().any(|kind| kind == "cancel"), "{kinds:?}");
    assert!(cancel_reason(&files, &format!("{}/r1", sandbox.text())).is_none());
}

#[test]
fn a_cancelled_client_that_ignores_sigterm_is_killed_after_the_grace() {
    let _guard = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(1_795_000_000_000);
    let process = process();
    let events = watched(&sandbox, &files, &clock, &process, StallPolicy { threshold_ms: 100, check_ms: 25, kill_grace_ms: 300 });
    let stubborn = ProcessCommand::new("perl", &["-e", "$SIG{TERM} = 'IGNORE'; $| = 1; print qq(up\\n); sleep 30"]);
    let launched = events.beating(|| process.run_watched(&stubborn, &events)).expect("runs");
    assert_eq!(launched.ending.signal, Some(9));
    events.finish(crate::json::object(vec![("status", Value::str("cancelled"))]));
    let texts: Vec<String> = events_of(&sandbox)
        .iter()
        .filter_map(|event| event.as_object()?.get("data")?.as_object()?.get("text")?.as_str().map(str::to_string))
        .collect();
    assert!(texts.iter().any(|text| text.contains("ignored SIGTERM; sent SIGKILL")), "{texts:?}");
}

#[test]
fn a_cancel_the_owner_made_first_is_not_signalled_again_and_a_watch_with_no_client_waits() {
    let _guard = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(1_795_000_000_000);
    let process = process();
    let events = watched(&sandbox, &files, &clock, &process, StallPolicy { threshold_ms: 50, check_ms: 10, kill_grace_ms: 5_000 });
    // Nothing started yet: the watch looks at no silence.
    events.beating(|| std::thread::sleep(std::time::Duration::from_millis(120)));
    assert!(!kinds(&events_of(&sandbox)).iter().any(|kind| kind == "stall"));
    assert!(mark_cancelled(&files, &format!("{}/r1", sandbox.text()), "cancelled by the owner"));
    let launched = events.beating(|| process.run_watched(&ProcessCommand::new("sh", &["-c", "sleep 0.5"]), &events)).expect("runs");
    // The marker was the owner's, so the watch wrote its cancel event but sent no second signal: the shell ran out.
    assert_eq!((launched.result.exit_code, launched.ending.signal), (0, None));
    assert!(kinds(&events_of(&sandbox)).iter().any(|kind| kind == "cancel"));
}

#[test]
fn the_runner_signals_a_whole_process_group_and_says_whether_any_was_there() {
    use std::os::unix::process::CommandExt;
    let _guard = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut child = std::process::Command::new("sleep").arg("30").process_group(0).spawn().expect("sleep");
    let pid = child.id();
    let process = process();
    assert!(process.signal_group(pid, 0));
    assert!(process.signal_group(pid, 15));
    child.wait().expect("ended");
    assert!(!process.signal_group(pid, 0));
}
