//! Tests for the streaming runner, the run's event log and the failure a run's result names.

use super::scenarios::{FixedClock, Sandbox, EXEC_LOCK};
use crate::adapters::fs::SystemFiles;
use crate::adapters::process::{signal_name, SystemProcess};
use crate::json::{parse, Value};
use crate::ports::{Ending, Files, FsError, LaunchWatch, Launched, Process, ProcessCommand, ProcessResult, Stream, Unwatched};
use crate::run::{RunMode, RunTokens};
use crate::run_command::RunResult;
use crate::run_events::{finished_data, recover_lost, RunEvents, RunIdentity, RunStart, EVENTS_FILE, STDERR_DIAGNOSTICS};
use crate::run_failure::{failure_of, tail, Failure, FailureKind, OUTPUT_TAIL_BYTES};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

fn serialised() -> MutexGuard<'static, ()> {
    EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn process() -> SystemProcess {
    SystemProcess { env: HashMap::from([("PATH".to_string(), std::env::var("PATH").unwrap_or_default())]) }
}

fn shell(body: &str) -> ProcessCommand {
    ProcessCommand::new("sh", &["-c", body])
}

/// A watch that keeps everything it is told.
#[derive(Default)]
struct Recording {
    pid: Mutex<Option<u32>>,
    out: Mutex<Vec<u8>>,
    err: Mutex<Vec<u8>>,
    notes: Mutex<Vec<String>>,
}

impl LaunchWatch for Recording {
    fn spawned(&self, pid: u32) {
        *self.pid.lock().expect("pid") = Some(pid);
    }
    fn output(&self, stream: Stream, chunk: &[u8]) {
        match stream {
            Stream::Stdout => self.out.lock().expect("out").extend_from_slice(chunk),
            Stream::Stderr => self.err.lock().expect("err").extend_from_slice(chunk),
        }
    }
    fn note(&self, text: &str) {
        self.notes.lock().expect("notes").push(text.to_string());
    }
}

#[test]
fn the_runner_streams_each_output_to_the_watch_and_names_the_process_it_started() {
    let _guard = serialised();
    let watch = Recording::default();
    let launched = process().run_watched(&shell("echo out; echo err >&2; exit 3"), &watch).expect("the shell runs");
    assert_eq!((launched.result.stdout.as_str(), launched.result.stderr.as_str(), launched.result.exit_code), ("out\n", "err\n", 3));
    assert_eq!(launched.ending.signal, None);
    assert_eq!(*watch.pid.lock().expect("pid"), launched.ending.pid);
    assert_eq!((watch.out.lock().expect("out").as_slice(), watch.err.lock().expect("err").as_slice()), (&b"out\n"[..], &b"err\n"[..]));
}

#[test]
fn a_client_killed_by_a_signal_has_the_signal_and_no_exit_code_of_its_own() {
    let _guard = serialised();
    let launched = process().run_watched(&shell("echo dying >&2; kill -9 $$"), &Unwatched).expect("the shell runs");
    assert_eq!(launched.ending.signal, Some(9));
    assert_eq!((launched.result.exit_code, launched.result.timed_out, launched.result.stderr.as_str()), (124, false, "dying\n"));
}

#[test]
fn a_time_limit_stops_the_whole_process_group_and_says_what_it_did() {
    let _guard = serialised();
    let watch = Recording::default();
    let mut stubborn = ProcessCommand::new("perl", &["-e", "$SIG{TERM} = 'IGNORE'; sleep 30"]);
    // Long enough for perl to ignore SIGTERM before it comes.
    stubborn.timeout_ms = Some(300);
    let launched = process().watch_with_grace(&stubborn, 100, &watch).expect("the shell starts");
    assert!(launched.result.timed_out);
    assert_eq!(launched.ending.signal, Some(9));
    let notes = watch.notes.lock().expect("notes").join("\n");
    assert!(notes.contains("time limit of 300 ms; sent SIGTERM"), "{notes}");
    assert!(notes.contains("ignored SIGTERM for 100 ms; sent SIGKILL"), "{notes}");
}

#[test]
fn what_a_client_leaves_running_in_its_group_is_killed_so_its_pipes_close() {
    let _guard = serialised();
    let watch = Recording::default();
    let started = std::time::Instant::now();
    let launched = process().run_watched(&shell("sleep 30 & echo left"), &watch).expect("the shell runs");
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert_eq!(launched.result.stdout, "left\n");
    assert!(watch.notes.lock().expect("notes").iter().any(|note| note.contains("left running in its process group")));
}

#[test]
fn the_runner_knows_whether_a_process_is_still_there() {
    assert!(process().is_alive(std::process::id()));
    let mut child = std::process::Command::new("true").spawn().expect("true");
    let pid = child.id();
    child.wait().expect("reaped");
    assert!(!process().is_alive(pid));
}

#[test]
fn a_signal_has_its_name_or_its_number() {
    assert_eq!((signal_name(9).as_str(), signal_name(15).as_str(), signal_name(2).as_str(), signal_name(31).as_str()), ("SIGKILL", "SIGTERM", "SIGINT", "SIG31"));
}

/// A runner that cannot stream: the trait's own `run_watched` and `is_alive` stand in.
struct Buffered;

impl Process for Buffered {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }
    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Err("no".into())
    }
    fn run(&self, _command: &ProcessCommand) -> Result<ProcessResult, String> {
        Ok(ProcessResult { stdout: "o".into(), stderr: "e".into(), exit_code: 0, timed_out: false })
    }
}

#[test]
fn a_runner_that_cannot_stream_hands_over_the_whole_output_and_calls_every_process_alive() {
    assert!(Buffered.resolve("x").is_none() && Buffered.exec("x", &[], None).is_err());
    let watch = Recording::default();
    let Launched { result, ending } = Buffered.run_watched(&shell("x"), &watch).expect("ran");
    assert_eq!((result.stdout.as_str(), ending), ("o", Ending::default()));
    assert_eq!(watch.err.lock().expect("err").as_slice(), b"e");
    assert!(Buffered.is_alive(1));
    // A watch that wants nothing takes everything quietly.
    Unwatched.spawned(1);
    Unwatched.output(Stream::Stdout, b"x");
    Unwatched.note("n");
}

/// Files that can do nothing but what the trait gives every file system.
struct Bare;

impl Files for Bare {
    fn read_text(&self, _path: &str) -> Result<String, FsError> {
        Err(FsError { code: "ENOENT".into(), message: "no".into() })
    }
    fn write_atomic(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn mkdir_all(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn create_exclusive(&self, _path: &str) -> Result<bool, FsError> {
        Ok(true)
    }
    fn remove_file(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn modified_ms(&self, _path: &str) -> Option<i64> {
        None
    }
    fn exists(&self, _path: &str) -> Result<bool, FsError> {
        Ok(false)
    }
    fn append(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn stamp(&self, _path: &str) -> Option<crate::ports::FileStamp> {
        None
    }
    fn write_private(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn write_text(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn temp_dir(&self, _parent: &str, _prefix: &str) -> Result<String, FsError> {
        Ok(String::new())
    }
    fn remove(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }
}

#[test]
fn a_directory_is_listed_in_name_order_and_a_file_system_that_cannot_list_says_so() {
    let sandbox = Sandbox::new();
    std::fs::write(sandbox.path.join("b"), "").expect("b");
    std::fs::create_dir(sandbox.path.join("a")).expect("a");
    let files = SystemFiles { cwd: sandbox.text() };
    assert_eq!(files.list_dir(".").expect("listed"), vec!["a".to_string(), "b".to_string()]);
    assert_eq!(files.list_dir("missing").expect_err("missing").code, "ENOENT");
    assert_eq!(Bare.list_dir(".").expect_err("cannot").code, "ENOSYS");
    // Every other method of the bare file system answers too.
    assert!(Bare.read_text("x").is_err() && Bare.write_atomic("x", "").is_ok() && Bare.mkdir_all("x").is_ok() && Bare.create_exclusive("x") == Ok(true));
    assert!(Bare.remove_file("x").is_ok() && Bare.modified_ms("x").is_none() && Bare.exists("x") == Ok(false) && Bare.append("x", "").is_ok());
    assert!(Bare.stamp("x").is_none() && Bare.write_private("x", "").is_ok() && Bare.write_text("x", "").is_ok() && Bare.temp_dir("x", "y").is_ok() && Bare.remove("x").is_ok());
}

fn identity() -> RunIdentity {
    RunIdentity { run_id: "r1".into(), workflow: "w".into(), client: "opencode".into(), model: "opencode-go/kimi".into(), family: "moonshot".into() }
}

fn start() -> RunStart<'static> {
    RunStart { mode: "read-only", time_limit_s: 900.0 }
}

/// The log's lines, parsed.
fn lines(sandbox: &Sandbox, run: &str) -> Vec<Value> {
    std::fs::read_to_string(sandbox.path.join(run).join(EVENTS_FILE)).unwrap_or_default().lines().map(|line| parse(line).expect("a JSON line")).collect()
}

fn member<'a>(line: &'a Value, key: &str) -> &'a Value {
    line.as_object().and_then(|line| line.get(key)).unwrap_or(&Value::Null)
}

fn data<'a>(line: &'a Value, key: &str) -> &'a Value {
    member(member(line, "data"), key)
}

fn types(lines: &[Value]) -> Vec<String> {
    lines.iter().map(|line| member(line, "type").as_str().unwrap_or_default().to_string()).collect()
}

#[test]
fn a_log_starts_with_the_envelope_and_otel_names_and_finishes_once() {
    let sandbox = Sandbox::new();
    std::fs::create_dir(sandbox.path.join("r1")).expect("run dir");
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(1_795_000_000_000);
    // A client with no adapter, so its output is only kept (ADR-0020 covers the clients that have one).
    let events = RunEvents::start(&files, &clock, &format!("{}/r1", sandbox.text()), RunIdentity { client: "plain".into(), ..identity() }, &start());
    events.spawned(42);
    events.output(Stream::Stdout, b"{\"type\":\"text\"}\n");
    events.output(Stream::Stdout, b"");
    events.output(Stream::Stderr, b"first line\n\nsecond ");
    events.output(Stream::Stderr, b"half\nunended");
    events.note("sent SIGTERM");
    events.heartbeat();
    events.finish(Value::str("not an object"));
    events.finish(Value::Null);
    drop(events);
    let lines = lines(&sandbox, "r1");
    assert_eq!(types(&lines), ["run.started", "diagnostic", "diagnostic", "diagnostic", "heartbeat", "diagnostic", "run.finished"]);
    let started = &lines[0];
    let keys: Vec<&str> = started.as_object().expect("object").entries().into_iter().map(|(key, _)| key).collect();
    assert_eq!(keys, ["v", "seq", "ts", "type", "run_id", "workflow", "gen_ai.operation.name", "gen_ai.system", "gen_ai.agent.name", "gen_ai.request.model", "data"]);
    assert_eq!((member(started, "v"), member(started, "ts").as_str()), (&Value::Num(1.0), Some("2026-11-18T11:06:40.000Z")));
    assert_eq!((member(started, "gen_ai.system").as_str(), member(started, "gen_ai.request.model").as_str()), (Some("moonshot"), Some("opencode-go/kimi")));
    assert_eq!(data(started, "gateway_pid"), &Value::Num(f64::from(std::process::id())));
    let texts: Vec<&str> = lines.iter().filter_map(|line| data(line, "text").as_str()).collect();
    assert_eq!(texts, ["first line", "second half", "sent SIGTERM", "unended"]);
    assert_eq!(data(&lines[4], "pid"), &Value::Num(42.0));
    assert_eq!(data(&lines[4], "stderr_bytes"), &Value::Num(31.0));
    let seqs: Vec<&Value> = lines.iter().map(|line| member(line, "seq")).collect();
    assert_eq!(seqs.last(), Some(&&Value::Num(7.0)));
    assert_eq!((data(&lines[6], "pid"), data(&lines[6], "duration_s")), (&Value::Num(42.0), &Value::Num(0.0)));
    // The raw output went to its files as it came.
    assert_eq!(std::fs::read_to_string(sandbox.path.join("r1/output.json")).expect("output"), "{\"type\":\"text\"}\n");
    assert_eq!(std::fs::read_to_string(sandbox.path.join("r1/stderr.txt")).expect("stderr"), "first line\n\nsecond half\nunended");
}

#[test]
fn a_heartbeat_before_the_client_started_or_wrote_has_nulls() {
    let sandbox = Sandbox::new();
    std::fs::create_dir(sandbox.path.join("r1")).expect("run dir");
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(0);
    let events = RunEvents::start(&files, &clock, &format!("{}/r1", sandbox.text()), identity(), &start());
    events.heartbeat();
    drop(events);
    let lines = lines(&sandbox, "r1");
    assert_eq!((data(&lines[1], "pid"), data(&lines[1], "last_output_age_s")), (&Value::Null, &Value::Null));
    // Dropped without a finish, the log ends with the gateway's own failure.
    assert_eq!(data(&lines[2], "status").as_str(), Some("failed"));
    assert_eq!(member(data(&lines[2], "failure"), "kind").as_str(), Some("gateway-error"));
}

#[test]
fn heartbeats_are_written_while_the_work_runs_and_stop_when_it_returns() {
    let sandbox = Sandbox::new();
    std::fs::create_dir(sandbox.path.join("r1")).expect("run dir");
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(0);
    let events = RunEvents::start(&files, &clock, &format!("{}/r1", sandbox.text()), identity(), &start()).with_heartbeat(20);
    let value = events.beating(|| {
        // Until two heartbeats are in, however busy the machine is.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while types(&lines(&sandbox, "r1")).iter().filter(|kind| *kind == "heartbeat").count() < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        7
    });
    assert_eq!(value, 7);
    let beats = types(&lines(&sandbox, "r1")).iter().filter(|kind| *kind == "heartbeat").count();
    assert!(beats >= 2, "{beats}");
    std::thread::sleep(std::time::Duration::from_millis(60));
    assert_eq!(types(&lines(&sandbox, "r1")).iter().filter(|kind| *kind == "heartbeat").count(), beats);
}

#[test]
fn stderr_past_the_diagnostic_cap_is_said_once_and_left_to_the_file() {
    let sandbox = Sandbox::new();
    std::fs::create_dir(sandbox.path.join("r1")).expect("run dir");
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(0);
    let events = RunEvents::start(&files, &clock, &format!("{}/r1", sandbox.text()), identity(), &start());
    let many: String = (0..STDERR_DIAGNOSTICS + 5).map(|line| format!("line {line}\n")).collect();
    events.output(Stream::Stderr, many.as_bytes());
    events.finish(Value::Null);
    let lines = lines(&sandbox, "r1");
    let diagnostics = types(&lines).iter().filter(|kind| *kind == "diagnostic").count();
    assert_eq!(diagnostics, STDERR_DIAGNOSTICS + 1);
    assert_eq!(data(&lines[STDERR_DIAGNOSTICS + 1], "text").as_str(), Some("further stderr lines are in stderr.txt only"));
}

fn result(ok: bool) -> RunResult {
    RunResult {
        ok,
        run_id: "r".into(),
        workflow: "w".into(),
        client: "opencode".into(),
        model: "m".into(),
        family: "f".into(),
        mode: RunMode::ReadOnly,
        tokens: RunTokens { cached: 0.0, input: 10.0, output: 2.0 },
        cost_usd: 0.0,
        output_path: "/o".into(),
        summary: String::new(),
        reason: (!ok).then(|| "exit-1".into()),
        unavailable_until: None,
        write: None,
        attempt: None,
        attempt_of: None,
        exit_code: if ok { Some(0) } else { None },
        signal: if ok { None } else { Some(9) },
        stderr_tail: if ok { None } else { Some("boom".into()) },
        failure: (!ok).then(|| Failure { kind: FailureKind::Signalled, reason: "the client was ended by SIGKILL".into(), output_tail: None }),
        cleanup_warning: None,
        warnings: Vec::new(),
        fallback: None,
        retry: None,
        rewrite: None,
        routing: crate::routing::Routing::unseen("opencode"),
    }
}

#[test]
fn the_finish_says_what_the_result_says_in_otel_words() {
    let ok = finished_data(&result(true));
    assert_eq!((member(&ok, "status").as_str(), member(&ok, "exit_code"), member(&ok, "signal"), member(&ok, "failure")), (Some("ok"), &Value::Num(0.0), &Value::Null, &Value::Null));
    assert_eq!((member(&ok, "reason"), member(&ok, "stderr_tail").as_str()), (&Value::Null, Some("")));
    assert_eq!(member(&ok, "gen_ai.usage.input_tokens"), &Value::Num(10.0));
    let failed = finished_data(&result(false));
    assert_eq!((member(&failed, "exit_code"), member(&failed, "signal").as_str(), member(&failed, "reason").as_str()), (&Value::Null, Some("SIGKILL"), Some("exit-1")));
    assert_eq!(member(member(&failed, "failure"), "kind").as_str(), Some("signalled"));
}

/// A run directory holding `text` as its log.
fn run_with_log(sandbox: &Sandbox, name: &str, text: &str) {
    std::fs::create_dir_all(sandbox.path.join("runs").join(name)).expect("run dir");
    std::fs::write(sandbox.path.join("runs").join(name).join(EVENTS_FILE), text).expect("log");
}

fn started_by(pid: &str) -> String {
    format!("{{\"v\":1,\"seq\":1,\"type\":\"run.started\",\"run_id\":\"r\",\"workflow\":\"w\",\"gen_ai.system\":\"f\",\"gen_ai.agent.name\":\"c\",\"gen_ai.request.model\":\"m\",\"data\":{{\"gateway_pid\":{pid}}}}}\n")
}

/// A runner that says the processes it is asked about are alive or gone.
struct Alive(bool);

impl Process for Alive {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }
    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Err("no".into())
    }
    fn run(&self, _command: &ProcessCommand) -> Result<ProcessResult, String> {
        Err("no".into())
    }
    fn is_alive(&self, _pid: u32) -> bool {
        self.0
    }
}

#[test]
fn a_log_whose_gateway_is_gone_is_finished_as_lost_once_and_no_other_is_touched() {
    let sandbox = Sandbox::new();
    let gone = started_by("999999");
    run_with_log(&sandbox, "a-lost", &gone);
    run_with_log(&sandbox, "b-finished", &format!("{gone}{{\"type\":\"run.finished\"}}\n"));
    run_with_log(&sandbox, "c-mine", &started_by(&std::process::id().to_string()));
    run_with_log(&sandbox, "d-no-start", "{\"type\":\"heartbeat\"}\n");
    run_with_log(&sandbox, "e-no-pid", &started_by("\"x\""));
    run_with_log(&sandbox, "f-claimed", &gone);
    std::fs::write(sandbox.path.join("runs/f-claimed/events.lost"), "").expect("claim");
    std::fs::create_dir_all(sandbox.path.join("runs/g-no-log")).expect("no log");
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(0);
    assert!(Alive(false).resolve("x").is_none() && Alive(false).exec("x", &[], None).is_err() && Alive(false).run(&shell("x")).is_err());

    // A gateway that is still there keeps its log open.
    recover_lost(&files, &Alive(true), &clock, &format!("{}/runs", sandbox.text()));
    assert_eq!(types(&lines(&sandbox, "runs/a-lost")), ["run.started"]);

    recover_lost(&files, &Alive(false), &clock, &format!("{}/runs", sandbox.text()));
    recover_lost(&files, &Alive(false), &clock, &format!("{}/runs", sandbox.text()));
    let lost = lines(&sandbox, "runs/a-lost");
    assert_eq!(types(&lost), ["run.started", "run.finished"]);
    assert_eq!((data(&lost[1], "status").as_str(), data(&lost[1], "reason").as_str(), member(&lost[1], "seq")), (Some("lost"), Some("gateway-lost"), &Value::Num(2.0)));
    assert!(member(data(&lost[1], "failure"), "reason").as_str().unwrap_or_default().contains("pid 999999"));
    assert_eq!(member(&lost[1], "gen_ai.agent.name").as_str(), Some("c"));
    for untouched in ["c-mine", "d-no-start", "e-no-pid", "f-claimed"] {
        assert!(!types(&lines(&sandbox, &format!("runs/{untouched}"))).contains(&"run.finished".to_string()), "{untouched}");
    }
    assert_eq!(types(&lines(&sandbox, "runs/b-finished")).len(), 2);
    // A runs directory that cannot be listed is left alone.
    recover_lost(&Bare, &Alive(false), &clock, "runs");
}

#[test]
fn a_failure_names_what_explains_it_most() {
    let ran = |stdout: &str, stderr: &str, exit_code: i32, timed_out: bool| ProcessResult { stdout: stdout.into(), stderr: stderr.into(), exit_code, timed_out };
    let plain = Ending::default();
    let killed = Ending { signal: Some(9), ..Ending::default() };

    let spawn = failure_of("launch-failed", &ran("", "agent-launcher: not found\n", 1, false), &plain, None);
    assert_eq!((spawn.kind, spawn.reason.as_str()), (FailureKind::SpawnFailed, "agent-launcher: not found"));
    assert_eq!(failure_of("launch-failed", &ran("", "", 1, false), &plain, None).reason, "the client could not be started");

    let stopped = failure_of("exit-124", &ran("", "", 124, false), &Ending { cancelled_by: Some(2), signal: Some(15), pid: None }, None);
    assert_eq!((stopped.kind, stopped.reason.as_str()), (FailureKind::Signalled, "the gateway was stopped by SIGINT and stopped the client's process group"));

    let timed = failure_of("timeout", &ran("", "", 124, true), &killed, None);
    assert_eq!((timed.kind, timed.reason.as_str()), (FailureKind::TimedOut, "the client outlived its time limit and was stopped; ended by SIGKILL"));
    assert_eq!(failure_of("timeout", &ran("", "", 0, true), &plain, None).reason, "the client outlived its time limit and was stopped");

    let signalled = failure_of("exit-124", &ran("", "warming up\n", 124, false), &killed, None);
    assert_eq!((signalled.kind, signalled.reason.as_str()), (FailureKind::Signalled, "the client was ended by SIGKILL; stderr: warming up"));
    assert_eq!(failure_of("exit-124", &ran("", "", 124, false), &killed, None).reason, "the client was ended by SIGKILL");

    // A Cloudflare 524 from the model proxy is named as such, wherever the client reported it.
    let cut = failure_of("exit-1", &ran("", "Error: 524 A timeout occurred\n", 1, false), &plain, None);
    assert!(cut.reason.starts_with("Error: 524 A timeout occurred (the model proxy sits behind Cloudflare") && cut.reason.contains("stream: true"), "{}", cut.reason);
    let reported_cut = failure_of("exit-1", &ran("", "", 1, false), &plain, Some("the model proxy cut the request off after 120 seconds with a 524 timeout"));
    assert!(reported_cut.reason.contains("every LiteLLM client must stream"), "{}", reported_cut.reason);
    assert_eq!(failure_of("exit-1", &ran("", "HTTP 5240 ok\n", 1, false), &plain, None).reason, "HTTP 5240 ok");

    // The two runs that failed with nothing to say: an error on stdout, exit 1, an empty stderr.
    let stdout = "{\"type\":\"error\",\"error\":{\"message\":\"Cancelled a request for input this run cannot answer: Web Search\"}}\n";
    let reported = failure_of("exit-1", &ran(stdout, "", 1, false), &plain, Some("Cancelled a request for input this run cannot answer: Web Search unknown"));
    assert_eq!((reported.kind, reported.reason.as_str()), (FailureKind::HarnessError, "the client reported: Cancelled a request for input this run cannot answer: Web Search unknown"));
    assert_eq!(reported.output_tail.as_deref(), Some(stdout));

    let denied = failure_of("exit-1", &ran("", "cat: /x: Operation not permitted\nmore\n", 1, false), &plain, Some("  "));
    assert_eq!((denied.kind, denied.reason.as_str()), (FailureKind::SandboxDenied, "cat: /x: Operation not permitted (the sandbox's own denial log is not captured)"));
    assert_eq!(failure_of("exit-1", &ran("", "Sandbox: node(1) deny(1) file-read-data /x\n", 1, false), &plain, None).kind, FailureKind::SandboxDenied);

    let last = failure_of("exit-2", &ran("", "one\ntwo\n\n", 2, false), &plain, None);
    assert_eq!((last.kind, last.reason.as_str(), last.output_tail), (FailureKind::HarnessError, "two", None));

    let silent = failure_of("parse-error", &ran("", "", 0, false), &plain, None);
    assert_eq!(silent.reason, "parse-error: the client exited 0 with nothing on stderr and no error in its output");
    assert_eq!(silent.output_tail, None);
}

#[test]
fn a_failure_prints_its_kind_reason_and_output_tail_and_a_tail_cuts_at_a_character() {
    let with = Failure { kind: FailureKind::GatewayError, reason: "r".into(), output_tail: Some("o".into()) };
    assert_eq!(crate::json::compact(&with.to_json()), r#"{"kind":"gateway-error","reason":"r","output_tail":"o"}"#);
    let without = Failure { kind: FailureKind::SpawnFailed, reason: "r".into(), output_tail: None };
    assert_eq!(crate::json::compact(&without.to_json()), r#"{"kind":"spawn-failed","reason":"r"}"#);
    let kinds: Vec<&str> = [FailureKind::Signalled, FailureKind::TimedOut, FailureKind::SandboxDenied, FailureKind::HarnessError].iter().map(|kind| kind.as_str()).collect();
    assert_eq!(kinds, ["signalled", "timed-out", "sandbox-denied", "harness-error"]);
    assert_eq!(tail("héllo", 4), "llo");
    assert_eq!(tail("abc", 10), "abc");
    assert_eq!(tail(&"x".repeat(OUTPUT_TAIL_BYTES + 5), OUTPUT_TAIL_BYTES).len(), OUTPUT_TAIL_BYTES);
}

#[test]
fn a_panic_in_the_work_stops_the_heartbeat_and_reaches_the_caller() {
    let sandbox = Sandbox::new();
    std::fs::create_dir(sandbox.path.join("r1")).expect("run dir");
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(0);
    let events = RunEvents::start(&files, &clock, &format!("{}/r1", sandbox.text()), identity(), &start());
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| events.beating(|| panic!("the work broke"))));
    assert!(caught.is_err());
}

#[test]
fn two_launches_at_once_share_the_stop_handlers_the_first_put_in() {
    let _guard = serialised();
    let slow = Recording::default();
    std::thread::scope(|scope| {
        let first = scope.spawn(|| process().run_watched(&shell("sleep 1"), &slow).expect("the shell runs"));
        while slow.pid.lock().expect("pid").is_none() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Started and ended while the first still runs: it neither puts the handlers in nor takes them out.
        assert_eq!(process().run(&shell("true")).expect("the shell runs").exit_code, 0);
        assert_eq!(first.join().expect("joined").result.exit_code, 0);
    });
}

#[test]
fn a_stop_of_the_gateway_reaches_the_launch_and_refuses_the_next() {
    let _guard = serialised();
    let watch = Recording::default();
    // The first read (before the spawn) finds no stop; every later one finds SIGINT.
    let reads = std::sync::atomic::AtomicUsize::new(0);
    let stop = || if reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 { 0 } else { 2 };
    let launched = process().launch(&shell("exec sleep 30"), 5_000, &watch, &stop).expect("the shell starts");
    assert_eq!((launched.ending.cancelled_by, launched.ending.signal, launched.result.timed_out), (Some(2), Some(15), false));
    assert!(watch.notes.lock().expect("notes").iter().any(|note| note.contains("stopped by SIGINT; sent SIGTERM")));
    let refused = process().launch(&shell("true"), 5_000, &watch, &|| 1).expect_err("refused");
    assert_eq!(refused, "spawn sh refused: the gateway was stopped by SIGHUP");
}
