//! `mcp`: the run state as a read-only MCP server over stdio, newline-delimited JSON-RPC 2.0. Four tools,
//! `ps`, `status`, `logs` and `tail`, answer from the same run index and event logs the command line reads, so
//! an agent that asks gets what `model-gateway ps` would have printed. Nothing here starts, changes or cancels
//! a run, and no socket is opened.
//!
//! The server is off unless setup registers it (`AGENT_KIT_RUN_STATE_MCP=1`, registry/estate-tooling.yaml): a
//! client that does not list it never starts it, and no tool costs a token while it is not listed.

use crate::commands::{self, Command};
use crate::json::{self, Object, Value};
use crate::ports::Runtime;
use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// The most lines `logs` and `tail` return unless the caller asks for fewer.
pub const MAX_LINES: usize = 200;

/// What `logs` returns when no `limit` is given: the newest lines.
pub const DEFAULT_LINES: usize = 50;

const TOOL_LIST: &str = r#"{"tools":[{"name":"ps","description":"List the running and recent gateway runs, newest first: state, workflow, harness, model and age. Read-only; the same table as `model-gateway ps`.","inputSchema":{"type":"object","properties":{"workflow":{"type":"string","description":"Only the runs of this workflow"},"state":{"type":"string","description":"Only the runs in this state: running, stalled, ok, failed, cancelled or lost"},"all":{"type":"boolean","description":"Every run the index holds, not only the recent ones"},"limit":{"type":"integer","minimum":1,"description":"Cut the list to this many runs"}}}},{"name":"status","description":"Summarise one run: state, current step, last event, tokens, cost, heartbeat age and the route that served it. Read-only; the same lines as `model-gateway status`.","inputSchema":{"type":"object","properties":{"run":{"type":"string","description":"A run id, or the start of one"}},"required":["run"]}},{"name":"logs","description":"A run's events, one line each, newest last. Read-only; the same lines as `model-gateway logs`, capped to the newest `limit`.","inputSchema":{"type":"object","properties":{"run":{"type":"string","description":"A run id, or the start of one"},"types":{"type":"string","description":"Comma-separated event types to keep, such as stall,run.finished"},"limit":{"type":"integer","minimum":1,"maximum":200,"description":"The newest this many lines; 50 when omitted"}},"required":["run"]}},{"name":"tail","description":"The latest events of the runs of the last 24 hours, one line each. Read-only and never follows; the same lines as `model-gateway tail`.","inputSchema":{"type":"object","properties":{"run":{"type":"string","description":"Only this run (an id, or the start of one)"},"workflow":{"type":"string","description":"Only the runs of this workflow"},"types":{"type":"string","description":"Comma-separated event types to keep"},"limit":{"type":"integer","minimum":1,"maximum":200,"description":"The newest this many events; 20 when omitted"}}}}]}"#;

/// Serves requests from `input` until it ends.
pub fn serve(runtime: &Runtime, input: &mut dyn BufRead, out: &mut dyn Write) -> Result<(), String> {
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).map_err(|error| format!("read: {error}"))? == 0 {
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(runtime, line.trim()) {
            writeln!(out, "{reply}").and_then(|()| out.flush()).map_err(|error| format!("write: {error}"))?;
        }
    }
}

fn error(id: &str, code: i32, message: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":{code},"message":{}}}}}"#, json::compact(&Value::str(message)))
}

/// The reply to one request line, or `None` for a notification.
pub fn handle(runtime: &Runtime, line: &str) -> Option<String> {
    let request = match json::parse(line) {
        Ok(Value::Obj(request)) => request,
        Ok(_) => return Some(error("null", -32600, "request must be an object")),
        Err(message) => return Some(error("null", -32700, &format!("parse error: {message}"))),
    };
    let id = json::compact(request.get("id")?);
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return Some(error(&id, -32600, "missing method"));
    };
    let params = request.get("params").and_then(Value::as_object);
    let result = match method {
        "initialize" => {
            let version = params.and_then(|params| params.get("protocolVersion")).and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
            format!(r#"{{"protocolVersion":{},"capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"model-gateway-run-state","version":"{}"}}}}"#, json::compact(&Value::str(version)), env!("CARGO_PKG_VERSION"))
        }
        "ping" => "{}".to_string(),
        "tools/list" => TOOL_LIST.to_string(),
        "tools/call" => call(runtime, params),
        other => return Some(error(&id, -32601, &format!("method not found: {other}"))),
    };
    Some(format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#))
}

/// A tool result carrying one text block.
fn text_result(text: &str, is_error: bool) -> String {
    format!(r#"{{"content":[{{"type":"text","text":{}}}],"isError":{is_error}}}"#, json::compact(&Value::str(text)))
}

fn call(runtime: &Runtime, params: Option<&Object>) -> String {
    let name = params.and_then(|params| params.get("name")).and_then(Value::as_str).unwrap_or_default();
    let empty = Object::new();
    let args = params.and_then(|params| params.get("arguments")).and_then(Value::as_object).unwrap_or(&empty);
    match tool(runtime, name, args) {
        Ok(text) => text_result(&text, false),
        Err(message) => text_result(&message, true),
    }
}

/// The newest `limit` lines of `text`.
fn newest(text: &str, limit: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(limit)..].iter().map(|line| format!("{line}\n")).collect()
}

/// The `limit` argument as a count of lines, capped; `default` when absent.
fn limit_of(args: &Object, default: usize) -> usize {
    match args.get("limit") {
        Some(Value::Num(number)) if *number >= 1.0 => (*number as usize).min(MAX_LINES),
        _ => default,
    }
}

/// The arguments the tool takes, as the input of the command that answers it. Anything else is ignored.
fn input_of(args: &Object, keys: &[&str]) -> Object {
    let mut input = Object::new();
    for key in keys {
        if let Some(value) = args.get(key) {
            input.set(key, value.clone());
        }
    }
    input
}

fn tool(runtime: &Runtime, name: &str, args: &Object) -> Result<String, String> {
    let ask = |command: Command, keys: &[&str]| commands::run(command, name, &input_of(args, keys), runtime).map_err(|error| format!("{}: {}", error.code, error.message));
    match name {
        "ps" => Ok(crate::run_status::ps_table(&ask(Command::Ps, &["workflow", "state", "all", "limit"])?)),
        "status" => Ok(crate::run_status::status_text(&ask(Command::Status, &["run"])?)),
        "logs" => Ok(newest(&crate::run_status::logs_text(&ask(Command::Logs, &["run", "types"])?), limit_of(args, DEFAULT_LINES))),
        "tail" => {
            let mut argv = vec!["-n".to_string(), limit_of(args, crate::tail::LINES).to_string()];
            for (key, flag) in [("run", "--run"), ("workflow", "--workflow"), ("types", "--types")] {
                if let Some(Value::Str(value)) = args.get(key) {
                    argv.extend([flag.to_string(), value.clone()]);
                }
            }
            let (mut out, mut err) = (Vec::new(), Vec::new());
            match crate::tail::run(&argv, runtime, &mut out, &mut err) {
                0 => Ok(String::from_utf8_lossy(&out).into_owned()),
                _ => Err(String::from_utf8_lossy(&err).trim().to_string()),
            }
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

/// Runs `mcp` over `args`, serving on `input` and `out`; the exit code.
pub fn run(args: &[String], runtime: &Runtime, input: &mut dyn BufRead, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if !args.is_empty() {
        let _ = writeln!(err, "model-gateway mcp: takes no arguments; it serves the MCP protocol on stdin and stdout");
        return 1;
    }
    match serve(runtime, input, out) {
        Ok(()) => 0,
        Err(message) => {
            let _ = writeln!(err, "model-gateway mcp: {message}");
            1
        }
    }
}
