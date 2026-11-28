//! Tests for `ps`, `status`, `logs` and `gc`: they answer from the run index and the event logs, never from the
//! client's raw output, and for a batch of three while it runs and after it ends.

use super::machine::{now_ms, Machine, DEFAULTS};
use crate::batch_command::BatchCommand;
use crate::json::{parse, Object, Value};
use crate::ports::{Process, ProcessCommand, ProcessResult, Runtime};
use crate::run_command::RunCommand;
use crate::run_events::{RunEvents, RunIdentity, RunStart};
use crate::run_index::{IndexEntry, IndexKind, RunIndex};
use crate::run_status::{format_event, gc, gc_text, locate, logs, logs_text, ps, ps_table, status, status_text, summary_of, type_filter, view_of, RunView};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

const ANSWER: &str = r#"{"type":"result","result":"done\nSUMMARY: fine","usage":{"input_tokens":10,"output_tokens":5}}"#;

fn machine() -> Machine {
    let machine = Machine::new(DEFAULTS);
    machine.process.install("claude");
    machine.process.answer("claude", Ok(ProcessResult { stdout: ANSWER.to_string(), stderr: String::new(), exit_code: 0, timed_out: false }));
    machine
}

fn input(text: &str) -> Object {
    match parse(text).expect("JSON") {
        Value::Obj(object) => object,
        other => panic!("not an object: {other:?}"),
    }
}

fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).unwrap_or_else(|| panic!("no {key} in {value:?}"))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    get(value, key).as_str().unwrap_or_else(|| panic!("{key} is not text in {value:?}"))
}

fn list(value: &Value, key: &str) -> Vec<Value> {
    match get(value, key) {
        Value::Arr(items) => items.clone(),
        other => panic!("{key} is not a list: {other:?}"),
    }
}

fn plan(count: usize) -> String {
    let delegations: Vec<String> = (0..count)
        .map(|n| format!(r#"{{"id":"d{n}","client":"claude","model":"claude-sonnet-5-5","mode":"read-only","prompt":"TASK-{n}","escalate":false}}"#))
        .collect();
    format!(r#"{{"workflow":"w","delegations":[{}]}}"#, delegations.join(","))
}

fn run_batch(runtime: &Runtime, lines: &Mutex<Vec<String>>) -> Value {
    let runs = RunCommand { runtime, decision: None };
    let progress = |line: &str| lines.lock().expect("lines").push(line.to_string());
    BatchCommand { runtime, runs: &runs, progress: &progress }.run(&parse(&plan(3)).expect("JSON")).expect("a result")
}

/// A client that stays running until the test lets it go, so a batch can be looked at while it runs.
struct Held {
    inner: Box<dyn Process>,
    started: AtomicUsize,
    gate: (Mutex<bool>, Condvar),
}

impl Process for Held {
    fn resolve(&self, binary: &str) -> Option<String> {
        self.inner.resolve(binary)
    }
    fn exec(&self, command: &str, args: &[&str], timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.inner.exec(command, args, timeout_ms)
    }
    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let mut open = self.gate.0.lock().expect("gate");
        while !*open {
            open = self.gate.1.wait(open).expect("gate");
        }
        drop(open);
        self.inner.run(command)
    }
}

#[test]
fn ps_lists_a_three_run_batch_running_while_it_runs_and_ok_after() {
    let mut machine = machine();
    let held = Arc::new(Held { inner: Box::new(machine.process.clone()), started: AtomicUsize::new(0), gate: (Mutex::new(false), Condvar::new()) });
    struct Shared(Arc<Held>);
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
    }
    machine.runtime.process = Box::new(Shared(Arc::clone(&held)));
    let lines = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        let batch = scope.spawn(|| run_batch(&machine.runtime, &lines));
        while held.started.load(Ordering::SeqCst) < 3 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Every client has started, so each run's log says so; wait for the three index lines.
        let during = loop {
            let during = ps(&input("{}"), &machine.runtime).expect("ps");
            if list(&during, "runs").len() == 3 {
                break during;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let states: Vec<&str> = list(&during, "runs").iter().map(|run| text(run, "state").to_string()).map(|state| if state == "running" { "running" } else { "other" }).collect();
        assert_eq!(states, ["running", "running", "running"]);
        let table = ps_table(&during);
        assert!(table.starts_with("RUN"), "{table}");
        assert_eq!(table.lines().count(), 4);
        assert!(table.lines().skip(1).all(|line| line.contains("running") && line.contains("claude-sonnet-5-5")), "{table}");
        // `status` of one of them answers while it runs.
        let one = text(&list(&during, "runs")[0], "run_id").to_string();
        let mid = status(&input(&format!(r#"{{"run":"{one}"}}"#)), &machine.runtime).expect("status");
        assert_eq!(text(get(&mid, "run"), "state"), "running");
        *held.gate.0.lock().expect("gate") = true;
        held.gate.1.notify_all();
        let result = batch.join().expect("the batch");
        assert_eq!(text(&result, "status"), "ok");
    });
    let after = ps(&input("{}"), &machine.runtime).expect("ps");
    let states: Vec<String> = list(&after, "runs").iter().map(|run| text(run, "state").to_string()).collect();
    assert_eq!(states, ["ok", "ok", "ok"]);
    // The progress lines: each delegation queued, running and done, once each.
    let lines = lines.lock().expect("lines");
    for id in ["d0", "d1", "d2"] {
        let own: Vec<&str> = lines.iter().filter(|line| line.contains(&format!(" {id}: "))).map(String::as_str).collect();
        assert_eq!(own.len(), 3, "{own:?}");
        assert!(own[0].contains(": queued claude:claude-sonnet-5-5") && own[1].contains(": running ") && own[2].contains(": done "), "{own:?}");
    }
}

#[test]
fn status_answers_from_the_event_log_without_reading_the_clients_raw_output() {
    let machine = machine();
    run_batch(&machine.runtime, &Mutex::new(Vec::new()));
    let runs = list(&ps(&input("{}"), &machine.runtime).expect("ps"), "runs");
    let id = text(&runs[0], "run_id").to_string();
    let dir = text(&runs[0], "run_dir").to_string();
    // The raw output and stderr are gone; the log is all that is left to read.
    std::fs::remove_file(format!("{dir}/output.json")).expect("output.json");
    std::fs::remove_file(format!("{dir}/stderr.txt")).expect("stderr.txt");
    let answer = status(&input(&format!(r#"{{"run":"{}"}}"#, &id[..id.len() - 2])), &machine.runtime).expect("status by prefix");
    let run = get(&answer, "run");
    assert_eq!((text(run, "state"), text(run, "client"), text(run, "model"), text(run, "mode")), ("ok", "claude", "claude-sonnet-5-5", "read-only"));
    assert_eq!((get(run, "tokens_in"), get(run, "tokens_out")), (&Value::Num(10.0), &Value::Num(5.0)));
    assert_eq!(text(run, "last_event"), "run.finished");
    assert!(text(run, "step").starts_with("run.finished status=ok"), "{}", text(run, "step"));
    let lines = status_text(&answer);
    assert!(lines.contains("state: ok\n") && lines.contains("ended_at: ") && lines.contains("heartbeat_age_s: -\n"), "{lines}");
    // `logs` shows the events, all of them or the types asked for, as lines or as JSON.
    let all = logs(&input(&format!(r#"{{"run":"{id}"}}"#)), &machine.runtime).expect("logs");
    let kinds: Vec<String> = list(&all, "events").iter().map(|event| text(event, "type").to_string()).collect();
    assert_eq!(kinds.first().map(String::as_str), Some("run.started"));
    assert_eq!(kinds.last().map(String::as_str), Some("run.finished"));
    let only = logs(&input(&format!(r#"{{"run":"{id}","types":"run.finished, ,run.started"}}"#)), &machine.runtime).expect("logs");
    assert_eq!(list(&only, "events").len(), 2);
    assert!(logs_text(&only).lines().next().expect("a line").contains(&format!(" {id} run.started mode=read-only")));
    assert_eq!(logs_text(&Value::Null), "");
}

#[test]
fn a_run_is_found_by_its_id_or_the_start_of_it_and_a_bad_name_says_what_is_wrong() {
    let machine = machine();
    run_batch(&machine.runtime, &Mutex::new(Vec::new()));
    let ids: Vec<String> = list(&ps(&input("{}"), &machine.runtime).expect("ps"), "runs").iter().map(|run| text(run, "run_id").to_string()).collect();
    assert_eq!(locate(&machine.runtime, &ids[0]).expect("exact").run_id, ids[0]);
    let common = &ids[0][..4];
    let ambiguous = locate(&machine.runtime, common).expect_err("many runs start so");
    assert_eq!(ambiguous.code, "ambiguous-run");
    assert!(ambiguous.message.starts_with(&format!("{common} starts 3 runs: ")), "{}", ambiguous.message);
    assert_eq!(locate(&machine.runtime, "nope").expect_err("none").code, "unknown-run");
    assert_eq!(locate(&machine.runtime, "").expect_err("empty").code, "invalid-input");
    assert_eq!(status(&input("{}"), &machine.runtime).expect_err("no run").code, "invalid-input");
    assert_eq!(logs(&input(r#"{"run": 3}"#), &machine.runtime).expect_err("no run").code, "invalid-input");
}

/// A run's log and index lines, written the way a gateway writes them.
fn start_run<'a>(machine: &'a Machine, id: &str, workflow: &str) -> RunEvents<'a> {
    let dir = format!("{}/state/agent-kit/model-gateway/runs/{id}", machine.sandbox.text());
    std::fs::create_dir_all(&dir).expect("run dir");
    let identity = RunIdentity { run_id: id.into(), workflow: workflow.into(), client: "pi".into(), model: "free/hy4-flash".into(), family: "tencent".into() };
    let index = crate::locations::run_index_path(&machine.runtime.env).expect("index path");
    RunEvents::start(machine.runtime.files.as_ref(), machine.runtime.clock.as_ref(), &dir, identity, &RunStart { mode: "read-only", time_limit_s: 900.0 }).indexed(Some(&index))
}

#[test]
fn a_run_that_goes_quiet_is_stalled_until_it_writes_again() {
    use crate::ports::LaunchWatch;
    let machine = machine();
    let events = start_run(&machine, "r-stall", "w");
    events.spawned(4242);
    events.sandbox("mg-1-2");
    events.heartbeat();
    events.record("stall", parse(r#"{"silent_s": 30, "threshold_s": 10, "cancel_at_s": 30}"#).expect("JSON"));
    let view = |machine: &Machine| view_of(&machine.runtime, &crate::run_status::indexed_runs(&machine.runtime).expect("runs")[0], now_ms());
    // A stall with no heartbeat after it that shows output: the run is stalled.
    assert_eq!(view(&machine).state, "stalled");
    assert_eq!((view(&machine).client_pid, view(&machine).sandbox.as_deref()), (Some(4242), Some("mg-1-2")));
    // The latest heartbeat after the stall that still reports a long silence keeps it stalled.
    events.record("heartbeat", parse(r#"{"last_output_age_s": 45}"#).expect("JSON"));
    assert_eq!(view(&machine).state, "stalled");
    events.record("heartbeat", parse(r#"{"last_output_age_s": 1}"#).expect("JSON"));
    let after = view(&machine);
    assert_eq!(after.state, "running");
    assert!(!after.finished());
    let shown = after.to_json(now_ms());
    assert_eq!((text(&shown, "sandbox"), get(&shown, "client_pid")), ("mg-1-2", &Value::Num(4242.0)));
    // What the run used so far is the sum of its `usage` events, until the finish says it in full.
    events.record("usage", parse(r#"{"gen_ai.usage.input_tokens": 100, "gen_ai.usage.output_tokens": 10, "cost_usd": 0.5}"#).expect("JSON"));
    events.record("usage", parse(r#"{"gen_ai.usage.input_tokens": 50}"#).expect("JSON"));
    let used = view(&machine);
    assert_eq!((used.tokens_in, used.tokens_out, used.cost_usd), (150.0, 10.0, 0.5));
    events.finish(parse(r#"{"status":"ok"}"#).expect("JSON"));
    let done = view(&machine);
    assert_eq!((done.state.as_str(), done.tokens_in, done.cost_usd), ("ok", 150.0, 0.5));
}

#[test]
fn a_run_whose_gateway_is_gone_and_wrote_no_finish_is_lost_and_one_with_no_log_says_what_the_index_knows() {
    let mut machine = machine();
    let mut child = std::process::Command::new("true").spawn().expect("true");
    let dead = child.id();
    child.wait().expect("reaped");
    // The machine's fake runner calls every process alive; this one knows the one that ended.
    struct Mortal(u32, Box<dyn Process>);
    impl Process for Mortal {
        fn resolve(&self, binary: &str) -> Option<String> {
            self.1.resolve(binary)
        }
        fn exec(&self, command: &str, args: &[&str], timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
            self.1.exec(command, args, timeout_ms)
        }
        fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
            self.1.run(command)
        }
        fn is_alive(&self, pid: u32) -> bool {
            pid != self.0
        }
    }
    machine.runtime.process = Box::new(Mortal(dead, Box::new(machine.process.clone())));
    let runs_dir = format!("{}/state/agent-kit/model-gateway/runs", machine.sandbox.text());
    std::fs::create_dir_all(format!("{runs_dir}/r-lost")).expect("dir");
    let started = format!(r#"{{"v":1,"seq":1,"ts":"2026-11-04T09:00:00.000Z","type":"run.started","run_id":"r-lost","workflow":"w","data":{{"gateway_pid":{dead},"mode":"read-only","time_limit_s":900}}}}"#);
    std::fs::write(format!("{runs_dir}/r-lost/events.jsonl"), format!("{started}\nnot json\n")).expect("log");
    let index = RunIndex::new(machine.runtime.files.as_ref(), machine.runtime.clock.as_ref(), &crate::locations::run_index_path(&machine.runtime.env).expect("path"));
    let entry = |kind, id: &str, status: Option<&str>| IndexEntry {
        kind,
        ts_ms: now_ms() - 60_000,
        run_id: id.into(),
        run_dir: format!("{runs_dir}/{id}"),
        workflow: "w".into(),
        client: "pi".into(),
        model: "m".into(),
        family: "f".into(),
        mode: "read-only".into(),
        gateway_pid: Some(dead),
        status: status.map(str::to_string),
        reason: status.map(|_| "why".to_string()),
    };
    index.record(&entry(IndexKind::Started, "r-lost", None));
    // A finished run whose directory is gone, one unfinished whose directory is gone, and one that is still there with no log.
    index.record(&entry(IndexKind::Started, "r-gone-ok", None));
    index.record(&entry(IndexKind::Finished, "r-gone-ok", Some("failed")));
    index.record(&entry(IndexKind::Started, "r-gone", None));
    std::fs::create_dir_all(format!("{runs_dir}/r-empty")).expect("dir");
    let mut live = entry(IndexKind::Started, "r-empty", None);
    live.gateway_pid = Some(std::process::id());
    index.record(&live);
    let state = |id: &str| {
        let run = locate(&machine.runtime, id).expect("run");
        view_of(&machine.runtime, &run, now_ms())
    };
    let lost = state("r-lost");
    assert_eq!((lost.state.as_str(), lost.reason.as_deref(), lost.events), ("lost", Some("gateway-lost"), 1));
    let gone_ok = state("r-gone-ok");
    assert_eq!((gone_ok.state.as_str(), gone_ok.reason.as_deref(), gone_ok.events), ("failed", Some("why"), 0));
    assert_eq!(state("r-gone").state, "lost");
    assert_eq!(state("r-empty").state, "running");
}

#[test]
fn ps_narrows_by_workflow_and_state_and_limit_and_hides_what_is_old_unless_asked() {
    let machine = machine();
    let mut going = Vec::new();
    for (id, workflow) in [("r1", "a"), ("r2", "b"), ("r3", "a")] {
        let events = start_run(&machine, id, workflow);
        if id == "r3" {
            // Dropped, a log would finish itself as failed: this one stays running.
            going.push(events);
        } else {
            events.finish(parse(r#"{"status":"failed","reason":"x"}"#).expect("JSON"));
        }
    }
    // An old finished run, which only --all lists, and an old one still running, which is always listed.
    let index = RunIndex::new(machine.runtime.files.as_ref(), machine.runtime.clock.as_ref(), &crate::locations::run_index_path(&machine.runtime.env).expect("path"));
    for (id, finished) in [("r-old", true), ("r-old-running", false)] {
        std::fs::create_dir_all(format!("{}/{id}", machine.sandbox.text())).expect("run dir");
        let mut entry = IndexEntry { kind: IndexKind::Started, ts_ms: now_ms() - 3 * 86_400_000, run_id: id.into(), run_dir: format!("{}/{id}", machine.sandbox.text()), workflow: "a".into(), client: "pi".into(), model: "m".into(), family: "f".into(), mode: "read-only".into(), gateway_pid: Some(std::process::id()), status: None, reason: None };
        index.record(&entry);
        if finished {
            entry.kind = IndexKind::Finished;
            entry.status = Some("ok".into());
            index.record(&entry);
        }
    }
    let ids = |text: &str| -> Vec<String> { list(&ps(&input(text), &machine.runtime).expect("ps"), "runs").iter().map(|run| text_of(run, "run_id")).collect() };
    fn text_of(run: &Value, key: &str) -> String {
        text(run, key).to_string()
    }
    assert_eq!(ids("{}").len(), 4, "{:?}", ids("{}"));
    assert!(ids("{}").contains(&"r-old-running".to_string()) && !ids("{}").contains(&"r-old".to_string()));
    assert_eq!(ids(r#"{"all":true}"#).len(), 5);
    assert_eq!(ids(r#"{"workflow":"b"}"#), ["r2"]);
    assert_eq!(ids(r#"{"state":"running"}"#), ["r3", "r-old-running"]);
    assert_eq!(ids(r#"{"limit":1}"#).len(), 1);
    assert_eq!(ps(&input(r#"{"limit":"x"}"#), &machine.runtime).expect_err("a bad limit").code, "invalid-input");
    assert_eq!(ps(&input(r#"{"limit":-1}"#), &machine.runtime).expect_err("a bad limit").code, "invalid-input");
}

fn view_with(state: &str) -> RunView {
    RunView {
        run_id: "r".into(),
        run_dir: "d".into(),
        workflow: "w".into(),
        client: "c".into(),
        model: "m".into(),
        mode: "read-only".into(),
        state: state.into(),
        started_ms: 0,
        ended_ms: None,
        last_type: String::new(),
        last_ms: None,
        step: String::new(),
        heartbeat_age_s: None,
        silent_s: None,
        tokens_in: 0.0,
        tokens_out: 0.0,
        cost_usd: 0.0,
        reason: None,
        failure: None,
        gateway_pid: None,
        client_pid: None,
        sandbox: None,
        events: 0,
    }
}

#[test]
fn the_table_shows_ages_in_the_unit_that_fits_and_states_say_whether_a_run_ended() {
    let ages = [(5, "5s"), (150, "2m"), (7_300, "2h"), (200_000, "2d")];
    let runs: Vec<Value> = ages.iter().map(|(seconds, _)| view_with("running").to_json(seconds * 1000)).collect();
    let table = ps_table(&crate::json::object(vec![("runs", Value::Arr(runs))]));
    for (line, (_, shown)) in table.lines().skip(1).zip(ages) {
        assert!(line.split_whitespace().any(|word| word == shown), "{line} lacks {shown}");
    }
    assert_eq!(ps_table(&Value::Null), "RUN  STATE  CLIENT  MODEL  MODE  AGE  LAST\n");
    assert!(!view_with("stalled").finished() && view_with("cancelled").finished() && view_with("lost").finished());
    let mut ended = view_with("ok");
    ended.ended_ms = Some(9_000);
    assert_eq!(ended.age_s(1_000_000), 9);
    let shown = ended.to_json(0);
    assert_eq!(text(&shown, "ended_at"), "1970-01-01T00:00:09.000Z");
    assert_eq!(status_text(&Value::Null), "");
}

#[test]
fn every_kind_of_event_has_a_line() {
    let event = |kind: &str, data: &str| parse(&format!(r#"{{"ts":"2026-11-04T10:00:00.000Z","run_id":"r","type":"{kind}","data":{data}}}"#)).expect("JSON");
    let cases = [
        ("run.started", r#"{"mode":"write","time_limit_s":900}"#, "mode=write time_limit=900s"),
        ("run.finished", r#"{"status":"failed","reason":"x","exit_code":1,"failure":{"kind":"harness-error","reason":"boom"}}"#, "status=failed reason=x exit=1 [harness-error] boom"),
        ("run.finished", r#"{"status":"ok","reason":null,"exit_code":0}"#, "status=ok reason= exit=0"),
        ("heartbeat", r#"{"alive_s":15,"stdout_bytes":10,"stderr_bytes":2,"last_output_age_s":3}"#, "alive 15s, stdout 10B, stderr 2B, silent 3s"),
        ("diagnostic", r#"{"source":"stderr","text":"two\nlines"}"#, "[stderr] two lines"),
        ("stall", r#"{"silent_s":180,"threshold_s":180,"cancel_at_s":540}"#, "silent 180s of 180s, cancel at 540s"),
        ("cancel", r#"{"by":"owner","reason":"cancelled by the owner"}"#, "by owner: cancelled by the owner"),
        ("process.started", r#"{"pid":7}"#, "pid 7"),
        ("sandbox.started", r#"{"name":"mg-1"}"#, "sandbox mg-1"),
        ("message", r#"{"text":"hello"}"#, "hello"),
        ("tool.call", r#"{"name":"grep"}"#, "grep"),
        ("usage", r#"{"input":1}"#, r#"{"input":1}"#),
    ];
    for (kind, data, line) in cases {
        assert_eq!(summary_of(&event(kind, data)), line, "{kind}");
    }
    assert_eq!(format_event(&event("process.started", r#"{"pid":7}"#)), "2026-11-04T10:00:00.000Z r process.started pid 7");
    // An event with no data and one that is not an object still make a line.
    assert_eq!(format_event(&parse(r#"{"type":"x"}"#).expect("JSON")), "  x {}");
    assert_eq!(format_event(&Value::Null), "   {}");
    assert!(summary_of(&event("message", &format!(r#"{{"text":"{}"}}"#, "z".repeat(400)))).chars().count() == 160);
    assert!(type_filter(&input(r#"{"types":" a, b ,,"}"#)) == ["a", "b"] && type_filter(&input("{}")).is_empty());
}

fn old_run(machine: &Machine, id: &str, days_ago: i64, finished: bool, place: &str) -> String {
    let runs_dir = format!("{}/{place}/runs", machine.sandbox.text());
    let dir = format!("{runs_dir}/{id}");
    std::fs::create_dir_all(&dir).expect("run dir");
    std::fs::write(format!("{dir}/events.jsonl"), "").expect("log");
    let index = RunIndex::new(machine.runtime.files.as_ref(), machine.runtime.clock.as_ref(), &crate::locations::run_index_path(&machine.runtime.env).expect("path"));
    let mut entry = IndexEntry { kind: IndexKind::Started, ts_ms: now_ms() - days_ago * 86_400_000, run_id: id.into(), run_dir: dir.clone(), workflow: "w".into(), client: "pi".into(), model: "m".into(), family: "f".into(), mode: "read-only".into(), gateway_pid: Some(std::process::id()), status: None, reason: None };
    index.record(&entry);
    if finished {
        entry.kind = IndexKind::Finished;
        entry.status = Some("ok".into());
        index.record(&entry);
    }
    dir
}

#[test]
fn gc_removes_the_runs_older_than_thirty_days_and_nothing_else() {
    let machine = machine();
    let old = old_run(&machine, "r-old", 31, true, "state");
    let kept = old_run(&machine, "r-29", 29, true, "state");
    let running = old_run(&machine, "r-old-running", 40, false, "state");
    // A directory that is not `.../runs/<run id>` is never removed, whatever the index says of it.
    let odd = old_run(&machine, "r-odd", 40, true, "state");
    let odd_index = format!("{}/odd", machine.sandbox.text());
    std::fs::create_dir_all(&odd_index).expect("odd");
    RunIndex::new(machine.runtime.files.as_ref(), machine.runtime.clock.as_ref(), &crate::locations::run_index_path(&machine.runtime.env).expect("path")).record(&IndexEntry {
        kind: IndexKind::Started,
        ts_ms: now_ms() - 50 * 86_400_000,
        run_id: "r-elsewhere".into(),
        run_dir: odd_index.clone(),
        workflow: "w".into(),
        client: "pi".into(),
        model: "m".into(),
        family: "f".into(),
        mode: "read-only".into(),
        gateway_pid: Some(std::process::id()),
        status: None,
        reason: None,
    });
    std::fs::write(format!("{odd_index}/events.jsonl"), format!(r#"{{"ts":"2026-01-01T00:00:00.000Z","type":"run.finished","data":{{"status":"ok"}}}}{}"#, "\n")).expect("log");
    let dry = gc(&input(r#"{"dry_run":true}"#), &machine.runtime).expect("dry gc");
    assert_eq!((list(&dry, "removed").len(), list(&dry, "skipped").len()), (2, 1));
    assert!(std::path::Path::new(&old).exists(), "a dry run removes nothing");
    assert!(gc_text(&dry).starts_with("would remove 2 run directories and 0 rotated index files; 1 left alone"));
    let done = gc(&input("{}"), &machine.runtime).expect("gc");
    assert_eq!(list(&done, "removed"), vec![Value::str("r-old"), Value::str("r-odd")]);
    assert!(!std::path::Path::new(&old).exists() && !std::path::Path::new(&odd).exists());
    assert!(std::path::Path::new(&kept).exists() && std::path::Path::new(&running).exists() && std::path::Path::new(&odd_index).exists());
    assert_eq!(text(&list(&done, "skipped")[0], "run_id"), "r-elsewhere");
    assert!(gc_text(&done).starts_with("removed 2 run directories"));
    // The removal is in the index, so `ps --all` no longer lists them.
    let listed: Vec<String> = list(&ps(&input(r#"{"all":true}"#), &machine.runtime).expect("ps"), "runs").iter().map(|run| text(run, "run_id").to_string()).collect();
    assert!(!listed.contains(&"r-old".to_string()) && listed.contains(&"r-29".to_string()), "{listed:?}");
    // A shorter retention takes more; a bad one is refused.
    let tighter = gc(&input(r#"{"days":10}"#), &machine.runtime).expect("gc");
    assert_eq!(list(&tighter, "removed"), vec![Value::str("r-29")]);
    assert_eq!(gc(&input(r#"{"days":"x"}"#), &machine.runtime).expect_err("bad days").code, "invalid-input");
    assert_eq!(gc_text(&Value::Null), "removed 0 run directories and 0 rotated index files; 0 left alone\n");
}

#[test]
fn gc_drops_rotated_index_files_that_are_old_and_leaves_a_directory_it_cannot_remove() {
    let machine = machine();
    let state = format!("{}/state/agent-kit/model-gateway", machine.sandbox.text());
    std::fs::create_dir_all(&state).expect("state");
    let (old, fresh) = (format!("{state}/run-index.0000000000001-aaaaaa.jsonl"), format!("{state}/run-index.0000000000002-bbbbbb.jsonl"));
    std::fs::write(&old, "").expect("old");
    std::fs::write(&fresh, "").expect("fresh");
    let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(40 * 86_400);
    std::fs::File::options().write(true).open(&old).expect("open").set_modified(long_ago).expect("mtime");
    // The machine's clock is fixed in 2026, so the cutoff is that moment minus 30 days: both files are newer than it unless set to the clock.
    let clock_time = std::time::UNIX_EPOCH + std::time::Duration::from_millis((now_ms() - 40 * 86_400_000) as u64);
    std::fs::File::options().write(true).open(&old).expect("open").set_modified(clock_time).expect("mtime");
    let dry = gc(&input(r#"{"dry_run":true}"#), &machine.runtime).expect("dry gc");
    assert_eq!(list(&dry, "index_files_removed"), vec![Value::str(&old)]);
    assert!(std::path::Path::new(&old).exists(), "a dry run removes nothing");
    let swept = gc(&input("{}"), &machine.runtime).expect("gc");
    assert_eq!(list(&swept, "index_files_removed"), vec![Value::str(&old)]);
    assert!(!std::path::Path::new(&old).exists() && std::path::Path::new(&fresh).exists());
    assert!(gc_text(&swept).contains("and 1 rotated index files"));
}

/// A client that is cancelled while it runs: every run directory of the machine gets the owner's marker.
struct CancelledWhileRunning {
    inner: Box<dyn Process>,
    runs: String,
    started: AtomicUsize,
}

impl Process for CancelledWhileRunning {
    fn resolve(&self, binary: &str) -> Option<String> {
        self.inner.resolve(binary)
    }
    fn exec(&self, command: &str, args: &[&str], timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.inner.exec(command, args, timeout_ms)
    }
    fn run(&self, _command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.started.fetch_add(1, Ordering::SeqCst);
        for entry in std::fs::read_dir(&self.runs).expect("runs").flatten() {
            std::fs::write(entry.path().join("cancel.requested"), "cancelled by the owner\n").expect("marker");
        }
        // The client was ended by the signal the cancel sent.
        Ok(ProcessResult { stdout: String::new(), stderr: String::new(), exit_code: 124, timed_out: false })
    }
}

#[test]
fn a_run_that_was_cancelled_ends_as_cancelled_with_one_finish_and_is_not_retried() {
    let mut machine = machine();
    let runs = format!("{}/state/agent-kit/model-gateway/runs", machine.sandbox.text());
    std::fs::create_dir_all(&runs).expect("runs dir");
    let process = Arc::new(CancelledWhileRunning { inner: Box::new(machine.process.clone()), runs, started: AtomicUsize::new(0) });
    struct Shared(Arc<CancelledWhileRunning>);
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
    }
    machine.runtime.process = Box::new(Shared(Arc::clone(&process)));
    // `escalate` is on by default, and a cancelled run is still tried once.
    let result = RunCommand { runtime: &machine.runtime, decision: None }
        .run(&parse(r#"{"workflow":"w","client":"claude","model":"claude-sonnet-5-5","mode":"read-only","prompt":"x"}"#).expect("JSON"))
        .expect("a result");
    assert_eq!((text(&result, "status"), text(&result, "reason")), ("failed", "cancelled"));
    assert_eq!(text(get(&result, "failure"), "kind"), "cancelled");
    assert_eq!(text(get(&result, "failure"), "reason"), "cancelled by the owner");
    assert_eq!(process.started.load(Ordering::SeqCst), 1, "a cancelled run is not retried");
    let run = list(&ps(&input("{}"), &machine.runtime).expect("ps"), "runs").remove(0);
    assert_eq!(text(&run, "state"), "cancelled");
    let finishes = logs(&input(&format!(r#"{{"run":"{}","types":"run.finished"}}"#, text(&run, "run_id"))), &machine.runtime).expect("logs");
    assert_eq!(list(&finishes, "events").len(), 1);
}

/// Files whose removals fail, as a directory the owner may not delete from would.
struct Stubborn(crate::adapters::fs::SystemFiles);

macro_rules! delegate {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&self, $($arg: $ty),*) -> $ret { self.0.$name($($arg),*) })*
    };
}

impl crate::ports::Files for Stubborn {
    delegate! {
        read_text(path: &str) -> Result<String, crate::ports::FsError>;
        write_atomic(path: &str, content: &str) -> Result<(), crate::ports::FsError>;
        mkdir_all(path: &str) -> Result<(), crate::ports::FsError>;
        create_exclusive(path: &str) -> Result<bool, crate::ports::FsError>;
        remove_file(path: &str) -> Result<(), crate::ports::FsError>;
        modified_ms(path: &str) -> Option<i64>;
        exists(path: &str) -> Result<bool, crate::ports::FsError>;
        append(path: &str, content: &str) -> Result<(), crate::ports::FsError>;
        stamp(path: &str) -> Option<crate::ports::FileStamp>;
        write_private(path: &str, content: &str) -> Result<(), crate::ports::FsError>;
        write_text(path: &str, content: &str) -> Result<(), crate::ports::FsError>;
        temp_dir(parent: &str, prefix: &str) -> Result<String, crate::ports::FsError>;
        list_dir(path: &str) -> Result<Vec<String>, crate::ports::FsError>;
        size(path: &str) -> Option<u64>;
    }
    fn remove(&self, path: &str) -> Result<(), crate::ports::FsError> {
        Err(crate::ports::FsError { code: "EACCES".into(), message: format!("EACCES: permission denied, rm '{path}'") })
    }
}

#[test]
fn gc_leaves_a_run_directory_it_cannot_remove_and_says_so() {
    let mut machine = machine();
    let dir = old_run(&machine, "r-stuck", 45, true, "state");
    machine.runtime.files = Box::new(Stubborn(crate::adapters::fs::SystemFiles { cwd: machine.sandbox.text() }));
    let swept = gc(&input("{}"), &machine.runtime).expect("gc");
    assert!(list(&swept, "removed").is_empty());
    assert!(text(&list(&swept, "skipped")[0], "reason").starts_with("EACCES"), "{swept:?}");
    assert!(std::path::Path::new(&dir).exists());
}
