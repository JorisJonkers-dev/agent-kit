//! Tests for what each client's own output says about a run, ported from the
//! node runner's tests of the same module.

use crate::json::{parse, Value};
use crate::outputs::*;
use crate::run::{ClientError, RunTokens, NO_TOKENS};

fn v(text: &str) -> Value {
    parse(text).expect("the test's JSON")
}

fn tokens(cached: f64, input: f64, output: f64) -> RunTokens {
    RunTokens { cached, input, output }
}

/// An output that holds nothing but what the arguments name.
fn output(answer: Option<&str>, charged: Option<f64>, failure: Option<&str>, tokens: RunTokens) -> ClientOutput {
    ClientOutput { answer: answer.map(str::to_string), charged, error: None, failure: failure.map(str::to_string), tokens }
}

fn lines(events: &[&str]) -> String {
    events.join("\n")
}

#[test]
fn vibe_takes_the_answer_from_the_journal_and_the_tokens_and_dollars_from_the_export() {
    let journal = r#"[{"content":[{"text":"done"}],"role":"assistant","type":"message"}]"#;
    let exported = v(r#"{"cost_usd":0.5,"usage":{"cached_input_tokens":1,"input_tokens":2,"output_tokens":3}}"#);
    assert_eq!(vibe_output(journal, Some(&exported)), output(Some("done"), Some(0.5), None, tokens(1.0, 2.0, 3.0)));
}

#[test]
fn vibe_has_no_tokens_and_no_dollars_without_an_export() {
    assert_eq!(vibe_output("not a journal", None), output(None, None, None, NO_TOKENS));
}

#[test]
fn vibe_carries_the_error_its_export_holds() {
    let exported = v(r#"{"error":{"message":"slow down","type":"rate_limit","code":"429","name":"E","status":429},"stop_reason":"stop"}"#);
    assert_eq!(
        vibe_output("", Some(&exported)).error,
        Some(ClientError { text: "slow down rate_limit 429 E stop".to_string(), status: Some(429.0) })
    );
}

#[test]
fn a_structured_error_reads_a_string_error_a_stop_reason_in_either_spelling_and_an_error_result() {
    let error = |exported: &str| vibe_output("", Some(&v(exported))).error;
    let said = |text: &str, status: Option<f64>| Some(ClientError { text: text.to_string(), status });
    assert_eq!(error(r#"{"error":"boom"}"#), said("boom", None));
    assert_eq!(error(r#"{"stopReason":"length"}"#), said("length", None));
    assert_eq!(error(r#"{"stop_reason":null,"stopReason":"length"}"#), said("length", None));
    assert_eq!(error(r#"{"stop_reason":"a","stopReason":"b"}"#), said("a", None));
    assert_eq!(error(r#"{"is_error":true,"result":"bad key"}"#), said("bad key", None));
    assert_eq!(error(r#"{"isError":true,"result":"bad key"}"#), said("bad key", None));
    assert_eq!(error(r#"{"is_error":true,"result":3}"#), None);
    assert_eq!(error(r#"{"is_error":false,"result":"fine"}"#), None);
    // A status with no text is still an error, found on the output itself when the error names none.
    assert_eq!(error(r#"{"status_code":503}"#), said("", Some(503.0)));
    assert_eq!(error(r#"{"error":{"message":"x"},"statusCode":502}"#), said("x", Some(502.0)));
    assert_eq!(error(r#"{"error":{"message":"x","status":1.5,"code":404}}"#), said("x", Some(404.0)));
    assert_eq!(error(r#"{"error":{"status":"x"},"status":"y"}"#), None);
    assert_eq!(error(r#"{"error":7,"stop_reason":8}"#), None);
    assert_eq!(error("[]"), None);
}

#[test]
fn agy_reads_the_response_and_the_usage_and_prices_nothing() {
    let stdout = r#"{"response":"pong\n","status":"SUCCESS","usage":{"cache_read_tokens":5,"input_tokens":7661,"output_tokens":29,"thinking_tokens":28}}"#;
    assert_eq!(agy_output(stdout), output(Some("pong\n"), None, None, tokens(5.0, 7661.0, 29.0)));
}

#[test]
fn agy_reads_a_result_with_no_usage_as_no_tokens() {
    let read = agy_output(r#"{"response":"x"}"#);
    assert_eq!((read.answer.as_deref(), read.tokens), (Some("x"), NO_TOKENS));
}

#[test]
fn agy_reports_the_failures_it_hides_behind_a_zero_exit_code() {
    let failure = |stdout: &str| agy_output(stdout).failure;
    assert_eq!(failure(r#"{"finished_reason":"print_timeout","response":"x"}"#).as_deref(), Some("timeout"));
    assert_eq!(failure(r#"{"denied_actions":[{"action":"write_file"}],"response":"x"}"#).as_deref(), Some("denied-action"));
    assert_eq!(failure(r#"{"response":"x","status":"ERROR"}"#).as_deref(), Some("client-error"));
    assert_eq!(failure(r#"{"denied_actions":[],"response":"x","status":"SUCCESS"}"#), None);
    assert_eq!(failure(r#"{"response":"x","status":3}"#), None);
}

#[test]
fn agy_has_no_answer_for_an_empty_response_a_list_or_text_that_is_not_json() {
    assert_eq!(agy_output(r#"{"response":" \n"}"#).answer, None);
    assert_eq!(agy_output("[]"), output(None, None, None, NO_TOKENS));
    assert_eq!(agy_output("oops"), output(None, None, None, NO_TOKENS));
}

#[test]
fn agy_reads_the_result_event_of_a_stream_and_the_last_object_with_a_response_or_usage_otherwise() {
    let streamed = lines(&[
        r#"{"event":"start"}"#,
        r#"{"event":"result","result":{"response":"first"}}"#,
        "garbage",
        r#"{"event":"result","result":{"response":"last","usage":{"input_tokens":4}}}"#,
        r#"{"event":"end"}"#,
    ]);
    assert_eq!(agy_output(&streamed).answer.as_deref(), Some("last"));
    assert_eq!(agy_output(&streamed).tokens, tokens(0.0, 4.0, 0.0));
    // A result event with no result falls back to the object that has a response, and so does a null one.
    assert_eq!(agy_output(&lines(&[r#"{"response":"plain"}"#, r#"{"event":"result"}"#])).answer.as_deref(), Some("plain"));
    assert_eq!(agy_output(&lines(&[r#"{"usage":{}}"#, r#"{"event":"result","result":null}"#])), output(None, None, None, NO_TOKENS));
    // A result that is no object is nothing.
    assert_eq!(agy_output(r#"{"event":"result","result":"text"}"#), output(None, None, None, NO_TOKENS));
}

const CLAUDE: &str =
    r#"{"is_error":false,"result":"fine","total_cost_usd":0.042,"type":"result","usage":{"cache_creation_input_tokens":20,"cache_read_input_tokens":300,"input_tokens":10,"output_tokens":40}}"#;

#[test]
fn claude_counts_cached_prompt_in_the_input_and_takes_the_dollars_it_charged() {
    assert_eq!(claude_output(CLAUDE), output(Some("fine"), Some(0.042), None, tokens(300.0, 330.0, 40.0)));
}

#[test]
fn claude_finds_the_result_among_a_list_of_events() {
    assert_eq!(claude_output(&format!(r#"[{{"type":"system"}},{CLAUDE},{{"type":"other"}},3]"#)).answer.as_deref(), Some("fine"));
    assert_eq!(claude_output(r#"[{"type":"system"}]"#), output(None, None, None, NO_TOKENS));
}

#[test]
fn claude_reports_an_error_result_and_prices_nothing_when_no_dollars_came_back() {
    assert_eq!(claude_output(&CLAUDE.replace("false", "true")).failure.as_deref(), Some("client-error"));
    assert_eq!(claude_output(&CLAUDE.replace("0.042", "0")).charged, None);
    assert_eq!(claude_output(&CLAUDE.replace("0.042", "\"free\"")).charged, None);
    let bare = claude_output(r#"{"result":"x"}"#);
    assert_eq!((bare.answer.as_deref(), bare.charged, bare.tokens), (Some("x"), None, NO_TOKENS));
}

#[test]
fn claude_has_no_answer_for_text_that_is_not_json_or_a_blank_result() {
    assert_eq!(claude_output("oops").answer, None);
    assert_eq!(claude_output(r#"{"result":""}"#).answer, None);
    assert_eq!(claude_output(r#"{"result":"  "}"#).answer, None);
}

#[test]
fn claude_carries_the_error_of_an_error_result() {
    let read = claude_output(r#"{"is_error":true,"result":"invalid api key","type":"result"}"#);
    assert_eq!(read.error, Some(ClientError { text: "invalid api key".to_string(), status: None }));
}

const FINISH: fn(&str, &str) -> String = |cost, tokens| format!(r#"{{"part":{{"cost":{cost},"tokens":{tokens}}},"type":"step_finish"}}"#);

fn text(part: &str) -> String {
    format!(r#"{{"part":{{"text":"{part}"}},"type":"text"}}"#)
}

#[test]
fn open_code_adds_up_every_steps_dollars_and_tokens_and_answers_with_the_last_step_that_wrote_text() {
    let events = [
        r#"{"type":"step_start"}"#.to_string(),
        text("first"),
        FINISH("0.01", r#"{"cache":{"read":5,"write":1},"input":10,"output":2,"reasoning":1}"#),
        r#"{"type":"step_start"}"#.to_string(),
        text("second "),
        text("half"),
        FINISH("0.02", r#"{"input":20,"output":4}"#),
        r#"{"type":"step_start"}"#.to_string(),
        FINISH("0", r#"{"input":1,"output":1}"#),
    ];
    let read = open_code_output(&events.join("\n"));
    assert_eq!(read.answer.as_deref(), Some("second half"));
    assert_eq!((read.failure, read.error), (None, None));
    assert_eq!(read.tokens, tokens(5.0, 37.0, 8.0));
    assert!((read.charged.expect("dollars") - 0.03).abs() < 1e-12);
}

#[test]
fn open_code_fails_on_an_error_event_and_skips_lines_and_events_it_cannot_read() {
    let stdout = format!("not json\n{}", lines(&[r#""a string""#, "null", r#"{"type":"error"}"#, r#"{"part":{"text":7},"type":"text"}"#, r#"{"type":"step_finish"}"#]));
    assert_eq!(open_code_output(&stdout), output(None, None, Some("client-error"), NO_TOKENS));
}

#[test]
fn open_code_answers_with_text_that_comes_before_any_step_starts() {
    assert_eq!(open_code_output(&lines(&[&text("a"), &text("b")])).answer.as_deref(), Some("ab"));
}

#[test]
fn open_code_has_no_answer_for_an_empty_output_and_keeps_the_error_of_an_error_event() {
    assert_eq!(open_code_output(""), output(None, None, None, NO_TOKENS));
    let read = open_code_output(r#"{"type":"error","error":{"message":"quota reached","status":429}}"#);
    assert_eq!(read.error, Some(ClientError { text: "quota reached".to_string(), status: Some(429.0) }));
}

fn message(text: &str) -> String {
    format!(r#"{{"item":{{"id":"item_0","text":"{text}","type":"agent_message"}},"type":"item.completed"}}"#)
}

fn turn(usage: &str) -> String {
    format!(r#"{{"type":"turn.completed","usage":{usage}}}"#)
}

#[test]
fn codex_answers_with_the_last_agent_message_and_counts_the_tokens_the_turn_reported() {
    let stdout = lines(&[
        r#"{"thread_id":"t-1","type":"thread.started"}"#,
        r#"{"type":"turn.started"}"#,
        &message("first"),
        &message("the answer\\nSUMMARY: it is fine"),
        &turn(r#"{"cached_input_tokens":100,"input_tokens":1000,"output_tokens":200,"reasoning_output_tokens":50}"#),
    ]);
    assert_eq!(codex_output(&stdout), output(Some("the answer\nSUMMARY: it is fine"), None, None, tokens(100.0, 1000.0, 200.0)));
}

#[test]
fn codex_adds_up_every_turn_of_a_run_and_prices_nothing_itself() {
    let stdout = lines(&[&turn(r#"{"cached_input_tokens":0,"input_tokens":10,"output_tokens":2}"#), &turn(r#"{"cached_input_tokens":5,"input_tokens":20,"output_tokens":4}"#)]);
    assert_eq!(codex_output(&stdout), output(None, None, None, tokens(5.0, 30.0, 6.0)));
    assert_eq!(codex_output(r#"{"type":"turn.completed"}"#), output(None, None, None, NO_TOKENS));
}

#[test]
fn codex_fails_on_an_error_event_keeping_what_it_said_for_the_refusal_patterns() {
    let said = |text: &str| Some(ClientError { text: text.to_string(), status: None });
    let read = codex_output(r#"{"message":"429 Too Many Requests","type":"error"}"#);
    assert_eq!((read.failure.as_deref(), read.error), (Some("client-error"), said("429 Too Many Requests")));
    let failed = codex_output(r#"{"error":{"message":"stream disconnected"},"type":"turn.failed"}"#);
    assert_eq!((failed.failure.as_deref(), failed.error), (Some("client-error"), said("stream disconnected")));
    let bare = codex_output(r#"{"type":"error"}"#);
    assert_eq!((bare.failure.as_deref(), bare.error), (Some("client-error"), None));
    let blank = codex_output(r#"{"message":"  ","type":"error"}"#);
    assert_eq!(blank.error, None);
}

#[test]
fn codex_has_no_answer_for_an_empty_output_a_blank_message_or_text_that_is_not_json() {
    assert_eq!(codex_output(""), output(None, None, None, NO_TOKENS));
    assert_eq!(codex_output(&message("  ")).answer, None);
    assert_eq!(codex_output("oops").answer, None);
    assert_eq!(codex_output(r#"{"item":{"text":"x","type":"reasoning"},"type":"item.completed"}"#).answer, None);
    assert_eq!(codex_output(r#"{"item":3,"type":"item.completed"}"#).answer, None);
}

#[test]
fn hermes_answers_with_what_it_printed_and_reads_the_report_it_wrote() {
    let report = v(r#"{"cache_read_tokens":10,"estimated_cost_usd":0.5,"failed":true,"input_tokens":100,"output_tokens":20}"#);
    assert_eq!(hermes_output("the answer\n", Some(&report)), output(Some("the answer"), Some(0.5), Some("failed"), tokens(10.0, 100.0, 20.0)));
    assert_eq!(hermes_output("", None), output(None, None, None, NO_TOKENS));
}
