//! `session-start` and the `critique-session-start` hook (ADR-0002): in a repository opted in to the critique, one
//! line suggesting `/flow setup` while this machine's setup is missing or failing, by the results `/flow setup`
//! recorded; silence otherwise. It reads local files and git only and never launches a CLI. Like the gates it is inert
//! under the kill switch and outside an opted-in repository, and anything that keeps it from deciding is silent and
//! logged. The node runner's `application/session-start.ts`.
//!
//! The node runner races its decision against a 700 ms timer and answers when the timer wins. This runner decides
//! without a second thread, so a decision that took longer than the deadline is answered the same way, but only once
//! it is made.

use crate::error::{CritiqueError, ShapeError};
use crate::fields::Fields;
use crate::locations::{critique_config_path, gate_log_path, setup_results_path};
use crate::repository::{read_allowlist, resolve, Resolution};
use crate::routing::CriticClient;
use crate::routing_source::load_routing;
use crate::runtime::Runtime;
use crate::setup_checks::{parse_setup_results, setup_checklist};
use model_gateway::json::{self, Value};
use model_gateway::text::dirname;
use model_gateway::timestamps::to_iso;
use std::time::{Duration, Instant};

/// How long a gate or the session-start hook may take to decide before it fails open.
pub const GATE_DEADLINE_MS: u64 = 700;

fn silent(reason: &str) -> Value {
    json::object(vec![("line", Value::Null), ("reason", Value::str(reason)), ("status", Value::str("silent"))])
}

fn failed_open(failure: &str, detail: &str) -> Value {
    json::object(vec![
        ("detail", Value::str(detail)),
        ("failure", Value::str(failure)),
        ("line", Value::Null),
        ("reason", Value::str("failed-open")),
        ("status", Value::str("silent")),
    ])
}

fn suggest(reason: &str, line: String) -> Value {
    json::object(vec![("line", Value::Str(line)), ("reason", Value::str(reason)), ("status", Value::str("suggest"))])
}

/// Runs `session-start` on the hook's payload; it never fails, and what kept it from deciding is logged.
pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    Ok(decide_in_time(raw, runtime, Duration::from_millis(GATE_DEADLINE_MS)))
}

/// The decision, or the failed-open answer when it errs or takes longer than `deadline`.
pub(crate) fn decide_in_time(raw: &Value, runtime: &Runtime, deadline: Duration) -> Value {
    if runtime.env.get("CRITIQUE_OFF").map(String::as_str) == Some("1") {
        return silent("off");
    }
    let started = Instant::now();
    let decided = decide(raw, runtime);
    let result = if started.elapsed() > deadline {
        failed_open("deadline", &format!("the session-start hook did not decide within {GATE_DEADLINE_MS} ms"))
    } else {
        decided.unwrap_or_else(|error| failed_open("error", &error.message))
    };
    if result.as_object().and_then(|object| object.get("reason")).and_then(Value::as_str) == Some("failed-open") {
        log(raw, runtime, &result);
    }
    result
}

fn shape(error: ShapeError) -> CritiqueError {
    CritiqueError::unexpected(error.0)
}

fn decide(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let config_path = critique_config_path(&runtime.env)?;
    if runtime.files.exists(&config_path).unwrap_or(false) {
        let text = runtime.files.read_text(&config_path).map_err(CritiqueError::from_fs)?;
        let value = json::parse(&text).map_err(CritiqueError::unexpected)?;
        let file = Fields::of(&value, "").map_err(shape)?;
        if file.has("disabled") && file.boolean("disabled").map_err(shape)? {
            return Ok(silent("disabled"));
        }
    }
    let cwd = Fields::of(raw, "the session-start payload").and_then(|fields| fields.string("cwd")).map_err(shape)?;
    let repository = match resolve(runtime, &cwd)? {
        Resolution::Refused(refusal) => {
            let reason = refusal.as_object().and_then(|object| object.get("reason")).and_then(Value::as_str);
            return Ok(silent(if reason == Some("nedap") { "nedap" } else { "no-remote" }));
        }
        Resolution::Resolved(repository) => repository,
    };
    if read_allowlist(runtime)?.get(&repository.key).is_none() {
        return Ok(silent("not-opted-in"));
    }
    let checks = setup_check_ids(&load_routing(runtime)?.clients);
    let results_path = setup_results_path(&runtime.env)?;
    let results = if runtime.files.exists(&results_path).unwrap_or(false) {
        setup_results(&runtime.files.read_text(&results_path).map_err(CritiqueError::from_fs)?).map_err(shape)?
    } else {
        Vec::new()
    };
    let outcome = |id: &String| results.iter().find(|(check, _)| check == id).map(|(_, result)| result.as_str());
    let failing: Vec<&String> = checks.iter().filter(|id| outcome(id) == Some("fail")).collect();
    if let Some(first) = failing.first() {
        let more = if failing.len() > 1 { format!(", and {} more", failing.len() - 1) } else { String::new() };
        return Ok(suggest("setup-failing", format!("Critique: /flow setup's {first} check failed on this machine{more}. Run /flow setup.")));
    }
    let never = checks.iter().filter(|id| outcome(id).is_none()).count();
    if never == 0 {
        return Ok(silent("setup-current"));
    }
    Ok(suggest(
        "setup-missing",
        format!("Critique: /flow setup has not checked this machine yet ({never} of {} checks never ran). Run /flow setup.", checks.len()),
    ))
}

/// The ids of the setup checks that are not read-only proofs, in the order setup walks them: Vibe's sign-in and each
/// Vibe critic, Antigravity's sign-in, its critics and its quota, then the hooks and the kill switch. The work clients
/// have no setup check. The node runner's `setupChecklist` in `domain/setup.ts`, without its read-only proofs.
pub(crate) fn setup_check_ids(clients: &[(String, CriticClient)]) -> Vec<String> {
    setup_checklist(clients).into_iter().filter(|check| check.kind != "read-only").map(|check| check.id).collect()
}

/// Each check's last result in `/flow setup`'s results file, read as strictly as the node runner's
/// `parseSetupResults`: an entry with a field of the wrong shape makes the whole file unreadable.
pub(crate) fn setup_results(text: &str) -> Result<Vec<(String, String)>, ShapeError> {
    Ok(parse_setup_results(text)?.into_iter().map(|(id, entry)| (id, entry.result)).collect())
}

/// Appends one line to the gate log beside the gates' own; a log that cannot be written is left alone.
fn log(raw: &Value, runtime: &Runtime, result: &Value) {
    let field = |key: &str| result.as_object().and_then(|object| object.get(key)).cloned().unwrap_or(Value::Null);
    let cwd = raw.as_object().and_then(|object| object.get("cwd")).filter(|cwd| cwd.as_str().is_some()).cloned().unwrap_or(Value::Null);
    let line = json::object(vec![
        ("cwd", cwd),
        ("detail", field("detail")),
        ("failure", field("failure")),
        ("gate", Value::str("session-start")),
        ("recorded_at", Value::Str(to_iso(runtime.clock.now_ms()))),
    ]);
    if let Ok(path) = gate_log_path(&runtime.env) {
        // Nowhere to log; the session starts all the same.
        let _ = runtime.files.mkdir_all(dirname(&path)).and_then(|()| runtime.files.append(&path, &format!("{}\n", json::compact(&line))));
    }
}
