//! `cancel <run>`: ends a run and everything it started. The client's whole process group gets SIGTERM, and
//! SIGKILL after the grace; the run's sandbox, when it has one, is removed with `sbx rm --force`. The run's own
//! gateway then writes the one `run.finished`, as `cancelled`. Only when that gateway is gone does `cancel`
//! write the finish itself, so a log never holds two.

use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::ports::Runtime;
use crate::run_events::{envelope, RunIdentity, EVENTS_FILE};
use crate::run_index::{IndexEntry, IndexKind, RunIndex};
use crate::run_status::{locate, read_events, view_of, RunView};
use crate::sbx_launch::SBX;
use crate::stall::mark_cancelled;
use crate::text::join;
use std::time::{Duration, Instant};

const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;

/// How long the owning gateway has to write the finish after the client is gone.
pub const FINISH_WAIT_MS: u64 = 15_000;

/// How long the client's group has to exit after SIGTERM.
pub const GRACE_MS: u64 = crate::adapters::process::KILL_GRACE_MS;

/// How long `sbx rm` may take.
const SBX_TIMEOUT_MS: u64 = 60_000;

/// What `cancel` waits for, which a test shortens.
#[derive(Clone, Copy, Debug)]
pub struct CancelWaits {
    pub grace_ms: u64,
    pub finish_ms: u64,
    pub poll_ms: u64,
}

impl Default for CancelWaits {
    fn default() -> Self {
        CancelWaits { grace_ms: GRACE_MS, finish_ms: FINISH_WAIT_MS, poll_ms: 50 }
    }
}

pub fn cancel(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    cancel_with(input, runtime, CancelWaits::default())
}

/// Cancels the run `input.run` names.
pub fn cancel_with(input: &Object, runtime: &Runtime, waits: CancelWaits) -> Result<Value, GatewayError> {
    let key = match input.get("run") {
        Some(Value::Str(run)) => run.clone(),
        _ => return Err(GatewayError::invalid_input("name the run: its id, or the start of it")),
    };
    let run = locate(runtime, &key)?;
    let now = runtime.clock.now_ms();
    let view = view_of(runtime, &run, now);
    if view.finished() && view.state != "lost" {
        return Ok(answer(&view, false, true, "the run had already finished"));
    }
    mark_cancelled(runtime.files.as_ref(), &run.run_dir, "cancelled by the owner");
    append_event(runtime, &run.run_dir, &view, "cancel", json::object(vec![("by", Value::str("owner")), ("reason", Value::str("cancelled by the owner"))]));
    let process = runtime.process.as_ref();
    if let Some(pid) = view.client_pid {
        process.signal_group(pid, SIGTERM);
    }
    if let Some(name) = &view.sandbox {
        // The microVM goes with whatever still runs in it; a sandbox the gateway already removed is no failure.
        let _ = process.exec(SBX, &["rm", "--force", name], Some(SBX_TIMEOUT_MS));
    }
    if let Some(pid) = view.client_pid {
        let started = Instant::now();
        while process.signal_group(pid, 0) && started.elapsed() < Duration::from_millis(waits.grace_ms) {
            std::thread::sleep(Duration::from_millis(waits.poll_ms));
        }
        if process.signal_group(pid, 0) {
            process.signal_group(pid, SIGKILL);
        }
    }
    let started = Instant::now();
    loop {
        let now = runtime.clock.now_ms();
        let latest = view_of(runtime, &run, now);
        if latest.finished() && latest.state != "lost" {
            return Ok(answer(&latest, true, true, "the run's own gateway wrote its finish"));
        }
        let gateway_gone = latest.gateway_pid.is_some_and(|pid| !process.is_alive(pid));
        if gateway_gone || started.elapsed() >= Duration::from_millis(waits.finish_ms) {
            break;
        }
        std::thread::sleep(Duration::from_millis(waits.poll_ms));
    }
    let latest = view_of(runtime, &run, runtime.clock.now_ms());
    let gateway_gone = latest.gateway_pid.is_some_and(|pid| !process.is_alive(pid));
    if !gateway_gone {
        return Ok(answer(&latest, true, false, "the run's gateway is still running and has not written the finish yet"));
    }
    // Its gateway is gone, so nobody else writes the finish. The claim is the one the lost-run recovery makes.
    let claimed = runtime.files.create_exclusive(&join(&[&run.run_dir, "events.lost"])) == Ok(true);
    if claimed {
        write_finish(runtime, &run.run_dir, &latest);
    }
    let finished = view_of(runtime, &run, runtime.clock.now_ms());
    let note = if claimed { "the run's gateway was gone, so cancel wrote the finish" } else { "the run's gateway was gone and another gateway claimed the finish" };
    Ok(answer(&finished, true, finished.finished() && finished.state != "lost", note))
}

fn answer(view: &RunView, cancelled: bool, finished: bool, note: &str) -> Value {
    json::object(vec![
        ("status", Value::str("ok")),
        ("run_id", Value::str(&view.run_id)),
        ("cancelled", Value::Bool(cancelled)),
        ("finished", Value::Bool(finished)),
        ("state", Value::str(&view.state)),
        ("note", Value::str(note)),
    ])
}

fn identity_of(view: &RunView, family: &str) -> RunIdentity {
    RunIdentity { run_id: view.run_id.clone(), workflow: view.workflow.clone(), client: view.client.clone(), model: view.model.clone(), family: family.to_string() }
}

/// The family of the run, which its first event names.
fn family_of(runtime: &Runtime, run_dir: &str) -> String {
    read_events(runtime, run_dir)
        .first()
        .and_then(Value::as_object)
        .and_then(|first| first.get("gen_ai.system"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// One line appended to the run's log by a process that is not the run's own.
fn append_event(runtime: &Runtime, run_dir: &str, view: &RunView, kind: &str, data: Value) {
    let seq = read_events(runtime, run_dir).len() as u64 + 1;
    let line = format!("{}\n", json::compact(&envelope(&identity_of(view, &family_of(runtime, run_dir)), seq, runtime.clock.now_ms(), kind, data)));
    let _ = runtime.files.append(&join(&[run_dir, EVENTS_FILE]), &line);
}

/// The finish of a cancelled run whose gateway is gone, in the log and the index.
fn write_finish(runtime: &Runtime, run_dir: &str, view: &RunView) {
    let why = crate::stall::cancel_reason(runtime.files.as_ref(), run_dir).unwrap_or_else(|| "cancelled".to_string());
    let data = json::object(vec![
        ("status", Value::str("cancelled")),
        ("exit_code", Value::Null),
        ("signal", Value::Null),
        ("reason", Value::str("cancelled")),
        ("failure", json::object(vec![("kind", Value::str("cancelled")), ("reason", Value::str(&format!("{why}; the run's gateway was gone, so cancel wrote this finish")))])),
    ]);
    append_event(runtime, run_dir, view, "run.finished", data);
    crate::locations::run_index_path(&runtime.env).iter().for_each(|path| {
        RunIndex::new(runtime.files.as_ref(), runtime.clock.as_ref(), path).record(&IndexEntry {
            kind: IndexKind::Finished,
            ts_ms: runtime.clock.now_ms(),
            run_id: view.run_id.clone(),
            run_dir: run_dir.to_string(),
            workflow: view.workflow.clone(),
            client: view.client.clone(),
            model: view.model.clone(),
            family: String::new(),
            mode: view.mode.clone(),
            gateway_pid: None,
            status: Some("cancelled".to_string()),
            reason: Some("cancelled".to_string()),
        });
    });
}


/// The answer as a sentence.
pub fn text(result: &Value) -> String {
    let object = result.as_object().cloned().unwrap_or_default();
    let get = |key: &str| object.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    format!("{}: {} ({})\n", get("run_id"), get("state"), get("note"))
}
