//! A run's event log: `events.jsonl` in its run directory, one JSON object per line, written by the gateway
//! while the run goes, whatever client it launched.
//!
//! Every line carries `v` (the schema version, 1), `seq` (1 up, per run), `ts` (RFC 3339 with milliseconds),
//! `type`, `run_id`, `workflow`, the OpenTelemetry GenAI attributes that apply (`gen_ai.operation.name`,
//! `gen_ai.system`, `gen_ai.agent.name`, `gen_ai.request.model`) and `data`. The types written here are the
//! harness-agnostic ones: `run.started`, `diagnostic` (a stderr line or something the runner did), `heartbeat`
//! (every 15 seconds while the client runs) and `run.finished`, exactly once per run. A gateway that dies
//! before it writes the finish leaves it to the next gateway that starts a run beside it (`recover_lost`).
//!
//! The log never fails a run: a line that cannot be written is dropped, and `result.json` stays the record.
//! The client's raw output is appended to `output.json` and `stderr.txt` as it is read, and rewritten whole when
//! the client is done.

use crate::json::{self, Value};
use crate::ports::{Clock, Files, LaunchWatch, Process, Stream};
use crate::run_failure::{tail, TAIL_BYTES};
use crate::text::join;
use crate::timestamps::to_iso;
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The version of the event schema every line carries.
pub const EVENTS_VERSION: f64 = 1.0;

/// How often a running client's heartbeat is written.
pub const HEARTBEAT_MS: u64 = 15_000;

/// How many stderr lines become diagnostics; the rest are in `stderr.txt` only.
pub const STDERR_DIAGNOSTICS: usize = 200;

/// How long one diagnostic's text may be, in characters.
const DIAGNOSTIC_CHARS: usize = 1_000;

/// The file name of the log in a run directory.
pub const EVENTS_FILE: &str = "events.jsonl";

/// The run every line of one log names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunIdentity {
    pub run_id: String,
    pub workflow: String,
    /// The client (harness) that runs the model: `opencode`, `pi-vibe`.
    pub client: String,
    pub model: String,
    /// The model family, which OpenTelemetry calls the system.
    pub family: String,
}

/// What `run.started` says beyond the identity.
pub struct RunStart<'a> {
    pub mode: &'a str,
    pub time_limit_s: f64,
}

#[derive(Default)]
struct State {
    seq: u64,
    pid: Option<u32>,
    stdout_bytes: u64,
    stderr_bytes: u64,
    last_output: Option<Instant>,
    /// A stderr line not yet ended by a newline.
    partial: Vec<u8>,
    stderr_lines: usize,
    finished: bool,
}

/// One run's event log, from `run.started` to `run.finished`. Dropped without a finish (an error of the
/// gateway's own after the launch), it writes a failed one, so every log that starts ends exactly once.
pub struct RunEvents<'a> {
    files: &'a dyn Files,
    clock: &'a dyn Clock,
    run_dir: String,
    identity: RunIdentity,
    heartbeat_ms: u64,
    started: Instant,
    /// The clock's monotonic reading at the start, which the finish's duration is measured from.
    started_ms: f64,
    state: Mutex<State>,
}

/// The envelope every line shares, in its order.
fn envelope(identity: &RunIdentity, seq: u64, ts_ms: i64, kind: &str, data: Value) -> Value {
    json::object(vec![
        ("v", Value::Num(EVENTS_VERSION)),
        ("seq", Value::Num(seq as f64)),
        ("ts", Value::str(&to_iso(ts_ms))),
        ("type", Value::str(kind)),
        ("run_id", Value::str(&identity.run_id)),
        ("workflow", Value::str(&identity.workflow)),
        ("gen_ai.operation.name", Value::str("invoke_agent")),
        ("gen_ai.system", Value::str(&identity.family)),
        ("gen_ai.agent.name", Value::str(&identity.client)),
        ("gen_ai.request.model", Value::str(&identity.model)),
        ("data", data),
    ])
}

/// A number, or `null` when there is none.
fn number_or_null(number: Option<f64>) -> Value {
    number.map_or(Value::Null, Value::Num)
}

/// Text cut to `DIAGNOSTIC_CHARS` characters.
fn clipped(text: &str) -> String {
    text.chars().take(DIAGNOSTIC_CHARS).collect()
}

impl<'a> RunEvents<'a> {
    /// Opens the log and writes `run.started`, naming the gateway's own process id so a later gateway can tell
    /// whether this one is still there.
    pub fn start(files: &'a dyn Files, clock: &'a dyn Clock, run_dir: &str, identity: RunIdentity, start: &RunStart) -> RunEvents<'a> {
        let events = RunEvents { files, clock, run_dir: run_dir.to_string(), identity, heartbeat_ms: HEARTBEAT_MS, started: Instant::now(), started_ms: clock.monotonic_ms(), state: Mutex::new(State::default()) };
        events.emit(
            "run.started",
            json::object(vec![
                ("gateway_pid", Value::Num(f64::from(std::process::id()))),
                ("mode", Value::str(start.mode)),
                ("time_limit_s", Value::Num(start.time_limit_s)),
            ]),
        );
        events
    }

    /// The same log with heartbeats `heartbeat_ms` apart, for a test that cannot wait 15 seconds.
    pub fn with_heartbeat(mut self, heartbeat_ms: u64) -> RunEvents<'a> {
        self.heartbeat_ms = heartbeat_ms;
        self
    }

    fn path(&self, name: &str) -> String {
        join(&[&self.run_dir, name])
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, kind: &str, data: Value) {
        let mut state = self.lock();
        self.emit_locked(&mut state, kind, data);
    }

    /// Writes one line as one append, so a reader never sees half of it.
    fn emit_locked(&self, state: &mut State, kind: &str, data: Value) {
        state.seq += 1;
        let line = format!("{}\n", json::compact(&envelope(&self.identity, state.seq, self.clock.now_ms(), kind, data)));
        let _ = self.files.append(&self.path(EVENTS_FILE), &line);
    }

    /// A `diagnostic` line: `source` is `stderr` or `gateway`.
    pub fn diagnostic(&self, source: &str, level: &str, text: &str) {
        self.emit("diagnostic", json::object(vec![("level", Value::str(level)), ("source", Value::str(source)), ("text", Value::str(&clipped(text)))]));
    }

    /// A `heartbeat` line: the client's process id, how long the run has gone, how much it wrote, and how long
    /// ago it last wrote anything (`null` while it wrote nothing).
    pub fn heartbeat(&self) {
        let mut state = self.lock();
        let data = json::object(vec![
            ("pid", number_or_null(state.pid.map(f64::from))),
            ("alive_s", Value::Num(self.started.elapsed().as_secs() as f64)),
            ("stdout_bytes", Value::Num(state.stdout_bytes as f64)),
            ("stderr_bytes", Value::Num(state.stderr_bytes as f64)),
            ("last_output_age_s", number_or_null(state.last_output.map(|at| at.elapsed().as_secs() as f64))),
        ]);
        self.emit_locked(&mut state, "heartbeat", data);
    }

    /// Runs `work` with a heartbeat written every `heartbeat_ms` until it returns.
    pub fn beating<T>(&self, work: impl FnOnce() -> T) -> T {
        let stop = (Mutex::new(false), Condvar::new());
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let mut done = stop.0.lock().unwrap_or_else(PoisonError::into_inner);
                while !*done {
                    done = stop.1.wait_timeout(done, Duration::from_millis(self.heartbeat_ms)).unwrap_or_else(PoisonError::into_inner).0;
                    if !*done {
                        self.heartbeat();
                    }
                }
            });
            // Stopped on the way out, a panic in `work` included: the scope waits for the heartbeat thread.
            let _stop = Stopper(&stop);
            work()
        })
    }

    /// Each complete stderr line still held becomes a diagnostic, up to `STDERR_DIAGNOSTICS`; `flush` takes the
    /// unended last line too.
    fn stderr_lines(&self, state: &mut State, flush: bool) {
        let mut lines: Vec<Vec<u8>> = Vec::new();
        while let Some(end) = state.partial.iter().position(|byte| *byte == b'\n') {
            lines.push(state.partial.drain(..=end).collect());
        }
        if flush && !state.partial.is_empty() {
            lines.push(std::mem::take(&mut state.partial));
        }
        for line in lines {
            let text = String::from_utf8_lossy(&line).trim().to_string();
            if text.is_empty() {
                continue;
            }
            state.stderr_lines += 1;
            if state.stderr_lines <= STDERR_DIAGNOSTICS {
                let data = json::object(vec![("level", Value::str("warn")), ("source", Value::str("stderr")), ("text", Value::str(&clipped(&text)))]);
                self.emit_locked(state, "diagnostic", data);
            } else if state.stderr_lines == STDERR_DIAGNOSTICS + 1 {
                let data = json::object(vec![("level", Value::str("info")), ("source", Value::str("gateway")), ("text", Value::str("further stderr lines are in stderr.txt only"))]);
                self.emit_locked(state, "diagnostic", data);
            }
        }
    }

    /// Writes `run.finished` once; a second call writes nothing. `data` is what the result says of the run.
    pub fn finish(&self, data: Value) {
        let mut state = self.lock();
        if state.finished {
            return;
        }
        self.stderr_lines(&mut state, true);
        state.finished = true;
        let mut data = match data {
            Value::Obj(object) => object,
            _ => json::Object::new(),
        };
        data.set("pid", number_or_null(state.pid.map(f64::from)));
        data.set("duration_s", Value::Num(((self.clock.monotonic_ms() - self.started_ms) / 1000.0).round()));
        self.emit_locked(&mut state, "run.finished", Value::Obj(data));
    }
}

/// Ends the heartbeat thread of `beating` when dropped.
struct Stopper<'s>(&'s (Mutex<bool>, Condvar));

impl Drop for Stopper<'_> {
    fn drop(&mut self) {
        *self.0 .0.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.0 .1.notify_all();
    }
}

impl LaunchWatch for RunEvents<'_> {
    fn spawned(&self, pid: u32) {
        self.lock().pid = Some(pid);
    }

    fn output(&self, stream: Stream, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        let mut state = self.lock();
        state.last_output = Some(Instant::now());
        let text = String::from_utf8_lossy(chunk);
        match stream {
            Stream::Stdout => {
                state.stdout_bytes += chunk.len() as u64;
                let _ = self.files.append(&self.path("output.json"), &text);
            }
            Stream::Stderr => {
                state.stderr_bytes += chunk.len() as u64;
                let _ = self.files.append(&self.path("stderr.txt"), &text);
                state.partial.extend_from_slice(chunk);
                self.stderr_lines(&mut state, false);
            }
        }
    }

    fn note(&self, text: &str) {
        self.diagnostic("gateway", "warn", text);
    }
}

impl Drop for RunEvents<'_> {
    fn drop(&mut self) {
        self.finish(json::object(vec![
            ("status", Value::str("failed")),
            ("reason", Value::str("gateway-error")),
            ("failure", json::object(vec![("kind", Value::str("gateway-error")), ("reason", Value::str("the gateway stopped before it wrote the run's result"))])),
        ]));
    }
}

/// What `run.finished` says of a run whose result was written: its status, how its process ended, its
/// reason and failure, the end of its stderr, and what it used in OpenTelemetry's words.
pub fn finished_data(result: &crate::run_command::RunResult) -> Value {
    json::object(vec![
        ("status", Value::str(if result.ok { "ok" } else { "failed" })),
        ("exit_code", number_or_null(result.exit_code.map(f64::from))),
        ("signal", result.signal.map_or(Value::Null, |signal| Value::str(&crate::adapters::process::signal_name(signal)))),
        ("reason", result.reason.as_deref().map_or(Value::Null, Value::str)),
        ("failure", result.failure.as_ref().map_or(Value::Null, crate::run_failure::Failure::to_json)),
        ("stderr_tail", Value::str(&tail(result.stderr_tail.as_deref().unwrap_or_default(), TAIL_BYTES))),
        ("gen_ai.usage.input_tokens", Value::Num(result.tokens.input)),
        ("gen_ai.usage.output_tokens", Value::Num(result.tokens.output)),
        ("cost_usd", Value::Num(result.cost_usd)),
    ])
}

/// Finishes, as `lost`, every log under `runs_dir` that started and never finished and whose gateway is gone:
/// the gateway was killed before it could write the finish. The first gateway to claim a log (an `events.lost`
/// file made exclusively) writes it, so two gateways starting at once never write two.
pub fn recover_lost(files: &dyn Files, process: &dyn Process, clock: &dyn Clock, runs_dir: &str) {
    let Ok(names) = files.list_dir(runs_dir) else {
        return;
    };
    for name in names {
        let run_dir = join(&[runs_dir, &name]);
        let Ok(text) = files.read_text(&join(&[&run_dir, EVENTS_FILE])) else {
            continue;
        };
        let lines: Vec<Value> = text.lines().filter_map(|line| json::parse(line).ok()).collect();
        let kind = |line: &Value| line.as_object().and_then(|line| line.get("type")).and_then(Value::as_str).map(str::to_string);
        let Some(started) = lines.first().filter(|line| kind(line).as_deref() == Some("run.started")).and_then(Value::as_object) else {
            continue;
        };
        if lines.iter().any(|line| kind(line).as_deref() == Some("run.finished")) {
            continue;
        }
        let pid = started.get("data").and_then(Value::as_object).and_then(|data| data.get("gateway_pid")).and_then(|pid| match pid {
            Value::Num(pid) => Some(*pid as u32),
            _ => None,
        });
        let Some(pid) = pid.filter(|pid| *pid != std::process::id() && !process.is_alive(*pid)) else {
            continue;
        };
        if files.create_exclusive(&join(&[&run_dir, "events.lost"])) != Ok(true) {
            continue;
        }
        let text_of = |key: &str| started.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let identity = RunIdentity {
            run_id: text_of("run_id"),
            workflow: text_of("workflow"),
            client: text_of("gen_ai.agent.name"),
            model: text_of("gen_ai.request.model"),
            family: text_of("gen_ai.system"),
        };
        let reason = format!("the gateway (pid {pid}) ended before the run finished; this finish was written by a later gateway");
        let data = json::object(vec![
            ("status", Value::str("lost")),
            ("exit_code", Value::Null),
            ("signal", Value::Null),
            ("reason", Value::str("gateway-lost")),
            ("failure", json::object(vec![("kind", Value::str("gateway-error")), ("reason", Value::str(&reason))])),
        ]);
        let line = format!("{}\n", json::compact(&envelope(&identity, lines.len() as u64 + 1, clock.now_ms(), "run.finished", data)));
        let _ = files.append(&join(&[&run_dir, EVENTS_FILE]), &line);
    }
}
