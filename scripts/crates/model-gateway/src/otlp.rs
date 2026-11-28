//! `export`: the event logs as OpenTelemetry traces, for a collector or a file. One trace per run with one span
//! for the run, and each event of its log a span event. The `gen_ai.*` attributes go out under the names and
//! with the values the log holds; the rest of an event's data goes under `gateway.*`.
//!
//! Off unless asked: `export` does nothing until an endpoint (`--endpoint`, `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`
//! or `OTEL_EXPORTER_OTLP_ENDPOINT`) or a file (`--file`) is named, and nothing in the gateway starts one by
//! itself. The request is OTLP/HTTP with a JSON body, which a collector takes on `/v1/traces`; the file gets the
//! same request as one line, as OpenTelemetry's own file exporter writes it.
//!
//! The text of messages and tool calls never leaves the machine through it (spec #934, story 51): `text`,
//! `input` and `output` are left out of the attributes, and a long value is cut.

use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::ports::{FsError, ProcessCommand, Runtime};
use crate::run_index::IndexedRun;
use crate::run_status::{indexed_runs, read_events};
use crate::sha256::sha256_hex;
use crate::timestamps;

/// The data keys that hold what the run said or read, which are not exported.
const CONTENT_KEYS: [&str; 3] = ["text", "input", "output"];

/// How long an exported text value may be, in characters.
const VALUE_CHARS: usize = 256;

/// How long the collector may take to answer, in milliseconds.
const POST_TIMEOUT_MS: u64 = 30_000;

fn attribute(key: &str, value: &Value) -> Option<Value> {
    let typed = match value {
        Value::Str(text) => json::object(vec![("stringValue", Value::str(&text.chars().take(VALUE_CHARS).collect::<String>()))]),
        Value::Bool(flag) => json::object(vec![("boolValue", Value::Bool(*flag))]),
        // An integer is an `intValue`, which OTLP's JSON writes as a string.
        Value::Num(number) if number.fract() == 0.0 && number.abs() < 9_007_199_254_740_992.0 => json::object(vec![("intValue", Value::str(&format!("{}", *number as i64)))]),
        Value::Num(number) => json::object(vec![("doubleValue", Value::Num(*number))]),
        Value::Arr(_) | Value::Obj(_) => json::object(vec![("stringValue", Value::str(&json::compact(value).chars().take(VALUE_CHARS).collect::<String>()))]),
        Value::Null => return None,
    };
    Some(json::object(vec![("key", Value::str(key)), ("value", typed)]))
}

/// The attributes of an event's data: `gen_ai.*` as they are, the rest under `gateway.`, content left out.
fn data_attributes(data: &Object) -> Vec<Value> {
    data.entries()
        .into_iter()
        .filter(|(key, _)| !CONTENT_KEYS.contains(key))
        .filter_map(|(key, value)| attribute(&if key.starts_with("gen_ai.") { key.to_string() } else { format!("gateway.{key}") }, value))
        .collect()
}

fn envelope_attributes(envelope: &Object) -> Vec<Value> {
    envelope.entries().into_iter().filter(|(key, _)| key.starts_with("gen_ai.")).filter_map(|(key, value)| attribute(key, value)).collect()
}

fn nanos(ms: i64) -> Value {
    Value::str(&format!("{}", ms * 1_000_000))
}

fn ts_ms(event: &Object) -> Option<i64> {
    event.get("ts").and_then(Value::as_str).and_then(timestamps::parse)
}

/// The span of one run, or `None` for a run whose log has no event.
pub fn span_of(run: &IndexedRun, events: &[Value]) -> Option<Value> {
    let objects: Vec<&Object> = events.iter().filter_map(Value::as_object).collect();
    let first = objects.first()?;
    let start = ts_ms(first).unwrap_or(run.started_ms);
    let finished = objects.iter().find(|event| event.get("type").and_then(Value::as_str) == Some("run.finished"));
    let finish_data = finished.and_then(|event| event.get("data")).and_then(Value::as_object);
    let end = objects.iter().rev().find_map(|event| ts_ms(event)).unwrap_or(start);
    let mut attributes = envelope_attributes(first);
    attributes.extend(attribute("gateway.run_id", &Value::str(&run.run_id)));
    attributes.extend(attribute("gateway.workflow", &Value::str(&run.workflow)));
    attributes.extend(attribute("gateway.mode", &Value::str(&run.mode)));
    if let Some(data) = finish_data {
        attributes.extend(envelope_attributes(data));
        attributes.extend(data.get("status").and_then(|status| attribute("gateway.status", status)));
    }
    let (code, message) = match finish_data.and_then(|data| data.get("status")).and_then(Value::as_str) {
        None => (0.0, None),
        Some("ok") => (1.0, None),
        Some(status) => (2.0, Some(finish_data.and_then(|data| data.get("reason")).and_then(Value::as_str).unwrap_or(status).to_string())),
    };
    let mut status = vec![("code", Value::Num(code))];
    status.extend(message.map(|text| ("message", Value::str(&text))));
    let span_events: Vec<Value> = objects
        .iter()
        .map(|event| {
            let mut attributes = Vec::new();
            attributes.extend(event.get("seq").and_then(|seq| attribute("gateway.seq", seq)));
            attributes.extend(event.get("data").and_then(Value::as_object).map(data_attributes).unwrap_or_default());
            json::object(vec![
                ("timeUnixNano", nanos(ts_ms(event).unwrap_or(start))),
                ("name", Value::str(event.get("type").and_then(Value::as_str).unwrap_or("event"))),
                ("attributes", Value::Arr(attributes)),
            ])
        })
        .collect();
    let name = format!("{} {}", first.get("gen_ai.operation.name").and_then(Value::as_str).unwrap_or("invoke_agent"), run.client);
    Some(json::object(vec![
        ("traceId", Value::str(&sha256_hex(run.run_id.as_bytes())[..32])),
        ("spanId", Value::str(&sha256_hex(format!("span:{}", run.run_id).as_bytes())[..16])),
        ("name", Value::str(&name)),
        ("kind", Value::Num(3.0)),
        ("startTimeUnixNano", nanos(start)),
        ("endTimeUnixNano", nanos(end)),
        ("attributes", Value::Arr(attributes)),
        ("events", Value::Arr(span_events)),
        ("status", json::object(status)),
    ]))
}

/// The OTLP trace request for `spans`.
pub fn request_of(spans: Vec<Value>) -> Value {
    let resource = json::object(vec![("attributes", Value::Arr(attribute("service.name", &Value::str("model-gateway")).into_iter().collect()))]);
    let scope = json::object(vec![("name", Value::str("model-gateway")), ("version", Value::str(env!("CARGO_PKG_VERSION")))]);
    json::object(vec![("resourceSpans", Value::Arr(vec![json::object(vec![("resource", resource), ("scopeSpans", Value::Arr(vec![json::object(vec![("scope", scope), ("spans", Value::Arr(spans))])]))])]))])
}

/// What `export` was asked: where to, and which runs.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExportOptions {
    pub endpoint: Option<String>,
    pub file: Option<String>,
    pub run: Option<String>,
    pub workflow: Option<String>,
}

pub const USAGE: &str = "export takes [--endpoint URL] [--file PATH] [--run RUN] [--workflow NAME]; name an endpoint (or set OTEL_EXPORTER_OTLP_ENDPOINT) or a file";

pub fn parse_args(args: &[String]) -> Option<ExportOptions> {
    let mut options = ExportOptions::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let value = iter.next()?.clone();
        match arg.as_str() {
            "--endpoint" => options.endpoint = Some(value),
            "--file" => options.file = Some(value),
            "--run" => options.run = Some(value),
            "--workflow" => options.workflow = Some(value),
            _ => return None,
        }
    }
    Some(options)
}

/// The traces endpoint: the one named, the signal's own variable as it is, or the base endpoint with `/v1/traces`.
fn endpoint_of(options: &ExportOptions, runtime: &Runtime) -> Option<String> {
    let env = |key: &str| runtime.env.get(key).filter(|value| !value.is_empty()).cloned();
    options.endpoint.clone().or_else(|| env("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")).or_else(|| env("OTEL_EXPORTER_OTLP_ENDPOINT").map(|base| format!("{}/v1/traces", base.trim_end_matches('/'))))
}

/// A value as a curl config file reads it between double quotes.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"").replace(['\n', '\r'], ""))
}

fn fs_message(error: FsError) -> String {
    error.message
}

/// Posts `body` to `endpoint` with curl, whose config (the URL and `OTEL_EXPORTER_OTLP_HEADERS`) arrives on stdin
/// so a credential in a header is never in the process list.
fn post(runtime: &Runtime, endpoint: &str, body: &str, scratch_parent: &str) -> Result<(), String> {
    let scratch = runtime.files.temp_dir(scratch_parent, ".otlp-").map_err(fs_message)?;
    let path = format!("{scratch}/request.json");
    let sent = runtime.files.write_private(&path, body).map_err(fs_message).and_then(|()| {
        let mut config = format!("url = {}\nrequest = \"POST\"\nheader = \"Content-Type: application/json\"\ndata-binary = {}\n", quoted(endpoint), quoted(&format!("@{path}")));
        for pair in runtime.env.get("OTEL_EXPORTER_OTLP_HEADERS").map(String::as_str).unwrap_or_default().split(',').filter(|pair| pair.contains('=')) {
            let (key, value) = pair.split_once('=').unwrap_or_default();
            config.push_str(&format!("header = {}\n", quoted(&format!("{}: {}", key.trim(), value.trim()))));
        }
        let mut command = ProcessCommand::new("curl", &["--silent", "--show-error", "--fail", "--config", "-", "--output", "/dev/null"]);
        command.stdin = Some(config);
        command.timeout_ms = Some(POST_TIMEOUT_MS);
        let result = runtime.process.run(&command)?;
        if result.timed_out {
            Err("the collector did not answer in time".to_string())
        } else if result.exit_code != 0 {
            Err(format!("the collector refused the request: {}", result.stderr.trim()))
        } else {
            Ok(())
        }
    });
    let _ = runtime.files.remove(&scratch);
    sent
}

/// Exports the runs the options name; how many spans went out and where.
pub fn export(options: &ExportOptions, runtime: &Runtime) -> Result<Value, GatewayError> {
    let endpoint = endpoint_of(options, runtime);
    if endpoint.is_none() && options.file.is_none() {
        return Err(GatewayError::invalid_input(USAGE));
    }
    // The scratch folder of a post is made beside the runs, in the state folder, which the index lives in too.
    let scratch_parent = crate::locations::runs_dir(&runtime.env)?;
    let runs: Vec<IndexedRun> = indexed_runs(runtime)
        .unwrap_or_default()
        .into_iter()
        .filter(|run| options.run.as_deref().is_none_or(|key| run.run_id.starts_with(key)))
        .filter(|run| options.workflow.as_deref().is_none_or(|workflow| run.workflow == workflow))
        .collect();
    let spans: Vec<Value> = runs.iter().filter_map(|run| span_of(run, &read_events(runtime, &run.run_dir))).collect();
    let count = spans.len();
    let mut sent_to = Vec::new();
    if count > 0 {
        let body = json::compact(&request_of(spans));
        if let Some(path) = &options.file {
            let _ = runtime.files.mkdir_all(crate::text::dirname(path));
            runtime.files.append(path, &format!("{body}\n")).map_err(|error| GatewayError::new("export-failed", format!("cannot write {path}: {}", error.message)))?;
            sent_to.push(Value::str(path));
        }
        if let Some(endpoint) = &endpoint {
            post(runtime, endpoint, &body, &scratch_parent).map_err(|message| GatewayError::new("export-failed", format!("{endpoint}: {message}")))?;
            sent_to.push(Value::str(endpoint));
        }
    }
    Ok(json::object(vec![("status", Value::str("ok")), ("spans", Value::count(count)), ("sent_to", Value::Arr(sent_to))]))
}

/// Runs `export` over `args`; the exit code.
pub fn run(args: &[String], runtime: &Runtime, out: &mut dyn std::io::Write, err: &mut dyn std::io::Write) -> i32 {
    let Some(options) = parse_args(args) else {
        let _ = writeln!(err, "model-gateway export: {USAGE}");
        return 1;
    };
    match export(&options, runtime) {
        Ok(result) => {
            let _ = writeln!(out, "{}", json::pretty(&result));
            0
        }
        Err(error) => {
            let _ = writeln!(err, "model-gateway export: {}: {}", error.code, error.message);
            1
        }
    }
}
