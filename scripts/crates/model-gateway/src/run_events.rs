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

use crate::event_adapters::{self, Adapter, AdapterEvent};
use crate::json::{self, Value};
use crate::ports::{Clock, Files, LaunchWatch, Process, Reply, Stream};
use crate::run_index::{IndexEntry, IndexKind, RunIndex};
use crate::stall::{self, StallPolicy, StallStep};
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
    /// The client's output read into events; `None` for a client with no adapter.
    adapter: Option<Box<dyn Adapter>>,
    /// A stdout line not yet ended by a newline.
    line: Vec<u8>,
    /// Whether an adapter wrote a `usage` event.
    usage_seen: bool,
    /// What the adapter has to say to the client's stdin, not yet taken by the runner.
    reply: Reply,
    seq: u64,
    pid: Option<u32>,
    stdout_bytes: u64,
    stderr_bytes: u64,
    last_output: Option<Instant>,
    /// A stderr line not yet ended by a newline.
    partial: Vec<u8>,
    stderr_lines: usize,
    finished: bool,
    /// Whether the stall was written for the silence now going, and whether the run was cancelled for it.
    stall_warned: bool,
    cancelled_at: Option<Instant>,
    killed: bool,
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
    mode: String,
    /// Where the global run index is, when the run is listed in it.
    index: Option<String>,
    /// The stall policy and the runner that can signal the client's group.
    stall: Option<(StallPolicy, &'a dyn Process)>,
}

/// The envelope every line shares, in its order.
pub(crate) fn envelope(identity: &RunIdentity, seq: u64, ts_ms: i64, kind: &str, data: Value) -> Value {
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
        let state = State { adapter: event_adapters::for_client(&identity.client), ..State::default() };
        let events = RunEvents { files, clock, run_dir: run_dir.to_string(), identity, heartbeat_ms: HEARTBEAT_MS, started: Instant::now(), started_ms: clock.monotonic_ms(), state: Mutex::new(state), mode: start.mode.to_string(), index: None, stall: None };
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

    /// The same log, listed in the global run index at `path` (none when the machine has no state directory to keep
    /// one in): its start is recorded now and its finish when it comes.
    pub fn indexed(mut self, path: Option<&str>) -> RunEvents<'a> {
        self.index = path.map(str::to_string);
        self.index_entry(IndexKind::Started, None);
        self
    }

    /// The same log with the stall policy watching its client: `process` signals the client's process group.
    pub fn with_stall(mut self, policy: StallPolicy, process: &'a dyn Process) -> RunEvents<'a> {
        self.stall = Some((policy, process));
        self
    }

    /// One line of the global index about this run, when it is listed in it.
    fn index_entry(&self, kind: IndexKind, ended: Option<(String, Option<String>)>) {
        let Some(path) = &self.index else {
            return;
        };
        let (status, reason) = ended.unzip();
        let entry = IndexEntry {
            kind,
            ts_ms: self.clock.now_ms(),
            run_id: self.identity.run_id.clone(),
            run_dir: self.run_dir.clone(),
            workflow: self.identity.workflow.clone(),
            client: self.identity.client.clone(),
            model: self.identity.model.clone(),
            family: self.identity.family.clone(),
            mode: self.mode.clone(),
            gateway_pid: (kind == IndexKind::Started).then(std::process::id),
            status,
            reason: reason.flatten(),
        };
        RunIndex::new(self.files, self.clock, path).record(&entry);
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

    /// A line of any other kind the run has to say, such as the route it was rewritten to or the fallback it is.
    pub fn record(&self, kind: &str, data: Value) {
        self.emit(kind, data);
    }

    /// Tells a client that is driven (ACP) what to ask it: the prompt, and the directory it works in.
    pub fn drive(&self, prompt: &str, cwd: &str) {
        if let Some(adapter) = self.lock().adapter.as_mut() {
            adapter.drive(prompt, cwd);
        }
    }

    /// Writes what an adapter read from the client's output as events. `flush` takes the unended last line too and
    /// asks the adapter for what it still holds, as the output has ended.
    fn adapter_lines(&self, state: &mut State, flush: bool) {
        let mut lines: Vec<Vec<u8>> = Vec::new();
        while let Some(end) = state.line.iter().position(|byte| *byte == b'\n') {
            lines.push(state.line.drain(..=end).collect());
        }
        if flush && !state.line.is_empty() {
            lines.push(std::mem::take(&mut state.line));
        }
        let Some(adapter) = state.adapter.as_mut() else {
            return;
        };
        let mut events: Vec<AdapterEvent> = Vec::new();
        for line in lines {
            let text = String::from_utf8_lossy(&line).trim().to_string();
            if !text.is_empty() {
                events.extend(adapter.line(&text));
            }
        }
        if flush {
            events.extend(adapter.finish());
        }
        if adapter.drives() {
            let out = adapter.outgoing();
            state.reply.text.push_str(&out.text);
            state.reply.close |= out.close;
        }
        for event in events {
            state.usage_seen |= event.kind == "usage";
            self.emit_locked(state, event.kind, event.data);
        }
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

    /// Runs `work` with a heartbeat written every `heartbeat_ms`, and the silence of the client looked at every
    /// `check_ms`, until it returns.
    pub fn beating<T>(&self, work: impl FnOnce() -> T) -> T {
        let stop = (Mutex::new(false), Condvar::new());
        let tick_ms = self.stall.map_or(self.heartbeat_ms, |(policy, _)| policy.check_ms.min(self.heartbeat_ms));
        std::thread::scope(|scope| {
            scope.spawn(|| self.keep_beating(&stop, tick_ms));
            // Stopped on the way out, a panic in `work` included: the scope waits for the heartbeat thread.
            let _stop = Stopper(&stop);
            work()
        })
    }

    /// The heartbeat thread's loop: a heartbeat every `heartbeat_ms` and a look at the silence every `tick_ms`, until
    /// `stop` is raised. It is not generic, so `beating` of every result type runs this one copy.
    fn keep_beating(&self, stop: &(Mutex<bool>, Condvar), tick_ms: u64) {
        let mut done = stop.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut last_beat = Instant::now();
        while !*done {
            done = stop.1.wait_timeout(done, Duration::from_millis(tick_ms)).unwrap_or_else(PoisonError::into_inner).0;
            if *done {
                break;
            }
            if last_beat.elapsed() >= Duration::from_millis(self.heartbeat_ms) {
                last_beat = Instant::now();
                self.heartbeat();
            }
            self.watch_silence();
        }
    }

    /// The stall policy's look at the client: a `stall` event once it has been silent for the threshold, a
    /// cancel at three times that (SIGTERM to its group, SIGKILL after the grace), and a fresh start of the
    /// count when it writes again.
    fn watch_silence(&self) {
        let Some((policy, process)) = self.stall else {
            return;
        };
        let mut state = self.lock();
        let Some(pid) = state.pid else {
            return;
        };
        if let Some(at) = state.cancelled_at {
            if !state.killed && at.elapsed() >= Duration::from_millis(policy.kill_grace_ms) {
                state.killed = true;
                process.signal_group(pid, 9);
                self.emit_locked(&mut state, "diagnostic", json::object(vec![("level", Value::str("warn")), ("source", Value::str("gateway")), ("text", Value::str("the cancelled client ignored SIGTERM; sent SIGKILL to its process group"))]));
            }
            return;
        }
        let silent = state.last_output.unwrap_or(self.started).elapsed();
        let silent_ms = silent.as_millis() as u64;
        if silent_ms < policy.threshold_ms {
            state.stall_warned = false;
        }
        match policy.step(silent_ms, state.stall_warned, false) {
            StallStep::Quiet => {}
            StallStep::Warn => {
                state.stall_warned = true;
                self.emit_locked(&mut state, "stall", Value::Obj(stall::stall_data(silent_ms / 1000, &policy)));
            }
            StallStep::Cancel => {
                let why = format!("cancelled by the stall policy: the client wrote nothing for {} s (threshold {} s, cancel at {} s)", silent_ms / 1000, policy.threshold_ms / 1000, policy.cancel_ms() / 1000);
                // A cancel the owner made first already left its marker and its signal.
                if stall::mark_cancelled(self.files, &self.run_dir, &why) {
                    process.signal_group(pid, 15);
                }
                state.cancelled_at = Some(Instant::now());
                let data = json::object(vec![("by", Value::str("stall")), ("reason", Value::str(&why))]);
                self.emit_locked(&mut state, "cancel", data);
            }
        }
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
        self.adapter_lines(&mut state, true);
        state.finished = true;
        let mut data = match data {
            Value::Obj(object) => object,
            _ => json::Object::new(),
        };
        // A harness that reports no usage as it goes (Vibe, which writes it to an export) has the result's.
        let counted = |key: &str| match data.get(key) {
            Some(Value::Num(number)) => *number,
            _ => 0.0,
        };
        let (input, output, cached, cost) = (counted("gen_ai.usage.input_tokens"), counted("gen_ai.usage.output_tokens"), counted("cached_input_tokens"), counted("cost_usd"));
        if state.adapter.is_some() && !state.usage_seen && input + output + cost > 0.0 {
            let usage = event_adapters::usage_of_result(input, output, cached, cost);
            self.emit_locked(&mut state, usage.kind, usage.data);
        }
        data.set("pid", number_or_null(state.pid.map(f64::from)));
        data.set("duration_s", Value::Num(((self.clock.monotonic_ms() - self.started_ms) / 1000.0).round()));
        let ended = (data.get("status").and_then(Value::as_str).unwrap_or("failed").to_string(), data.get("reason").and_then(Value::as_str).map(str::to_string));
        self.emit_locked(&mut state, "run.finished", Value::Obj(data));
        self.index_entry(IndexKind::Finished, Some(ended));
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
        let mut state = self.lock();
        state.pid = Some(pid);
        self.emit_locked(&mut state, "process.started", json::object(vec![("pid", Value::Num(f64::from(pid)))]));
    }

    fn sandbox(&self, name: &str) {
        self.emit("sandbox.started", json::object(vec![("name", Value::str(name))]));
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
                state.line.extend_from_slice(chunk);
                self.adapter_lines(&mut state, false);
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

    fn interactive(&self) -> bool {
        self.lock().adapter.as_ref().is_some_and(|adapter| adapter.drives())
    }

    fn opening(&self) -> String {
        self.lock().adapter.as_mut().map(|adapter| adapter.opening()).unwrap_or_default()
    }

    fn reply(&self, _chunk: &[u8]) -> Reply {
        std::mem::take(&mut self.lock().reply)
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
        ("status", Value::str(if result.ok { "ok" } else if result.reason.as_deref() == Some("cancelled") { "cancelled" } else { "failed" })),
        ("exit_code", number_or_null(result.exit_code.map(f64::from))),
        ("signal", result.signal.map_or(Value::Null, |signal| Value::str(&crate::adapters::process::signal_name(signal)))),
        ("reason", result.reason.as_deref().map_or(Value::Null, Value::str)),
        ("failure", result.failure.as_ref().map_or(Value::Null, crate::run_failure::Failure::to_json)),
        ("stderr_tail", Value::str(&tail(result.stderr_tail.as_deref().unwrap_or_default(), TAIL_BYTES))),
        ("gen_ai.usage.input_tokens", Value::Num(result.tokens.input)),
        ("gen_ai.usage.output_tokens", Value::Num(result.tokens.output)),
        ("cached_input_tokens", Value::Num(result.tokens.cached)),
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
