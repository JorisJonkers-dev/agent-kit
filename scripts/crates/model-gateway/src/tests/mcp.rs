//! Tests for the run-state MCP server: it answers from the run index the command line reads, and writes nothing.

use super::machine::{Machine, DEFAULTS};
use crate::json::{parse, Value};
use crate::mcp::{handle, run, serve, DEFAULT_LINES};
use crate::run_events::{RunEvents, RunIdentity, RunStart};

fn start<'a>(machine: &'a Machine, id: &str, workflow: &str) -> RunEvents<'a> {
    let dir = format!("{}/state/agent-kit/model-gateway/runs/{id}", machine.sandbox.text());
    std::fs::create_dir_all(&dir).expect("run dir");
    let identity = RunIdentity { run_id: id.into(), workflow: workflow.into(), client: "pi".into(), model: "free/hy4-flash".into(), family: "tencent".into() };
    let index = crate::locations::run_index_path(&machine.runtime.env).expect("index path");
    RunEvents::start(machine.runtime.files.as_ref(), machine.runtime.clock.as_ref(), &dir, identity, &RunStart { mode: "read-only", time_limit_s: 900.0 }).indexed(Some(&index))
}

fn two_runs(machine: &Machine) {
    let first = start(machine, "r-one", "a");
    for n in 0..60 {
        first.diagnostic("stderr", "info", &format!("line {n}"));
    }
    first.finish(parse(r#"{"status":"ok","reason":null,"exit_code":0}"#).expect("JSON"));
    start(machine, "r-two", "b").finish(parse(r#"{"status":"failed","reason":"boom"}"#).expect("JSON"));
}

fn ask(machine: &Machine, request: &str) -> Value {
    parse(&handle(&machine.runtime, request).expect("a reply")).expect("JSON reply")
}

fn call(machine: &Machine, name: &str, arguments: &str) -> (bool, String) {
    let reply = ask(machine, &format!(r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"{name}","arguments":{arguments}}}}}"#));
    let result = reply.as_object().and_then(|reply| reply.get("result")).and_then(Value::as_object).cloned().expect("a result");
    let text = match result.get("content") {
        Some(Value::Arr(blocks)) => blocks[0].as_object().and_then(|block| block.get("text")).and_then(Value::as_str).unwrap_or_default().to_string(),
        _ => String::new(),
    };
    (result.get("isError") == Some(&Value::Bool(true)), text)
}

#[test]
fn it_introduces_itself_lists_four_read_only_tools_and_ignores_notifications() {
    let machine = Machine::new(DEFAULTS);
    let init = ask(&machine, r#"{"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#);
    let text = crate::json::compact(&init);
    assert!(text.contains(r#""id":"a""#) && text.contains("2025-03-26") && text.contains("model-gateway-run-state"), "{text}");
    assert!(crate::json::compact(&ask(&machine, r#"{"id":1,"method":"initialize"}"#)).contains("2024-11-05"));
    assert!(crate::json::compact(&ask(&machine, r#"{"id":2,"method":"ping"}"#)).contains(r#""result":{}"#));
    let tools = crate::json::compact(&ask(&machine, r#"{"id":3,"method":"tools/list"}"#));
    for name in ["ps", "status", "logs", "tail"] {
        assert!(tools.contains(&format!(r#""name":"{name}""#)), "{tools}");
    }
    for forbidden in ["cancel", "gc", "run\"", "batch"] {
        assert!(!tools.contains(&format!(r#""name":"{forbidden}"#)), "{forbidden} is not read-only");
    }
    assert!(handle(&machine.runtime, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
}

#[test]
fn bad_requests_get_the_json_rpc_errors() {
    let machine = Machine::new(DEFAULTS);
    for (request, code) in [("[1]", "-32600"), ("{nope", "-32700"), (r#"{"id":1}"#, "-32600"), (r#"{"id":1,"method":"nope"}"#, "-32601")] {
        let reply = crate::json::compact(&ask(&machine, request));
        assert!(reply.contains(code), "{request}: {reply}");
    }
}

#[test]
fn ps_status_and_logs_answer_what_the_command_line_prints() {
    let machine = Machine::new(DEFAULTS);
    two_runs(&machine);
    let (error, table) = call(&machine, "ps", "{}");
    assert!(!error && table.contains("r-one") && table.contains("r-two"), "{table}");
    let (_, table) = call(&machine, "ps", r#"{"workflow":"b","state":"failed","all":true,"limit":5,"ignored":1}"#);
    assert!(table.contains("r-two") && !table.contains("r-one"), "{table}");
    let (_, status) = call(&machine, "status", r#"{"run":"r-tw"}"#);
    assert!(status.contains("run_id: r-two") && status.contains("state: failed"), "{status}");
    let (_, logs) = call(&machine, "logs", r#"{"run":"r-one"}"#);
    assert_eq!(logs.lines().count(), DEFAULT_LINES);
    assert!(logs.contains("run.finished"), "the newest lines are kept");
    let (_, logs) = call(&machine, "logs", r#"{"run":"r-one","types":"run.finished","limit":999}"#);
    assert_eq!(logs.lines().count(), 1);
    let (_, logs) = call(&machine, "logs", r#"{"run":"r-one","limit":2}"#);
    assert_eq!(logs.lines().count(), 2);
    let (_, logs) = call(&machine, "logs", r#"{"run":"r-one","limit":0}"#);
    assert_eq!(logs.lines().count(), DEFAULT_LINES);
}

#[test]
fn tail_returns_the_latest_events_with_the_filters_and_never_follows() {
    let machine = Machine::new(DEFAULTS);
    two_runs(&machine);
    let (error, text) = call(&machine, "tail", r#"{"types":"run.finished","workflow":"a","run":"r-one","limit":3}"#);
    assert!(!error, "{text}");
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("r-one run.finished"), "{text}");
    let (_, text) = call(&machine, "tail", "{}");
    assert_eq!(text.lines().count(), crate::tail::LINES);
}

#[test]
fn a_failed_ask_is_a_tool_error_not_a_protocol_error() {
    let machine = Machine::new(DEFAULTS);
    let (error, text) = call(&machine, "status", r#"{"run":"nothing"}"#);
    assert!(error && text.contains("unknown-run"), "{text}");
    let (error, text) = call(&machine, "status", "{}");
    assert!(error && text.contains("invalid-input"), "{text}");
    let (error, text) = call(&machine, "cancel", r#"{"run":"r"}"#);
    assert!(error && text.contains("unknown tool: cancel"), "{text}");
    // The runs of a window that has none are an empty answer.
    let reply = ask(&machine, r#"{"id":1,"method":"tools/call"}"#);
    assert!(crate::json::compact(&reply).contains("unknown tool"));
}

#[test]
fn a_tail_that_the_command_refuses_is_a_tool_error() {
    let mut machine = Machine::new(DEFAULTS);
    machine.runtime.env.remove("HOME");
    machine.runtime.env.remove("XDG_STATE_HOME");
    let (error, text) = call(&machine, "tail", "{}");
    assert!(error && !text.is_empty(), "{text}");
}

#[test]
fn serve_answers_each_line_until_the_input_ends_and_run_refuses_arguments() {
    let machine = Machine::new(DEFAULTS);
    two_runs(&machine);
    let mut input = "\n{\"id\":1,\"method\":\"ping\"}\n{\"method\":\"notifications/initialized\"}\n{\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"ps\"}}\n".as_bytes();
    let mut out = Vec::new();
    assert_eq!(run(&[], &machine.runtime, &mut input, &mut out, &mut Vec::new()), 0);
    let lines: Vec<String> = String::from_utf8(out).expect("utf8").lines().map(str::to_string).collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[1].contains("r-one"), "{}", lines[1]);
    let mut err = Vec::new();
    assert_eq!(run(&["--x".to_string()], &machine.runtime, &mut "".as_bytes(), &mut Vec::new(), &mut err), 1);
    assert!(String::from_utf8(err).expect("utf8").contains("takes no arguments"));
}

struct Broken;
impl std::io::Read for Broken {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("gone"))
    }
}
impl std::io::Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("closed"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_broken_pipe_ends_the_server_with_a_message() {
    let machine = Machine::new(DEFAULTS);
    assert!(serve(&machine.runtime, &mut std::io::BufReader::new(Broken), &mut Vec::new()).expect_err("read fails").starts_with("read:"));
    let mut err = Vec::new();
    assert_eq!(run(&[], &machine.runtime, &mut std::io::BufReader::new(Broken), &mut Vec::new(), &mut err), 1);
    assert!(String::from_utf8(err).expect("utf8").contains("read: gone"));
    assert!(serve(&machine.runtime, &mut "{\"id\":1,\"method\":\"ping\"}\n".as_bytes(), &mut Broken).expect_err("write fails").starts_with("write:"));
}
