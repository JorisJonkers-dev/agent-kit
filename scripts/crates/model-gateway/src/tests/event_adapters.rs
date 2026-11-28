//! Tests for the event adapters: each harness's recorded stream reads as the events its golden file holds, the
//! usage in those events is the spend record's, and a line no adapter knows never fails anything.
//!
//! The fixtures in `event_fixtures` are streams recorded from real runs of each harness (paths and secrets
//! stripped, long text cut), except `codex`, which is written from Codex's documented `exec --json` format
//! because no Codex run was permitted on the machine the others were recorded on. `BLESS=1` rewrites the goldens.

use crate::event_adapters::{for_client, transcript, AdapterEvent};
use crate::json::{self, Value};
use crate::outputs::{agy_output, claude_output, codex_output, hermes_output, open_code_output, pi_output};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/src/tests/event_fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// Every event a client's adapter reads from a stream, as the compact line the golden holds.
fn read(client: &str, stdout: &str) -> Vec<AdapterEvent> {
    let mut adapter = for_client(client).expect("an adapter");
    let mut events: Vec<AdapterEvent> = stdout.lines().filter(|line| !line.trim().is_empty()).flat_map(|line| adapter.line(line)).collect();
    events.extend(adapter.finish());
    events
}

fn rendered(events: &[AdapterEvent]) -> String {
    events.iter().map(|event| format!("{}\n", json::compact(&json::object(vec![("type", Value::str(event.kind)), ("data", event.data.clone())])))).collect()
}

fn golden(client: &str, name: &str) -> Vec<AdapterEvent> {
    let events = read(client, &fixture(&format!("{name}.stdout")));
    let path = format!("{}/src/tests/event_fixtures/{name}.events", env!("CARGO_MANIFEST_DIR"));
    if std::env::var("BLESS").is_ok() {
        std::fs::write(&path, rendered(&events)).expect("bless");
    }
    assert_eq!(rendered(&events), fixture(&format!("{name}.events")), "{name}: the events differ from the golden (BLESS=1 rewrites it)");
    events
}

fn kinds(events: &[AdapterEvent]) -> Vec<&'static str> {
    events.iter().map(|event| event.kind).collect()
}

fn num(event: &AdapterEvent, key: &str) -> f64 {
    match event.data.as_object().and_then(|data| data.get(key)) {
        Some(Value::Num(number)) => *number,
        _ => 0.0,
    }
}

/// The tokens and dollars the usage events of a stream add up to: input, output, cached, cost.
fn usage_sum(events: &[AdapterEvent]) -> (f64, f64, f64, f64) {
    events.iter().filter(|event| event.kind == "usage").fold((0.0, 0.0, 0.0, 0.0), |sum, event| {
        (sum.0 + num(event, "gen_ai.usage.input_tokens"), sum.1 + num(event, "gen_ai.usage.output_tokens"), sum.2 + num(event, "cached_input_tokens"), sum.3 + num(event, "cost_usd"))
    })
}

fn recorded(output: &crate::outputs::ClientOutput) -> (f64, f64, f64, f64) {
    (output.tokens.input, output.tokens.output, output.tokens.cached, output.charged.unwrap_or(0.0))
}

fn close(left: (f64, f64, f64, f64), right: (f64, f64, f64, f64)) {
    assert!((left.0 - right.0).abs() < 1e-9 && (left.1 - right.1).abs() < 1e-9 && (left.2 - right.2).abs() < 1e-9 && (left.3 - right.3).abs() < 1e-6, "{left:?} against {right:?}");
}

#[test]
fn a_claude_stream_is_its_messages_tool_calls_and_totals() {
    let events = golden("claude", "claude");
    assert_eq!(kinds(&events), ["tool.started", "tool.finished", "message", "usage"]);
    close(usage_sum(&events), recorded(&claude_output(&fixture("claude.stdout"))));
    assert!(usage_sum(&events).3 > 0.0);
}

#[test]
fn a_claude_run_that_hit_its_limit_reports_the_error_beside_its_usage() {
    let events = golden("claude", "claude-limit");
    assert_eq!(kinds(&events), ["usage", "diagnostic"]);
    close(usage_sum(&events), recorded(&claude_output(&fixture("claude-limit.stdout"))));
}

#[test]
fn an_opencode_stream_is_its_text_tool_calls_and_steps() {
    let events = golden("opencode", "opencode");
    assert_eq!(kinds(&events), ["tool.started", "tool.finished", "usage", "message", "usage"]);
    close(usage_sum(&events), recorded(&open_code_output(&fixture("opencode.stdout"))));
}

#[test]
fn a_pi_stream_is_its_messages_and_tool_executions_not_its_deltas() {
    let events = golden("pi-free", "pi");
    assert_eq!(kinds(&events), ["usage", "tool.started", "tool.finished", "message", "usage"]);
    close(usage_sum(&events), recorded(&pi_output(&fixture("pi.stdout"))));
}

#[test]
fn a_vibe_stream_is_its_tool_effects_and_answer_and_has_no_usage_of_its_own() {
    let events = golden("vibe", "vibe");
    assert_eq!(kinds(&events), ["tool.started", "tool.finished", "message"]);
    assert_eq!(crate::outputs::vibe_output(&fixture("vibe.stdout"), None).answer.as_deref(), Some("The first heading in **AGENTS.md** is:\n\n**Build and test**"));
}

#[test]
fn an_agy_stream_is_its_tool_steps_answer_and_totals() {
    let events = golden("agy", "agy");
    assert_eq!(kinds(&events), ["tool.started", "tool.finished", "message", "usage"]);
    close(usage_sum(&events), recorded(&agy_output(&fixture("agy.stdout"))));
}

#[test]
fn a_codex_stream_is_its_items_and_turns() {
    let events = golden("codex", "codex");
    assert_eq!(kinds(&events), ["message", "tool.started", "tool.finished", "tool.started", "tool.finished", "plan", "diagnostic", "message", "usage", "diagnostic"]);
    close(usage_sum(&events), recorded(&codex_output(&fixture("codex.stdout"))));
}

/// What the gateway writes to Hermes while it reads the recorded answers, one at a time, in the order a run
/// goes: the handshake, the session, the prompt.
#[test]
fn a_hermes_run_over_acp_is_driven_to_its_answer_and_read_into_events() {
    let mut adapter = for_client("hermes").expect("an adapter");
    assert!(adapter.drives());
    adapter.drive("read AGENTS.md", "/work");
    let opening = adapter.opening();
    assert!(opening.contains("\"method\":\"initialize\"") && opening.ends_with('\n'));
    let stdout = fixture("hermes.stdout");
    let mut events = Vec::new();
    let mut sent = String::new();
    for line in stdout.lines() {
        events.extend(adapter.line(line));
        let out = adapter.outgoing();
        sent.push_str(&out.text);
        assert_eq!(out.close, line.contains("\"stopReason\""), "{line}");
    }
    assert!(sent.contains("\"method\":\"session/new\"") && sent.contains("\"cwd\":\"/work\""));
    assert!(sent.contains("\"method\":\"session/prompt\"") && sent.contains("\"sessionId\":\"sess-1\"") && sent.contains("read AGENTS.md"));
    let path = format!("{}/src/tests/event_fixtures/hermes.events", env!("CARGO_MANIFEST_DIR"));
    if std::env::var("BLESS").is_ok() {
        std::fs::write(&path, rendered(&events)).expect("bless");
    }
    assert_eq!(rendered(&events), fixture("hermes.events"));
    assert_eq!(kinds(&events), ["message", "tool.started", "message", "usage"]);
    let run = transcript(&stdout).expect("an ACP run");
    assert_eq!(run.error, None);
    let counted = hermes_output(&stdout, None);
    close(usage_sum(&events), recorded(&counted));
    assert_eq!(counted.answer.as_deref(), Some("# Agents"));
}

#[test]
fn a_line_no_adapter_knows_is_a_diagnostic_and_never_a_failure() {
    for client in ["claude", "codex", "opencode", "pi", "pi-free", "vibe", "agy", "hermes"] {
        let mut adapter = for_client(client).expect("an adapter");
        for line in ["not json at all", "[1,2]", "{\"type\":\"a-type-nobody-wrote\"}", "{\"no\":\"type\"}"] {
            let events = adapter.line(line);
            assert_eq!(kinds(&events), ["diagnostic"], "{client}: {line}");
            let data = events[0].data.as_object().expect("data");
            assert_eq!((data.get("source").and_then(Value::as_str), data.get("level").and_then(Value::as_str)), (Some("adapter"), Some("info")));
        }
        assert!(adapter.finish().is_empty());
    }
    assert!(for_client("a-client-with-no-stream").is_none());
}

#[test]
fn a_harness_that_is_only_read_is_not_driven() {
    let mut adapter = for_client("opencode").expect("an adapter");
    adapter.drive("prompt", "/work");
    assert_eq!((adapter.drives(), adapter.opening(), adapter.outgoing().close), (false, String::new(), false));
}

fn lines(text: &[&str]) -> Vec<AdapterEvent> {
    let mut adapter = for_client("claude").expect("an adapter");
    text.iter().flat_map(|line| adapter.line(line)).collect()
}

#[test]
fn claude_blocks_it_has_no_event_for_are_left_out_and_errors_are_kept() {
    let events = lines(&[
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hmm"},{"type":"image"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"ignored"},{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"denied"}]},{"type":"tool_result","tool_use_id":"t2","content":42}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"a user text"}]}}"#,
    ]);
    assert_eq!(kinds(&events), ["message", "tool.started", "tool.finished", "tool.finished"]);
    let finished = events[2].data.as_object().expect("data");
    assert_eq!((finished.get("status").and_then(Value::as_str), finished.get("gen_ai.tool.name").and_then(Value::as_str), finished.get("output").and_then(Value::as_str)), (Some("error"), Some("Bash"), Some("denied")));
    let unknown = events[3].data.as_object().expect("data");
    assert_eq!((unknown.get("gen_ai.tool.name").and_then(Value::as_str), unknown.get("output").and_then(Value::as_str)), (Some(""), Some("")));
}

#[test]
fn codex_items_in_every_phase_and_every_failure_are_read() {
    let mut adapter = for_client("codex").expect("an adapter");
    let mut events = Vec::new();
    for line in [
        r#"{"type":"thread.started","thread_id":"x"}"#,
        r#"{"type":"turn.started"}"#,
        r#"{"type":"item.started","item":{"id":"i1","type":"mcp_tool_call","server":"docs","tool":"search","status":"in_progress"}}"#,
        r#"{"type":"item.updated","item":{"id":"i1","type":"mcp_tool_call"}}"#,
        r#"{"type":"item.completed","item":{"id":"i1","type":"mcp_tool_call","server":"docs","tool":"search","status":"failed"}}"#,
        r#"{"type":"item.completed","item":{"id":"i2","type":"command_execution","command":"false","exit_code":1,"aggregated_output":"no"}}"#,
        r#"{"type":"item.completed","item":{"id":"i3","type":"command_execution","command":"true","exit_code":null,"aggregated_output":""}}"#,
        r#"{"type":"item.started","item":{"id":"i4","type":"agent_message"}}"#,
        r#"{"type":"item.completed","item":{"id":"i5","type":"reasoning","text":"thinking"}}"#,
        r#"{"type":"item.completed","item":{"id":"i6","type":"error","message":"item failed"}}"#,
        r#"{"type":"item.completed","item":{"id":"i7","type":"an-item-nobody-wrote"}}"#,
        r#"{"type":"item.completed"}"#,
        r#"{"type":"turn.failed","error":{"message":"turn broke"}}"#,
        r#"{"type":"turn.failed"}"#,
        r#"{"type":"error"}"#,
    ] {
        events.extend(adapter.line(line));
    }
    assert_eq!(kinds(&events), ["tool.started", "tool.finished", "tool.finished", "tool.finished", "message", "diagnostic", "diagnostic", "diagnostic", "diagnostic", "diagnostic", "diagnostic"]);
    let status = |at: usize| events[at].data.as_object().and_then(|data| data.get("status")).and_then(Value::as_str).map(str::to_string);
    assert_eq!((status(1), status(2), status(3)), (Some("error".to_string()), Some("error".to_string()), Some("ok".to_string())));
    assert_eq!(events[0].data.as_object().and_then(|data| data.get("gen_ai.tool.name")).and_then(Value::as_str), Some("docs.search"));
}

#[test]
fn opencode_tools_that_have_not_ended_and_errors_are_read() {
    let mut adapter = for_client("opencode").expect("an adapter");
    let mut events = Vec::new();
    for line in [
        r#"{"type":"tool_use","part":{"callID":"c1","tool":"bash","state":{"status":"running","input":{"command":"ls"}}}}"#,
        r#"{"type":"tool_use","part":{"callID":"c2","tool":"bash","state":{"status":"error","error":"boom"}}}"#,
        r#"{"type":"reasoning","part":{"text":"thinking"}}"#,
        r#"{"type":"error","error":{"name":"ProviderError","data":{"message":"rate limited"}}}"#,
        r#"{"type":"error","error":{"name":"UnknownError"}}"#,
        r#"{"type":"error"}"#,
    ] {
        events.extend(adapter.line(line));
    }
    assert_eq!(kinds(&events), ["tool.started", "tool.started", "tool.finished", "message", "diagnostic", "diagnostic", "diagnostic"]);
}

#[test]
fn pi_messages_that_stopped_with_an_error_and_failed_tools_are_read() {
    let mut adapter = for_client("pi").expect("an adapter");
    let mut events = Vec::new();
    for line in [
        r#"{"type":"message_end","message":{"role":"user","content":[]}}"#,
        r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"},{"type":"toolCall"}],"usage":{"input":10,"cacheRead":5,"output":3,"cost":{"total":0.5}},"stopReason":"error","errorMessage":"provider said no"}}"#,
        r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"aborted"}}"#,
        r#"{"type":"tool_execution_end","toolCallId":"t1","toolName":"read","isError":true,"result":{"content":[{"type":"text","text":"missing"}]}}"#,
    ] {
        events.extend(adapter.line(line));
    }
    assert_eq!(kinds(&events), ["message", "usage", "diagnostic", "usage", "diagnostic", "tool.finished"]);
    close(usage_sum(&events), (15.0, 3.0, 5.0, 0.5));
}

#[test]
fn vibe_effects_that_are_running_or_failed_and_other_entries_are_read() {
    let mut adapter = for_client("vibe").expect("an adapter");
    let mut events = Vec::new();
    for line in [
        r#"{"type":"message","role":"user","content":[]}"#,
        r#"{"type":"effect","id":"e1","title":"run","detail":{"input":{}},"state":{"status":"running"}}"#,
        r#"{"type":"effect","toolCallId":"e2","detail":{"toolName":"bash"},"state":{"status":"failed","display":{"message":"exit 1"}}}"#,
        r#"{"type":"journal-note"}"#,
    ] {
        events.extend(adapter.line(line));
    }
    assert_eq!(kinds(&events), ["tool.started", "tool.started", "tool.finished", "diagnostic"]);
}

#[test]
fn agy_steps_of_every_kind_and_a_failed_result_are_read() {
    let mut adapter = for_client("agy").expect("an adapter");
    let mut events = Vec::new();
    for line in [
        r#"{"event":"init","init":{}}"#,
        r#"{"event":"step_update","step_update":{"step_type":"user_input","state":"DONE"}}"#,
        r#"{"event":"step_update","step_update":{"step_type":"agent_response","state":"ACTIVE","text_delta":"Hel"}}"#,
        r#"{"event":"step_update","step_update":{"step_type":"agent_response","state":"DONE","text_delta":"lo"}}"#,
        r#"{"event":"step_update","step_update":{"step_type":"a-step-nobody-wrote","state":"DONE"}}"#,
        r#"{"event":"step_update","step_update":{}}"#,
        r#"{"event":"result","result":{"status":"TIMEOUT"}}"#,
        r#"{"event":"result"}"#,
        r#"{"event":"a-new-event"}"#,
    ] {
        events.extend(adapter.line(line));
    }
    assert_eq!(kinds(&events), ["message", "diagnostic", "diagnostic", "usage", "diagnostic", "usage", "diagnostic", "diagnostic"]);
    assert_eq!(events[0].data.as_object().and_then(|data| data.get("text")).and_then(Value::as_str), Some("Hello"));
}

/// Feeds an ACP adapter the agent's lines one at a time and gives back its events and what it wrote.
fn acp(lines: &[&str]) -> (Vec<AdapterEvent>, crate::event_adapters::Reply) {
    let mut adapter = for_client("hermes").expect("an adapter");
    let mut events = Vec::new();
    let mut reply = crate::event_adapters::Reply::default();
    for line in lines {
        events.extend(adapter.line(line));
        let out = adapter.outgoing();
        reply.text.push_str(&out.text);
        reply.close |= out.close;
    }
    events.extend(adapter.finish());
    (events, reply)
}

#[test]
fn acp_requests_of_the_agent_are_answered_and_its_odd_lines_are_diagnostics() {
    let (events, reply) = acp(&[
        r#"{"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"options":[{"kind":"allow_once","optionId":"a"},{"kind":"reject_once","optionId":"r"}]}}"#,
        r#"{"jsonrpc":"2.0","id":8,"method":"session/request_permission","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":9,"method":"fs/read_text_file","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"some/notification"}"#,
        r#"{"jsonrpc":"2.0","result":{}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{}}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"a_new_update"}}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"usage_update"}}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"config_option_update"}}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"x","status":"in_progress"}}}"#,
        r#"{"jsonrpc":"2.0","id":99,"result":{}}"#,
    ]);
    assert_eq!(kinds(&events), ["diagnostic", "diagnostic", "diagnostic", "diagnostic"]);
    assert!(reply.text.contains("\"optionId\":\"r\"") && reply.text.contains("\"outcome\":\"cancelled\"") && reply.text.contains("-32601"));
    assert!(!reply.close);
}

#[test]
fn acp_messages_are_joined_from_chunks_and_plans_and_tool_updates_are_read() {
    let (events, reply) = acp(&[
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"text":"think "}}}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"text":"hard"}}}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"so"}}}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"plan","entries":[{"content":"read","status":"in_progress"},{"content":"answer"}]}}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"t1","kind":"read","rawInput":{"path":"a"}}}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"failed","content":[{"type":"content","content":{"type":"text","text":"gone"}}]}}}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"done"}}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"max_tokens"}}"#,
    ]);
    assert_eq!(kinds(&events), ["message", "message", "plan", "tool.started", "tool.finished", "message", "usage", "diagnostic"]);
    assert!(reply.close);
}

#[test]
fn an_acp_error_answer_ends_the_conversation_with_the_agents_words() {
    let (events, reply) = acp(&[r#"{"jsonrpc":"2.0","id":2,"error":{"code":-1,"message":"no such directory"}}"#, r#"{"jsonrpc":"2.0","id":1,"error":{}}"#]);
    assert_eq!(kinds(&events), ["diagnostic", "diagnostic"]);
    assert!(reply.close);
    let run = transcript("{\"jsonrpc\":\"2.0\",\"id\":2,\"error\":{\"message\":\"no such directory\"}}\n").expect("an ACP run");
    assert_eq!(run.error.as_deref(), Some("no such directory"));
    assert_eq!(transcript("{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{}}\n").expect("an ACP run").error.as_deref(), Some("the agent answered with an error"));
}

#[test]
fn an_acp_transcript_takes_the_text_after_the_last_tool_call_and_notices_a_prompt_never_answered() {
    let run = transcript("{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"text\":\"before\"}}}}\n{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"tool_call\"}}}\n{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"text\":\" after \"}}}}\nnot json\n").expect("an ACP run");
    assert_eq!((run.answer.as_deref(), run.error.as_deref()), (Some("after"), Some("the agent never answered the prompt")));
    let stopped = transcript("{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"stopReason\":\"refusal\"}}\n").expect("an ACP run");
    assert_eq!((stopped.answer, stopped.error.as_deref()), (None, Some("the prompt ended with stop reason refusal")));
    assert!(transcript("plain text from a one-shot run\n").is_none());
}

mod in_a_run {
    use super::fixture;
    use crate::adapters::fs::SystemFiles;
    use crate::adapters::process::SystemProcess;
    use crate::json::{self, parse, Value};
    use crate::ports::{LaunchWatch, ProcessCommand, Process, Stream};
    use crate::run_events::{RunEvents, RunIdentity, RunStart, EVENTS_FILE};
    use crate::tests::scenarios::{FixedClock, Sandbox, EXEC_LOCK};
    use std::collections::HashMap;

    fn events_of(sandbox: &Sandbox) -> Vec<Value> {
        std::fs::read_to_string(sandbox.path.join("r1").join(EVENTS_FILE)).unwrap_or_default().lines().map(|line| parse(line).expect("a JSON line")).collect()
    }

    fn kinds(lines: &[Value]) -> Vec<String> {
        lines.iter().filter_map(|line| line.as_object().and_then(|line| line.get("type")).and_then(Value::as_str)).map(str::to_string).collect()
    }

    fn started(sandbox: &Sandbox, client: &str) -> (SystemFiles, FixedClock, RunIdentity) {
        std::fs::create_dir(sandbox.path.join("r1")).expect("run dir");
        (SystemFiles { cwd: sandbox.text() }, FixedClock(1_795_000_000_000), RunIdentity { run_id: "r1".into(), workflow: "w".into(), client: client.into(), model: "m".into(), family: "f".into() })
    }

    fn start() -> RunStart<'static> {
        RunStart { mode: "read-only", time_limit_s: 60.0 }
    }

    #[test]
    fn a_runs_log_gets_the_messages_and_tools_as_the_output_arrives_even_split_mid_line() {
        let sandbox = Sandbox::new();
        let (files, clock, identity) = started(&sandbox, "claude");
        let run_dir = format!("{}/r1", sandbox.text());
        let events = RunEvents::start(&files, &clock, &run_dir, identity, &start());
        let stdout = fixture("claude.stdout");
        let (first, rest) = stdout.split_at(stdout.len() / 2);
        events.output(Stream::Stdout, first.as_bytes());
        events.output(Stream::Stdout, rest.trim_end().as_bytes());
        events.output(Stream::Stdout, b"\n\nnot json\n");
        let before = kinds(&events_of(&sandbox));
        assert!(before.contains(&"tool.started".to_string()) && before.contains(&"usage".to_string()), "{before:?}");
        // The unended last line is read when the run finishes; the usage came from the stream, so none is added.
        events.output(Stream::Stdout, b"{\"type\":\"result\"");
        events.finish(json::object(vec![("status", Value::str("ok")), ("gen_ai.usage.input_tokens", Value::Num(5.0)), ("gen_ai.usage.output_tokens", Value::Num(1.0)), ("cost_usd", Value::Num(0.5))]));
        let all = kinds(&events_of(&sandbox));
        assert_eq!(all.iter().filter(|kind| *kind == "usage").count(), 1);
        assert_eq!(all.iter().filter(|kind| *kind == "diagnostic").count(), 2);
        assert_eq!(all.last().map(String::as_str), Some("run.finished"));
    }

    #[test]
    fn a_harness_that_reports_no_usage_has_the_results_written_before_the_finish() {
        let sandbox = Sandbox::new();
        let (files, clock, identity) = started(&sandbox, "vibe");
        let run_dir = format!("{}/r1", sandbox.text());
        let events = RunEvents::start(&files, &clock, &run_dir, identity, &start());
        events.output(Stream::Stdout, fixture("vibe.stdout").as_bytes());
        events.finish(json::object(vec![
            ("status", Value::str("ok")),
            ("gen_ai.usage.input_tokens", Value::Num(100.0)),
            ("gen_ai.usage.output_tokens", Value::Num(10.0)),
            ("cached_input_tokens", Value::Num(40.0)),
            ("cost_usd", Value::Num(0.0)),
        ]));
        let lines = events_of(&sandbox);
        let kinds = kinds(&lines);
        assert_eq!(&kinds[kinds.len() - 2..], ["usage", "run.finished"]);
        let usage = lines[lines.len() - 2].as_object().and_then(|line| line.get("data")).and_then(Value::as_object).expect("data");
        assert_eq!((usage.get("cached_input_tokens"), usage.get("cost_usd")), (Some(&Value::Num(40.0)), Some(&Value::Null)));
    }

    #[test]
    fn a_run_with_no_usage_and_a_client_with_no_adapter_write_nothing_extra() {
        let sandbox = Sandbox::new();
        let (files, clock, identity) = started(&sandbox, "vibe");
        let run_dir = format!("{}/r1", sandbox.text());
        let events = RunEvents::start(&files, &clock, &run_dir, identity, &start());
        events.finish(json::object(vec![("status", Value::str("failed"))]));
        assert_eq!(kinds(&events_of(&sandbox)), ["run.started", "run.finished"]);
        let plain = Sandbox::new();
        let (files, clock, identity) = started(&plain, "a-client-with-no-stream");
        let run_dir = format!("{}/r1", plain.text());
        let events = RunEvents::start(&files, &clock, &run_dir, identity, &start());
        events.output(Stream::Stdout, b"{\"type\":\"text\"}\n");
        events.drive("prompt", "/work");
        assert!(!events.interactive());
        assert_eq!((events.opening(), events.reply(b"x").text), (String::new(), String::new()));
        events.finish(json::object(vec![("gen_ai.usage.input_tokens", Value::Num(5.0))]));
        assert_eq!(kinds(&events_of(&plain)), ["run.started", "run.finished"]);
    }

    /// A stand-in for `hermes acp`: answers the three requests of a run, then waits for its stdin to close.
    const FAKE_HERMES: &str = r#"read a; echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}'
read b; echo '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}'
read c; echo "$c" > prompt.seen
echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"t1","title":"read: a","kind":"read"}}}'
echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed"}}}'
echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"hi"}}}}'
echo '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn","usage":{"inputTokens":7,"outputTokens":2}}}'
cat > /dev/null; echo closed > closed.seen"#;

    #[test]
    fn hermes_is_driven_over_acp_through_a_real_process_until_its_prompt_is_answered() {
        let _guard = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let sandbox = Sandbox::new();
        let (files, clock, identity) = started(&sandbox, "hermes");
        let run_dir = format!("{}/r1", sandbox.text());
        let events = RunEvents::start(&files, &clock, &run_dir, identity, &start());
        events.drive("say hi", &sandbox.text());
        assert!(events.interactive());
        let process = SystemProcess { env: HashMap::from([("PATH".to_string(), std::env::var("PATH").unwrap_or_default())]) };
        let command = ProcessCommand { cwd: Some(sandbox.text()), timeout_ms: Some(30_000), ..ProcessCommand::new("sh", &["-c", FAKE_HERMES]) };
        let launched = process.run_watched(&command, &events).expect("the stand-in runs");
        assert_eq!(launched.result.exit_code, 0);
        let kinds = kinds(&events_of(&sandbox));
        assert_eq!(kinds, ["run.started", "tool.started", "tool.finished", "message", "usage"]);
        let prompt = std::fs::read_to_string(sandbox.path.join("prompt.seen")).expect("the prompt reached it");
        assert!(prompt.contains("\"method\":\"session/prompt\"") && prompt.contains("\"sessionId\":\"s1\"") && prompt.contains("say hi"), "{prompt}");
        assert_eq!(std::fs::read_to_string(sandbox.path.join("closed.seen")).expect("stdin was closed").trim(), "closed");
        let out = crate::outputs::hermes_output(&launched.result.stdout, None);
        assert_eq!((out.answer.as_deref(), out.tokens.input, out.tokens.output, out.failure), (Some("hi"), 7.0, 2.0, None));
    }

    /// A watch that says it is interactive and leaves the rest to its defaults.
    struct Plain;

    impl LaunchWatch for Plain {
        fn spawned(&self, _pid: u32) {}
        fn output(&self, _stream: Stream, _chunk: &[u8]) {}
        fn note(&self, _text: &str) {}
        fn interactive(&self) -> bool {
            true
        }
    }

    #[test]
    fn an_interactive_watch_with_nothing_to_say_leaves_the_pipe_open_until_the_program_ends() {
        let _guard = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let process = SystemProcess { env: HashMap::from([("PATH".to_string(), std::env::var("PATH").unwrap_or_default())]) };
        let launched = process.run_watched(&ProcessCommand::new("sh", &["-c", "echo one; sleep 0.2; echo two"]), &Plain).expect("runs");
        assert_eq!(launched.result.stdout, "one\ntwo\n");
    }
}

#[test]
fn hermes_info_logs_are_not_an_error_channel_and_every_other_clients_stderr_is() {
    let stderr = "2026-11-28 10:19:36 [INFO] hermes_cli.model_catalog: fetch failed: HTTP Error 403: Forbidden\nreal error: rate limit\n";
    assert_eq!(crate::event_adapters::error_channel("hermes", stderr), "real error: rate limit\n");
    assert_eq!(crate::event_adapters::error_channel("opencode", stderr), stderr);
}
