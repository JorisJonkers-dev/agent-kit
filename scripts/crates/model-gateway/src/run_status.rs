//! What a run is doing, answered from its event log (ADR-0017) and the global run index: `ps` lists runs,
//! `status` summarises one, `logs` shows its events, `gc` applies the retention. None of them reads the client's
//! raw output.

use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::ports::Runtime;
use crate::run_events::EVENTS_FILE;
use crate::run_index::{fold, IndexEntry, IndexKind, IndexedRun, RunIndex};
use crate::text::{basename, dirname, join};
use crate::timestamps::{parse, to_iso};

/// How long a run is kept, in days.
pub const RETENTION_DAYS: f64 = 30.0;

/// How far back `ps` looks unless it is given `--all`, in hours.
pub const RECENT_HOURS: f64 = 24.0;

/// How many runs `ps` lists unless it is given `--limit`.
pub const PS_LIMIT: usize = 50;

const DAY_MS: i64 = 86_400_000;
const HOUR_MS: i64 = 3_600_000;

/// The events of a run's log, in order; lines that do not parse are skipped.
pub fn read_events(runtime: &Runtime, run_dir: &str) -> Vec<Value> {
    let text = runtime.files.read_text(&join(&[run_dir, EVENTS_FILE])).unwrap_or_default();
    text.lines().filter_map(|line| json::parse(line).ok()).collect()
}

fn text_of(object: &Object, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_string)
}

fn number_of(object: &Object, key: &str) -> Option<f64> {
    match object.get(key) {
        Some(Value::Num(number)) => Some(*number),
        _ => None,
    }
}

fn data_of(event: &Value) -> Object {
    event.as_object().and_then(|object| object.get("data")).and_then(Value::as_object).cloned().unwrap_or_default()
}

fn type_of(event: &Value) -> String {
    event.as_object().and_then(|object| text_of(object, "type")).unwrap_or_default()
}

fn ts_of(event: &Value) -> Option<i64> {
    event.as_object().and_then(|object| text_of(object, "ts")).and_then(|ts| parse(&ts))
}

/// A run as its event log and index entry say it is.
#[derive(Clone, Debug, PartialEq)]
pub struct RunView {
    pub run_id: String,
    pub run_dir: String,
    pub workflow: String,
    pub client: String,
    pub model: String,
    pub mode: String,
    /// `running`, `stalled`, `ok`, `failed`, `cancelled` or `lost`.
    pub state: String,
    pub started_ms: i64,
    pub ended_ms: Option<i64>,
    pub last_type: String,
    pub last_ms: Option<i64>,
    /// What the run is doing: its latest event that is more than a heartbeat.
    pub step: String,
    pub heartbeat_age_s: Option<i64>,
    pub silent_s: Option<f64>,
    pub tokens_in: f64,
    pub tokens_out: f64,
    pub cost_usd: f64,
    pub reason: Option<String>,
    pub failure: Option<Value>,
    pub gateway_pid: Option<u32>,
    pub client_pid: Option<u32>,
    pub sandbox: Option<String>,
    pub events: usize,
    /// What served the run, from its `routing` event: the model that answered, the LiteLLM deployment and base URL.
    pub served_model: Option<String>,
    pub litellm_model_id: Option<String>,
    pub api_base: Option<String>,
    /// Whether LiteLLM fell back to another deployment (`None` when the gateway could not see), whether the gateway
    /// itself fell back from a limited free model (a `fallback` event), and whether this run is a retry.
    pub fallback_fired: Option<bool>,
    pub gateway_fallback: bool,
    pub retry: bool,
}

impl RunView {
    pub fn finished(&self) -> bool {
        !matches!(self.state.as_str(), "running" | "stalled")
    }

    pub fn age_s(&self, now_ms: i64) -> i64 {
        (self.ended_ms.unwrap_or(now_ms) - self.started_ms).max(0) / 1000
    }

    pub fn to_json(&self, now_ms: i64) -> Value {
        let mut members = vec![
            ("run_id", Value::str(&self.run_id)),
            ("state", Value::str(&self.state)),
            ("workflow", Value::str(&self.workflow)),
            ("client", Value::str(&self.client)),
            ("model", Value::str(&self.model)),
            ("mode", Value::str(&self.mode)),
            ("started_at", Value::str(&to_iso(self.started_ms))),
            ("age_s", Value::Num(self.age_s(now_ms) as f64)),
            ("step", Value::str(&self.step)),
            ("last_event", Value::str(&self.last_type)),
            ("last_event_age_s", self.last_ms.map_or(Value::Null, |at| Value::Num(((now_ms - at).max(0) / 1000) as f64))),
            ("heartbeat_age_s", self.heartbeat_age_s.map_or(Value::Null, |age| Value::Num(age as f64))),
            ("silent_s", self.silent_s.map_or(Value::Null, Value::Num)),
            ("tokens_in", Value::Num(self.tokens_in)),
            ("tokens_out", Value::Num(self.tokens_out)),
            ("cost_usd", Value::Num(self.cost_usd)),
            ("events", Value::count(self.events)),
            ("served_model", self.served_model.as_deref().map_or(Value::Null, Value::str)),
            ("litellm_model_id", self.litellm_model_id.as_deref().map_or(Value::Null, Value::str)),
            ("api_base", self.api_base.as_deref().map_or(Value::Null, Value::str)),
            ("fallback_fired", self.fallback_fired.map_or(Value::Null, Value::Bool)),
            ("gateway_fallback", Value::Bool(self.gateway_fallback)),
            ("retry", Value::Bool(self.retry)),
            ("run_dir", Value::str(&self.run_dir)),
        ];
        members.extend(self.ended_ms.map(|at| ("ended_at", Value::str(&to_iso(at)))));
        members.extend(self.reason.as_deref().map(|reason| ("reason", Value::str(reason))));
        members.extend(self.failure.clone().map(|failure| ("failure", failure)));
        members.extend(self.client_pid.map(|pid| ("client_pid", Value::Num(f64::from(pid)))));
        members.extend(self.sandbox.as_deref().map(|name| ("sandbox", Value::str(name))));
        json::object(members)
    }
}

/// The one line that says what an event holds.
pub fn summary_of(event: &Value) -> String {
    let data = data_of(event);
    let text = |key: &str| text_of(&data, key).unwrap_or_default();
    let num = |key: &str| number_of(&data, key).map_or_else(String::new, json::number_text);
    let line = match type_of(event).as_str() {
        "run.started" => format!("mode={} time_limit={}s", text("mode"), num("time_limit_s")),
        "run.finished" => {
            let failure = data.get("failure").and_then(Value::as_object).map(|failure| format!(" [{}] {}", text_of(failure, "kind").unwrap_or_default(), text_of(failure, "reason").unwrap_or_default())).unwrap_or_default();
            format!("status={} reason={} exit={}{failure}", text("status"), text("reason"), num("exit_code"))
        }
        "heartbeat" => format!("alive {}s, stdout {}B, stderr {}B, silent {}s", num("alive_s"), num("stdout_bytes"), num("stderr_bytes"), num("last_output_age_s")),
        "diagnostic" => format!("[{}] {}", text("source"), text("text")),
        "stall" => format!("silent {}s of {}s, cancel at {}s", num("silent_s"), num("threshold_s"), num("cancel_at_s")),
        "cancel" => format!("by {}: {}", text("by"), text("reason")),
        "process.started" => format!("pid {}", num("pid")),
        "sandbox.started" => format!("sandbox {}", text("name")),
        _ => ["text", "gen_ai.tool.name", "name", "message", "tool", "summary", "route"].iter().find_map(|key| text_of(&data, key)).unwrap_or_else(|| json::compact(&Value::Obj(data.clone()))),
    };
    line.replace('\n', " ").chars().take(160).collect()
}

/// `ts run_id type summary`, the line `logs` and `tail` print.
pub fn format_event(event: &Value) -> String {
    let object = event.as_object().cloned().unwrap_or_default();
    format!("{} {} {} {}", text_of(&object, "ts").unwrap_or_default(), text_of(&object, "run_id").unwrap_or_default(), type_of(event), summary_of(event)).trim_end().to_string()
}

/// The run as its log says it is. A log that cannot be read leaves what the index knows.
pub fn view_of(runtime: &Runtime, run: &IndexedRun, now_ms: i64) -> RunView {
    let events = read_events(runtime, &run.run_dir);
    let mut view = RunView {
        run_id: run.run_id.clone(),
        run_dir: run.run_dir.clone(),
        workflow: run.workflow.clone(),
        client: run.client.clone(),
        model: run.model.clone(),
        mode: run.mode.clone(),
        state: "running".to_string(),
        started_ms: run.started_ms,
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
        gateway_pid: run.gateway_pid,
        client_pid: None,
        sandbox: None,
        events: events.len(),
        served_model: None,
        litellm_model_id: None,
        api_base: None,
        fallback_fired: None,
        gateway_fallback: false,
        retry: false,
    };
    let mut stalled_at: Option<f64> = None;
    for event in &events {
        let kind = type_of(event);
        let data = data_of(event);
        view.last_type = kind.clone();
        view.last_ms = ts_of(event).or(view.last_ms);
        if kind != "heartbeat" {
            view.step = format!("{kind} {}", summary_of(event));
        }
        match kind.as_str() {
            "run.started" => view.gateway_pid = number_of(&data, "gateway_pid").map(|pid| pid as u32).or(view.gateway_pid),
            "process.started" => view.client_pid = number_of(&data, "pid").map(|pid| pid as u32),
            "sandbox.started" => view.sandbox = text_of(&data, "name"),
            "heartbeat" => {
                view.heartbeat_age_s = ts_of(event).map(|at| (now_ms - at).max(0) / 1000);
                view.silent_s = number_of(&data, "last_output_age_s");
                // Output after the stall ends it.
                if stalled_at.is_some_and(|at| view.silent_s.is_some_and(|silent| silent < at)) {
                    stalled_at = None;
                }
            }
            "stall" => stalled_at = number_of(&data, "silent_s"),
            "routing" => {
                view.served_model = text_of(&data, "served_model");
                view.litellm_model_id = text_of(&data, "litellm_model_id");
                view.api_base = text_of(&data, "api_base");
                view.fallback_fired = match data.get("fallback_fired") {
                    Some(Value::Bool(fired)) => Some(*fired),
                    _ => None,
                };
                view.gateway_fallback |= matches!(data.get("gateway_fallback"), Some(Value::Bool(true)));
            }
            "fallback" => view.gateway_fallback = true,
            "retry" => view.retry = true,
            "usage" => {
                view.tokens_in += number_of(&data, "gen_ai.usage.input_tokens").unwrap_or(0.0);
                view.tokens_out += number_of(&data, "gen_ai.usage.output_tokens").unwrap_or(0.0);
                view.cost_usd += number_of(&data, "cost_usd").unwrap_or(0.0);
            }
            "run.finished" => {
                view.state = text_of(&data, "status").unwrap_or("failed".to_string());
                view.ended_ms = ts_of(event);
                view.reason = text_of(&data, "reason");
                view.failure = data.get("failure").filter(|failure| **failure != Value::Null).cloned();
                if data.get("gen_ai.usage.input_tokens").is_some() {
                    view.tokens_in = number_of(&data, "gen_ai.usage.input_tokens").unwrap_or(0.0);
                    view.tokens_out = number_of(&data, "gen_ai.usage.output_tokens").unwrap_or(0.0);
                    view.cost_usd = number_of(&data, "cost_usd").unwrap_or(0.0);
                }
            }
            _ => {}
        }
    }
    if events.is_empty() {
        // No log to read: the index's own word, when it has one.
        if let Some((status, reason, at)) = &run.finished {
            view.state.clone_from(status);
            view.reason.clone_from(reason);
            view.ended_ms = Some(*at);
        }
    }
    if view.state == "running" && stalled_at.is_some() {
        view.state = "stalled".to_string();
    }
    let gateway_gone = view.gateway_pid.is_some_and(|pid| pid != std::process::id() && !runtime.process.is_alive(pid));
    if !view.finished() && (gateway_gone || events.is_empty() && run.finished.is_none() && gone_for_good(runtime, run)) {
        view.state = "lost".to_string();
        view.reason = Some("gateway-lost".to_string());
    }
    view
}

/// A run with no log and no process to write one: its directory is gone.
fn gone_for_good(runtime: &Runtime, run: &IndexedRun) -> bool {
    runtime.files.exists(&run.run_dir) == Ok(false)
}

fn index_of<'a>(runtime: &'a Runtime) -> Result<RunIndex<'a>, GatewayError> {
    Ok(RunIndex::new(runtime.files.as_ref(), runtime.clock.as_ref(), &crate::locations::run_index_path(&runtime.env)?))
}

/// The runs the index lists, in the order they started.
pub fn indexed_runs(runtime: &Runtime) -> Result<Vec<IndexedRun>, GatewayError> {
    Ok(fold(&index_of(runtime)?.read()))
}

/// The run `key` names: its id, or the start of it when only one run's id starts so.
pub fn locate(runtime: &Runtime, key: &str) -> Result<IndexedRun, GatewayError> {
    if key.is_empty() {
        return Err(GatewayError::invalid_input("name the run: its id, or the start of it"));
    }
    let runs = indexed_runs(runtime)?;
    if let Some(run) = runs.iter().find(|run| run.run_id == key) {
        return Ok(run.clone());
    }
    let matching: Vec<&IndexedRun> = runs.iter().filter(|run| run.run_id.starts_with(key)).collect();
    match matching.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(GatewayError::new("unknown-run", format!("no run {key} is in the run index"))),
        many => Err(GatewayError::new("ambiguous-run", format!("{key} starts {} runs: {}", many.len(), many.iter().take(5).map(|run| run.run_id.as_str()).collect::<Vec<_>>().join(", ")))),
    }
}

fn run_key(input: &Object) -> Result<String, GatewayError> {
    match input.get("run") {
        Some(Value::Str(run)) => Ok(run.clone()),
        _ => Err(GatewayError::invalid_input("name the run: its id, or the start of it")),
    }
}

fn ok_with(members: Vec<(&str, Value)>) -> Value {
    let mut all = vec![("status", Value::str("ok"))];
    all.extend(members);
    json::object(all)
}

fn flag_number(input: &Object, key: &str) -> Result<Option<f64>, GatewayError> {
    match input.get(key) {
        None => Ok(None),
        Some(Value::Num(number)) if number.is_finite() && *number >= 0.0 => Ok(Some(*number)),
        Some(_) => Err(GatewayError::invalid_input(format!("{key} must be a number of at least 0"))),
    }
}

/// `ps`: the running runs and the recent ones, newest first. `all` lists every run the index holds, `workflow`
/// and `state` narrow the list, `limit` cuts it.
pub fn ps(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let now = runtime.clock.now_ms();
    let all = matches!(input.get("all"), Some(Value::Bool(true)));
    let limit = flag_number(input, "limit")?.map_or(PS_LIMIT, |limit| limit as usize);
    let workflow = text_of(input, "workflow");
    let state = text_of(input, "state");
    let recent_from = now - (RECENT_HOURS * HOUR_MS as f64) as i64;
    let mut views: Vec<RunView> = indexed_runs(runtime)?
        .iter()
        .rev()
        .filter(|run| workflow.as_deref().is_none_or(|workflow| run.workflow == workflow))
        .filter(|run| all || run.started_ms >= recent_from || run.finished.is_none())
        .map(|run| view_of(runtime, run, now))
        .filter(|view| state.as_deref().is_none_or(|state| view.state == state))
        .filter(|view| all || !view.finished() || view.started_ms >= recent_from)
        .take(limit)
        .collect();
    views.sort_by_key(|view| std::cmp::Reverse(view.started_ms));
    Ok(ok_with(vec![("runs", Value::Arr(views.iter().map(|view| view.to_json(now)).collect()))]))
}

/// `ps` as a table.
pub fn ps_table(result: &Value) -> String {
    let runs = match result.as_object().and_then(|object| object.get("runs")) {
        Some(Value::Arr(items)) => items.clone(),
        _ => Vec::new(),
    };
    let mut rows = vec![["RUN".to_string(), "STATE".into(), "CLIENT".into(), "MODEL".into(), "MODE".into(), "AGE".into(), "LAST".into()]];
    for run in runs {
        let object = run.as_object().cloned().unwrap_or_default();
        let get = |key: &str| text_of(&object, key).unwrap_or_default();
        let age = number_of(&object, "age_s").unwrap_or(0.0) as i64;
        rows.push([get("run_id"), get("state"), get("client"), get("model"), get("mode"), age_words(age), get("last_event")]);
    }
    let widths: Vec<usize> = (0..7).map(|column| rows.iter().map(|row| row[column].chars().count()).max().unwrap_or(0)).collect();
    rows.iter().map(|row| format!("{}\n", row.iter().enumerate().map(|(column, cell)| format!("{cell:<width$}", width = widths[column])).collect::<Vec<_>>().join("  ").trim_end())).collect()
}

fn age_words(seconds: i64) -> String {
    match seconds {
        0..=119 => format!("{seconds}s"),
        120..=7199 => format!("{}m", seconds / 60),
        7200..=172_799 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

/// `status <run>`: one run, from its event log.
pub fn status(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let run = locate(runtime, &run_key(input)?)?;
    let now = runtime.clock.now_ms();
    Ok(ok_with(vec![("run", view_of(runtime, &run, now).to_json(now))]))
}

/// `status` as lines of `name: value`.
pub fn status_text(result: &Value) -> String {
    let object = result.as_object().and_then(|object| object.get("run")).and_then(Value::as_object).cloned().unwrap_or_default();
    object
        .entries()
        .into_iter()
        .map(|(name, value)| {
            let shown = match value {
                Value::Str(text) => text.clone(),
                Value::Null => "-".to_string(),
                other => json::compact(other),
            };
            format!("{name}: {shown}\n")
        })
        .collect()
}

/// The comma-separated list of event types, as a list.
pub fn type_filter(input: &Object) -> Vec<String> {
    text_of(input, "types").map(|types| types.split(',').map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).collect()).unwrap_or_default()
}

/// `logs <run>`: the run's events (all of them, or the types named).
pub fn logs(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let run = locate(runtime, &run_key(input)?)?;
    let types = type_filter(input);
    let events: Vec<Value> = read_events(runtime, &run.run_dir).into_iter().filter(|event| types.is_empty() || types.contains(&type_of(event))).collect();
    Ok(ok_with(vec![("run_id", Value::str(&run.run_id)), ("events", Value::Arr(events))]))
}

/// `logs` as one line per event.
pub fn logs_text(result: &Value) -> String {
    match result.as_object().and_then(|object| object.get("events")) {
        Some(Value::Arr(events)) => events.iter().map(|event| format!("{}\n", format_event(event))).collect(),
        _ => String::new(),
    }
}

/// `gc`: removes the directories of runs that ended (or were lost) more than `days` days ago (30 by default),
/// notes each removal in the index, and drops rotated index files that old. A run still going is never removed.
pub fn gc(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let days = flag_number(input, "days")?.unwrap_or(RETENTION_DAYS);
    let dry = matches!(input.get("dry_run"), Some(Value::Bool(true)));
    let now = runtime.clock.now_ms();
    let cutoff = now - (days * DAY_MS as f64) as i64;
    let index = index_of(runtime)?;
    let mut removed: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    for run in fold(&index.read()).iter().filter(|run| run.started_ms < cutoff) {
        let view = view_of(runtime, run, now);
        if !view.finished() {
            continue;
        }
        // Only a directory the gateway made for a run: `.../runs/<run id>`.
        if basename(&run.run_dir) != run.run_id || basename(dirname(&run.run_dir)) != "runs" {
            skipped.push(json::object(vec![("run_id", Value::str(&run.run_id)), ("reason", Value::str("its directory is not .../runs/<run id>"))]));
            continue;
        }
        if !dry {
            if let Err(error) = runtime.files.remove(&run.run_dir) {
                skipped.push(json::object(vec![("run_id", Value::str(&run.run_id)), ("reason", Value::str(&error.message))]));
                continue;
            }
            index.record(&IndexEntry {
                kind: IndexKind::Removed,
                ts_ms: now,
                run_id: run.run_id.clone(),
                run_dir: run.run_dir.clone(),
                workflow: run.workflow.clone(),
                client: run.client.clone(),
                model: run.model.clone(),
                family: run.family.clone(),
                mode: run.mode.clone(),
                gateway_pid: None,
                status: None,
                reason: None,
            });
        }
        removed.push(Value::str(&run.run_id));
    }
    let mut files: Vec<Value> = Vec::new();
    for (path, modified) in index.rotated_files() {
        if modified.is_some_and(|at| at < cutoff) {
            if !dry {
                let _ = runtime.files.remove_file(&path);
            }
            files.push(Value::str(&path));
        }
    }
    Ok(ok_with(vec![("days", Value::Num(days)), ("dry_run", Value::Bool(dry)), ("removed", Value::Arr(removed)), ("skipped", Value::Arr(skipped)), ("index_files_removed", Value::Arr(files))]))
}

/// `gc` as a sentence.
pub fn gc_text(result: &Value) -> String {
    let object = result.as_object().cloned().unwrap_or_default();
    let count = |key: &str| match object.get(key) {
        Some(Value::Arr(items)) => items.len(),
        _ => 0,
    };
    let verb = if matches!(object.get("dry_run"), Some(Value::Bool(true))) { "would remove" } else { "removed" };
    format!("{verb} {} run directories and {} rotated index files; {} left alone\n", count("removed"), count("index_files_removed"), count("skipped"))
}
