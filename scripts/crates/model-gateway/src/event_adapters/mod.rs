//! Event adapters: what each harness prints while it runs, turned into the event types of the run's log
//! (ADR-0017, ADR-0020): `message`, `plan`, `tool.started`, `tool.finished` and `usage`.
//!
//! An adapter reads the harness's stdout one line at a time. A harness with a native JSON stream (Claude, Codex,
//! OpenCode, pi, Vibe, Antigravity) has a [`Mapper`] that reads one JSON object and says which events it is; a
//! line it does not know, or that is not JSON, becomes a `diagnostic` and never fails the run. Hermes has no
//! native stream, so its adapter drives the harness over ACP (`acp`), which writes to its stdin as well.
//!
//! Event data follows OpenTelemetry's GenAI names where one exists: `gen_ai.tool.name`, `gen_ai.tool.call.id`,
//! `gen_ai.usage.input_tokens` and `gen_ai.usage.output_tokens`. Text is clipped, so a log line stays small.

mod acp;
mod agy;
mod claude;
mod codex;
mod opencode;
mod pi;
mod vibe;

pub use acp::{transcript, Transcript};

use crate::json::{self, Object, Value};
pub use crate::ports::Reply;
use crate::run::count;

/// How many characters of a message one event keeps.
const MESSAGE_CHARS: usize = 8_000;

/// How many characters of a tool's input or output one event keeps.
const TOOL_CHARS: usize = 2_000;

/// How many characters of an unrecognised line its diagnostic keeps.
const LINE_CHARS: usize = 300;

/// One event an adapter read from the harness's output: its type and its data.
#[derive(Clone, Debug, PartialEq)]
pub struct AdapterEvent {
    pub kind: &'static str,
    pub data: Value,
}

/// Reads one harness's stdout a line at a time.
pub trait Adapter: Send {
    /// The events one non-blank stdout line is.
    fn line(&mut self, text: &str) -> Vec<AdapterEvent>;
    /// What the adapter still holds when the output ends.
    fn finish(&mut self) -> Vec<AdapterEvent> {
        Vec::new()
    }
    /// Whether the harness is driven through its stdin (ACP) and not only read.
    fn drives(&self) -> bool {
        false
    }
    /// Tells a driving adapter what to ask the harness: the prompt and the directory it works in.
    fn drive(&mut self, _prompt: &str, _cwd: &str) {}
    /// What a driving adapter writes first.
    fn opening(&mut self) -> String {
        String::new()
    }
    /// What a driving adapter has to say after the lines it has read.
    fn outgoing(&mut self) -> Reply {
        Reply::default()
    }
}

/// The adapter for a client of the gateway, `None` for one whose output is not read into events.
pub fn for_client(client: &str) -> Option<Box<dyn Adapter>> {
    Some(match client {
        "claude" => Box::new(Lines(claude::Claude::default())),
        "codex" => Box::new(Lines(codex::Codex)),
        "opencode" => Box::new(Lines(opencode::OpenCode)),
        "vibe" => Box::new(Lines(vibe::Vibe)),
        "agy" => Box::new(Lines(agy::Agy::default())),
        "hermes" => Box::new(acp::Acp::default()),
        other if other == "pi" || other.starts_with("pi-") => Box::new(Lines(pi::Pi)),
        _ => return None,
    })
}

/// Turns one parsed line of a native JSON stream into events.
pub trait Mapper: Send {
    /// The events the object is; `None` when it is none the mapper knows, which becomes a diagnostic.
    fn map(&mut self, event: &Object) -> Option<Vec<AdapterEvent>>;
}

/// A native JSON stream: a line that is not an object, or that the mapper does not know, is a diagnostic.
struct Lines<M>(M);

impl<M: Mapper> Adapter for Lines<M> {
    fn line(&mut self, text: &str) -> Vec<AdapterEvent> {
        match json::parse(text) {
            Ok(Value::Obj(event)) => self.0.map(&event).unwrap_or_else(|| vec![diagnostic(&format!("unrecognised output line: {}", clip(text, LINE_CHARS)))]),
            _ => vec![diagnostic(&format!("output line is not a JSON object: {}", clip(text, LINE_CHARS)))],
        }
    }
}

/// Text cut to `limit` characters.
pub(crate) fn clip(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// An adapter's own diagnostic: a line it could not read. It never fails the run.
pub(crate) fn diagnostic(text: &str) -> AdapterEvent {
    AdapterEvent { kind: "diagnostic", data: json::object(vec![("level", Value::str("info")), ("source", Value::str("adapter")), ("text", Value::str(text))]) }
}

/// A `diagnostic` the harness itself reported as an error.
pub(crate) fn harness_error(text: &str) -> AdapterEvent {
    AdapterEvent { kind: "diagnostic", data: json::object(vec![("level", Value::str("error")), ("source", Value::str("harness")), ("text", Value::str(&clip(text, MESSAGE_CHARS)))]) }
}

/// What the model said (`kind` is `text`) or thought (`thinking`).
pub(crate) fn message(role: &str, kind: &str, text: &str) -> AdapterEvent {
    AdapterEvent { kind: "message", data: json::object(vec![("role", Value::str(role)), ("kind", Value::str(kind)), ("text", Value::str(&clip(text, MESSAGE_CHARS)))]) }
}

/// A plan or to-do list: each entry's text and status.
pub(crate) fn plan(entries: Vec<(String, String)>) -> AdapterEvent {
    let entries = entries.iter().map(|(text, status)| json::object(vec![("text", Value::str(text)), ("status", Value::str(status))])).collect();
    AdapterEvent { kind: "plan", data: json::object(vec![("entries", Value::Arr(entries))]) }
}

/// A tool call that began.
pub(crate) fn tool_started(id: &str, name: &str, input: &Value) -> AdapterEvent {
    AdapterEvent {
        kind: "tool.started",
        data: json::object(vec![("gen_ai.tool.call.id", Value::str(id)), ("gen_ai.tool.name", Value::str(name)), ("input", Value::str(&clip(&json::compact(input), TOOL_CHARS)))]),
    }
}

/// A tool call that ended, `ok` or `error`.
pub(crate) fn tool_finished(id: &str, name: &str, ok: bool, output: &str) -> AdapterEvent {
    AdapterEvent {
        kind: "tool.finished",
        data: json::object(vec![
            ("gen_ai.tool.call.id", Value::str(id)),
            ("gen_ai.tool.name", Value::str(name)),
            ("status", Value::str(if ok { "ok" } else { "error" })),
            ("output", Value::str(&clip(output, TOOL_CHARS))),
        ]),
    }
}

/// Tokens counted as the spend record counts them (`RunTokens`: the cached prompt is inside the input) and the
/// dollars the harness itself charged, `null` when it priced nothing.
pub(crate) fn usage(input: f64, output: f64, cached: f64, cost_usd: Option<f64>) -> AdapterEvent {
    AdapterEvent {
        kind: "usage",
        data: json::object(vec![
            ("gen_ai.usage.input_tokens", Value::Num(input)),
            ("gen_ai.usage.output_tokens", Value::Num(output)),
            ("cached_input_tokens", Value::Num(cached)),
            ("cost_usd", cost_usd.map_or(Value::Null, Value::Num)),
        ]),
    }
}

/// A member's text.
pub(crate) fn text<'a>(object: &'a Object, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

/// The text at the end of a path of members, `None` when any step of it is missing.
pub(crate) fn text_in<'a>(object: &'a Object, path: &[&str]) -> Option<&'a str> {
    let (last, parents) = path.split_last()?;
    let mut at = object;
    for key in parents {
        at = at.get(key)?.as_object()?;
    }
    text(at, last)
}

/// A member's object, empty when it has none.
pub(crate) fn object_of(object: &Object, key: &str) -> Object {
    object.get(key).and_then(Value::as_object).cloned().unwrap_or_default()
}

/// A member's array, empty when it has none.
pub(crate) fn array_of<'a>(object: &'a Object, key: &str) -> &'a [Value] {
    match object.get(key) {
        Some(Value::Arr(items)) => items,
        _ => &[],
    }
}

/// A member as a count, 0 when it is none.
pub(crate) fn number(object: &Object, key: &str) -> f64 {
    count(object.get(key))
}

/// The text of a tool result's content, which is a string or a list of `{type: text, text}` parts.
pub(crate) fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::Str(text)) => text.clone(),
        Some(Value::Arr(parts)) => parts.iter().filter_map(Value::as_object).filter_map(|part| text(part, "text")).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// The usage event of a run's result, for a harness whose own output reports none: `cost_usd` is the gateway's
/// figure, which is `null` when there is none.
pub fn usage_of_result(input: f64, output: f64, cached: f64, cost_usd: f64) -> AdapterEvent {
    usage(input, output, cached, Some(cost_usd).filter(|cost| *cost > 0.0))
}

/// What a client's stderr says, for the words that refuse a run (`rate limit`, `forbidden`). `hermes acp` logs at
/// INFO to stderr as Python's logger does, and its lines name HTTP errors of requests it went on without (a model
/// catalogue it could not fetch: `403 Forbidden`), which are not why a run failed; every other client's stderr is
/// its own error channel as it is.
pub fn error_channel(client: &str, stderr: &str) -> String {
    if client != "hermes" {
        return stderr.to_string();
    }
    stderr.lines().filter(|line| !line.contains(" [INFO] ")).map(|line| format!("{line}\n")).collect()
}
