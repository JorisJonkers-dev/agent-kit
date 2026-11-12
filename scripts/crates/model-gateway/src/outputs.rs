//! What each client's own output says about a run: the final answer, the tokens
//! it counted, the dollars it charged, why it failed although it exited 0, and
//! the error its structured output carries.

use crate::json::{self, Object, Value};
use crate::run::{charged_of, count, final_answer_of, hermes_usage, one_shot_answer, run_tokens, ClientError, RunTokens, NO_TOKENS};
use crate::text::js_trim;

/// What one client's own output says about a run: the final answer, the tokens
/// it counted, the dollars it charged when it priced the run, why it failed
/// when its output says so although it exited 0, and the error its own
/// structured output carries.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientOutput {
    /// The delegate's final answer; `None` when the output holds none.
    pub answer: Option<String>,
    pub tokens: RunTokens,
    /// The dollars the client itself charged; `None` when it priced nothing.
    pub charged: Option<f64>,
    /// `timeout`, `denied-action` or `client-error`; `None` when the output reports no failure.
    pub failure: Option<String>,
    /// The error the client's own structured output carries; `None` when it reports none.
    pub error: Option<ClientError>,
}

/// A non-blank string, or `None`.
fn text_of(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|text| !js_trim(text).is_empty())
}

/// Parsed JSON, or `None` when the text is not JSON.
fn parse_json(text: &str) -> Option<Value> {
    json::parse(text).ok()
}

/// The dollars a client reported when it priced the run: a number above 0, else none.
fn charged_amount(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

/// The text of every member of `value` the keys name that is a string.
fn strings_of<'a>(value: &'a Object, keys: &[&str]) -> Vec<&'a str> {
    keys.iter().filter_map(|key| value.get(key).and_then(Value::as_str)).collect()
}

/// The error a client's structured output carries, as text the refusal
/// patterns can read: its `error` field, its stop reason, and the result of a
/// run it marks as an error. A client that reports no error has none.
fn structured_error(exported: &Value) -> Option<ClientError> {
    let exported = exported.as_object()?;
    let mut parts: Vec<&str> = Vec::new();
    let mut status = None;
    match exported.get("error") {
        Some(Value::Str(error)) => parts.push(error),
        Some(Value::Obj(error)) => {
            parts.extend(strings_of(error, &["message", "type", "code", "name"]));
            status = http_status(error);
        }
        _ => {}
    }
    // `stop_reason ?? stopReason`: a null counts as absent.
    let stop = match exported.get("stop_reason") {
        None | Some(Value::Null) => exported.get("stopReason"),
        found => found,
    };
    if let Some(Value::Str(stop)) = stop {
        parts.push(stop);
    }
    let marked = exported.get("is_error") == Some(&Value::Bool(true)) || exported.get("isError") == Some(&Value::Bool(true));
    if let (true, Some(Value::Str(result))) = (marked, exported.get("result")) {
        parts.push(result);
    }
    if status.is_none() {
        status = http_status(exported);
    }
    if parts.is_empty() && status.is_none() {
        return None;
    }
    Some(ClientError { text: parts.join(" "), status })
}

/// The HTTP status an object names, when it names one as a whole number.
fn http_status(value: &Object) -> Option<f64> {
    ["status", "status_code", "statusCode", "code"].iter().find_map(|key| match value.get(key) {
        Some(Value::Num(number)) if number.is_finite() && number.fract() == 0.0 => Some(*number),
        _ => None,
    })
}

fn nothing() -> ClientOutput {
    ClientOutput { answer: None, charged: None, error: None, failure: None, tokens: NO_TOKENS }
}

/// Vibe: the journal on stdout holds the answer, and the export it wrote holds the tokens, the dollars and any error.
pub fn vibe_output(stdout: &str, exported: Option<&Value>) -> ClientOutput {
    ClientOutput {
        answer: final_answer_of(stdout),
        charged: charged_of(exported),
        error: exported.and_then(structured_error),
        failure: None,
        tokens: run_tokens(exported),
    }
}

/// Antigravity: one object with the `response` and a `usage` of token counts,
/// and no dollars (it is quota, not money), whether it printed that object
/// alone (`--output-format json`) or as the `result` event of a stream
/// (`--output-format stream-json`). agy reports `SUCCESS` and exits 0 for runs
/// that produced nothing, so the fields decide: a print timeout, a denied
/// action or another status is a failure.
pub fn agy_output(stdout: &str) -> ClientOutput {
    let result = agy_result_of(stdout);
    let Some(object) = result.as_ref().and_then(Value::as_object) else {
        return nothing();
    };
    let usage = object.get("usage").and_then(Value::as_object).cloned().unwrap_or_default();
    ClientOutput {
        answer: text_of(object.get("response")).map(str::to_string),
        charged: None,
        error: result.as_ref().and_then(structured_error),
        failure: agy_failure(object),
        tokens: RunTokens {
            cached: count(usage.get("cache_read_tokens")),
            input: count(usage.get("input_tokens")),
            output: count(usage.get("output_tokens")),
        },
    }
}

/// The result an agy run printed: the `result` of its last result event when it
/// printed a stream, else the one object it printed, else nothing.
fn agy_result_of(stdout: &str) -> Option<Value> {
    let events: Vec<Value> = stdout.split('\n').filter_map(parse_json).filter(|event| event.as_object().is_some()).collect();
    let streamed = events
        .iter()
        .rfind(|event| event.as_object().and_then(|event| event.get("event")).and_then(Value::as_str) == Some("result"))
        .and_then(|event| event.as_object()?.get("result"))
        .filter(|result| **result != Value::Null);
    streamed
        .or_else(|| events.iter().rfind(|event| event.as_object().is_some_and(|event| event.get("response").is_some() || event.get("usage").is_some())))
        .cloned()
}

fn agy_failure(result: &Object) -> Option<String> {
    if result.get("finished_reason").and_then(Value::as_str) == Some("print_timeout") {
        return Some("timeout".to_string());
    }
    if matches!(result.get("denied_actions"), Some(Value::Arr(denied)) if !denied.is_empty()) {
        return Some("denied-action".to_string());
    }
    match result.get("status") {
        Some(Value::Str(status)) if status != "SUCCESS" => Some("client-error".to_string()),
        _ => None,
    }
}

/// Claude Code, `--output-format json`: one `result` object (or a list of
/// events that ends in one) with the answer in `result`, the dollars in
/// `total_cost_usd` and the tokens in `usage`. Anthropic counts cached prompt
/// apart from `input_tokens`, so the input here is the sum, which is what a
/// Vibe run's input counts.
pub fn claude_output(stdout: &str) -> ClientOutput {
    let result = match parse_json(stdout) {
        Some(Value::Arr(events)) => events
            .into_iter()
            .rfind(|event| event.as_object().and_then(|event| event.get("type")).and_then(Value::as_str) == Some("result")),
        parsed => parsed,
    };
    let Some(object) = result.as_ref().and_then(Value::as_object) else {
        return nothing();
    };
    let usage = object.get("usage").and_then(Value::as_object).cloned().unwrap_or_default();
    let cached = count(usage.get("cache_read_input_tokens"));
    ClientOutput {
        answer: text_of(object.get("result")).map(str::to_string),
        charged: match object.get("total_cost_usd") {
            Some(Value::Num(cost)) => charged_amount(*cost),
            _ => None,
        },
        error: result.as_ref().and_then(structured_error),
        failure: (object.get("is_error") == Some(&Value::Bool(true))).then(|| "client-error".to_string()),
        tokens: RunTokens {
            cached,
            input: count(usage.get("input_tokens")) + cached + count(usage.get("cache_creation_input_tokens")),
            output: count(usage.get("output_tokens")),
        },
    }
}

/// Hermes, one-shot: `hermes -z` prints the response text alone, and the usage
/// report it wrote holds the tokens it counted, the dollars it estimated and
/// whether it counts the run as failed. A report that is missing or holds
/// nonsense counts no tokens and no dollars, and a run it does not call failed
/// is not failed.
pub fn hermes_output(stdout: &str, usage: Option<&Value>) -> ClientOutput {
    let report = hermes_usage(usage);
    ClientOutput {
        answer: one_shot_answer(stdout),
        charged: report.cost,
        error: None,
        failure: report.failed.then(|| "failed".to_string()),
        tokens: report.tokens,
    }
}

/// What an event says in its own `message`, as the error the refusal patterns read.
fn said_of(event: &Object) -> Option<ClientError> {
    text_of(event.get("message")).map(|text| ClientError { text: text.to_string(), status: None })
}

/// Codex, `exec --json`: one event per line. The answer is the text of the last
/// `agent_message` item; every `turn.completed` adds the tokens its turn
/// reported, its cached prompt counted apart from its input as Vibe's export
/// counts it, and its reasoning tokens already inside its output; an `error` or
/// a `turn.failed` event fails the run. Codex prices nothing, so its dollars
/// come from the catalogue's prices.
pub fn codex_output(stdout: &str) -> ClientOutput {
    let mut answer = None;
    let mut error = None;
    let mut failure = None;
    let mut sum = NO_TOKENS;
    for line in stdout.split('\n') {
        let Some(Value::Obj(event)) = parse_json(line) else {
            continue;
        };
        let item = event.get("item").and_then(Value::as_object).cloned().unwrap_or_default();
        let kind = event.get("type").and_then(Value::as_str);
        if kind == Some("item.completed") && item.get("type").and_then(Value::as_str) == Some("agent_message") && text_of(item.get("text")).is_some() {
            answer = text_of(item.get("text")).map(str::to_string);
        } else if kind == Some("turn.completed") {
            let usage = event.get("usage").and_then(Value::as_object).cloned().unwrap_or_default();
            sum.cached += count(usage.get("cached_input_tokens"));
            sum.input += count(usage.get("input_tokens"));
            sum.output += count(usage.get("output_tokens"));
        } else if kind == Some("error") || kind == Some("turn.failed") {
            failure = Some("client-error".to_string());
            error = structured_error(&Value::Obj(event.clone())).or_else(|| said_of(&event));
        }
    }
    ClientOutput { answer, charged: None, error, failure, tokens: sum }
}

/// OpenCode, `--format json`: one event per line. The answer is the text of the
/// last step that wrote any; every `step_finish` adds its dollars and tokens
/// (its cache is counted apart from its input, as Anthropic's is, and its
/// reasoning is billed as output); an `error` event fails the run.
pub fn open_code_output(stdout: &str) -> ClientOutput {
    let mut answer: Option<String> = None;
    let mut fresh = true;
    let mut charged = 0.0;
    let mut error = None;
    let mut failure = None;
    let mut sum = NO_TOKENS;
    for line in stdout.split('\n') {
        let Some(Value::Obj(event)) = parse_json(line) else {
            continue;
        };
        let part = event.get("part").and_then(Value::as_object).cloned().unwrap_or_default();
        let kind = event.get("type").and_then(Value::as_str);
        if kind == Some("step_start") {
            fresh = true;
        } else if kind == Some("text") && text_of(part.get("text")).is_some() {
            let text = text_of(part.get("text")).unwrap_or_default();
            answer = Some(if fresh { text.to_string() } else { format!("{}{text}", answer.unwrap_or_default()) });
            fresh = false;
        } else if kind == Some("step_finish") {
            let tokens = part.get("tokens").and_then(Value::as_object).cloned().unwrap_or_default();
            let cache = tokens.get("cache").and_then(Value::as_object).cloned().unwrap_or_default();
            charged += count(part.get("cost"));
            sum.cached += count(cache.get("read"));
            sum.input += count(tokens.get("input")) + count(cache.get("read")) + count(cache.get("write"));
            sum.output += count(tokens.get("output")) + count(tokens.get("reasoning"));
        } else if kind == Some("error") {
            failure = Some("client-error".to_string());
            error = structured_error(&Value::Obj(event));
        }
    }
    ClientOutput { answer, charged: charged_amount(charged), error, failure, tokens: sum }
}
