//! Tests for the stub ACP agent, driven through its public API.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};

use crate::stub::{
    fenced_findings, fenced_questions, findings_document, model_option, questions_document, serve,
    serve_with_role, Role, Stub, FAIL, MODELS_ENV, NEEDS_HUMAN, OPEN_QUESTIONS, REPORT_DIR_ENV,
    ROLE_ENV, VERDICT_ENV,
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let tick = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "gold-band-proof-stub-{}-{}-{}",
        std::process::id(),
        tick,
        label
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

fn one(stub: &mut Stub, request: &Value, verdict: &str, models: &str) -> Value {
    let frames = stub.handle(request, verdict, models);
    assert_eq!(frames.len(), 1, "expected one frame for {request}");
    frames.into_iter().next().unwrap()
}

/// The answer text one prompt streams, asserting the turn ends after it.
fn prompt_text(stub: &mut Stub, verdict: &str) -> String {
    let frames = stub.handle(
        &request(
            2,
            "session/prompt",
            json!({ "sessionId": "stub-session-1" }),
        ),
        verdict,
        "",
    );
    assert_eq!(
        frames.len(),
        2,
        "expected an update and an end for {verdict}"
    );
    assert_eq!(frames[1]["result"]["stopReason"], "end_turn");
    frames[0]["params"]["update"]["content"]["text"]
        .as_str()
        .expect("the update carries text")
        .to_string()
}

/// The JSON document inside one fenced json block.
fn fenced_body(text: &str) -> Value {
    let body = text
        .strip_prefix("```json\n")
        .unwrap_or_else(|| panic!("not a fenced json block: {text}"))
        .strip_suffix("\n```")
        .unwrap_or_else(|| panic!("not a closed json block: {text}"));
    serde_json::from_str(body).unwrap_or_else(|error| panic!("not JSON ({error}): {body}"))
}

#[test]
fn the_environment_names_are_the_ones_gold_band_sets() {
    assert_eq!(VERDICT_ENV, "STUB_ACP_VERDICT");
    assert_eq!(MODELS_ENV, "STUB_ACP_MODELS");
    assert_eq!(ROLE_ENV, "STUB_ACP_ROLE");
    assert_eq!(REPORT_DIR_ENV, "STUB_ACP_REPORT_DIR");
    assert_eq!(NEEDS_HUMAN, "needs-human");
    assert_eq!(OPEN_QUESTIONS, "open-questions");
    assert_eq!(FAIL, "fail");
}

#[test]
fn role_parse_reads_every_spelling() {
    assert_eq!(Role::parse(""), Role::Findings);
    assert_eq!(Role::parse("findings"), Role::Findings);
    assert_eq!(Role::parse(" answer "), Role::Answer);
    assert_eq!(Role::parse("questions"), Role::Questions);
    assert_eq!(Role::parse(" questions "), Role::Questions);
    assert_eq!(Role::parse("gate:build"), Role::Gate("build".to_string()));
    assert_eq!(Role::parse("copy:lint"), Role::Copy("lint".to_string()));
    assert_eq!(Role::parse("gate:"), Role::Findings);
    assert_eq!(Role::parse("copy:"), Role::Findings);
    assert_eq!(Role::parse("nonsense"), Role::Findings);
}

#[test]
fn role_text_round_trips_through_parse() {
    assert_eq!(Role::Findings.as_text(), "findings");
    assert_eq!(Role::Answer.as_text(), "answer");
    assert_eq!(Role::Questions.as_text(), "questions");
    assert_eq!(Role::Gate("build".to_string()).as_text(), "gate:build");
    assert_eq!(Role::Copy("lint".to_string()).as_text(), "copy:lint");
    for role in [
        Role::Findings,
        Role::Answer,
        Role::Questions,
        Role::Gate("build".to_string()),
        Role::Copy("lint".to_string()),
    ] {
        assert_eq!(Role::parse(&role.as_text()), role);
    }
}

#[test]
fn initialize_answers_with_the_protocol_version_and_no_auth() {
    let mut stub = Stub::new();
    let frame = one(&mut stub, &request(1, "initialize", json!({})), "", "");
    assert_eq!(frame["jsonrpc"], "2.0");
    assert_eq!(frame["id"], json!(1));
    assert_eq!(frame["result"]["protocolVersion"], json!(1));
    assert_eq!(
        frame["result"]["agentCapabilities"]["loadSession"],
        json!(false)
    );
    assert_eq!(frame["result"]["authMethods"], json!([]));
}

#[test]
fn session_new_hands_out_incrementing_ids_and_the_model_picker() {
    let mut stub = Stub::default();
    let first = one(&mut stub, &request(1, "session/new", json!({})), "", "a, b");
    assert_eq!(first["result"]["sessionId"], "stub-session-1");
    assert_eq!(
        first["result"]["configOptions"],
        json!([{
            "id": "model",
            "name": "Model",
            "category": "model",
            "type": "select",
            "currentValue": "a",
            "options": [{ "value": "a", "name": "a" }, { "value": "b", "name": "b" }],
        }])
    );
    let second = one(&mut stub, &request(2, "session/new", json!({})), "", "");
    assert_eq!(second["result"]["sessionId"], "stub-session-2");
    assert_eq!(second["result"]["configOptions"], json!([]));
}

#[test]
fn session_list_is_always_empty() {
    let mut stub = Stub::new();
    one(&mut stub, &request(1, "session/new", json!({})), "", "");
    let frame = one(&mut stub, &request(2, "session/list", json!({})), "", "");
    assert_eq!(frame["result"], json!({ "sessions": [] }));
}

#[test]
fn set_config_option_reports_the_model_only_for_the_model_config() {
    let mut stub = Stub::new();
    let chosen = one(
        &mut stub,
        &request(
            1,
            "session/set_config_option",
            json!({ "configId": "model", "value": "b" }),
        ),
        "",
        "a,b",
    );
    assert_eq!(chosen["result"]["configOptions"][0]["currentValue"], "b");
    let other = one(
        &mut stub,
        &request(
            2,
            "session/set_config_option",
            json!({ "configId": "mode", "value": "b" }),
        ),
        "",
        "a,b",
    );
    assert_eq!(other["result"]["configOptions"][0]["currentValue"], "a");
    let blank = one(
        &mut stub,
        &request(
            3,
            "session/set_config_option",
            json!({ "configId": "model", "value": "" }),
        ),
        "",
        "a,b",
    );
    assert_eq!(blank["result"]["configOptions"][0]["currentValue"], "a");
    let none = one(
        &mut stub,
        &request(
            4,
            "session/set_config_option",
            json!({ "configId": "model", "value": "b" }),
        ),
        "",
        "",
    );
    assert_eq!(none["result"]["configOptions"], json!([]));
}

#[test]
fn set_mode_answers_with_an_empty_result() {
    let mut stub = Stub::new();
    let frame = one(
        &mut stub,
        &request(1, "session/set_mode", json!({ "modeId": "code" })),
        "",
        "",
    );
    assert_eq!(frame["result"], json!({}));
}

#[test]
fn delete_and_close_forget_the_session() {
    let mut stub = Stub::new();
    one(&mut stub, &request(1, "session/new", json!({})), "", "");
    one(&mut stub, &request(2, "session/new", json!({})), "", "");
    let deleted = one(
        &mut stub,
        &request(
            3,
            "session/delete",
            json!({ "sessionId": "stub-session-1" }),
        ),
        "",
        "",
    );
    assert_eq!(deleted["result"], json!({}));
    let after_delete = stub.handle(&request(4, "session/prompt", json!({})), "", "");
    assert_eq!(after_delete[0]["params"]["sessionId"], "stub-session-2");
    let closed = one(
        &mut stub,
        &request(5, "session/close", json!({ "sessionId": "stub-session-2" })),
        "",
        "",
    );
    assert_eq!(closed["result"], json!({}));
    let after_close = stub.handle(&request(6, "session/prompt", json!({})), "", "");
    assert_eq!(after_close[0]["params"]["sessionId"], "");
    let bare = one(&mut stub, &request(7, "session/delete", json!({})), "", "");
    assert_eq!(bare["result"], json!({}));
}

#[test]
fn prompt_streams_the_fenced_findings_and_ends_the_turn() {
    let mut stub = Stub::new();
    one(&mut stub, &request(1, "session/new", json!({})), "", "");
    let frames = stub.handle(
        &request(
            2,
            "session/prompt",
            json!({ "sessionId": "stub-session-1" }),
        ),
        "",
        "",
    );
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["jsonrpc"], "2.0");
    assert_eq!(frames[0]["method"], "session/update");
    assert_eq!(frames[0]["params"]["sessionId"], "stub-session-1");
    assert_eq!(
        frames[0]["params"]["update"]["sessionUpdate"],
        "agent_message_chunk"
    );
    assert_eq!(frames[0]["params"]["update"]["content"]["type"], "text");
    assert_eq!(
        frames[0]["params"]["update"]["content"]["text"],
        fenced_findings("")
    );
    assert_eq!(frames[1]["id"], json!(2));
    assert_eq!(frames[1]["result"], json!({ "stopReason": "end_turn" }));
}

#[test]
fn prompt_with_the_needs_human_verdict_streams_the_marker_finding() {
    let mut stub = Stub::new();
    one(&mut stub, &request(1, "session/new", json!({})), "", "");
    let frames = stub.handle(
        &request(
            2,
            "session/prompt",
            json!({ "sessionId": "stub-session-1" }),
        ),
        " needs-human ",
        "",
    );
    assert_eq!(frames.len(), 2);
    let text = frames[0]["params"]["update"]["content"]["text"]
        .as_str()
        .unwrap();
    assert_eq!(text, fenced_findings(NEEDS_HUMAN));
    assert!(text.contains("stub-needs-human-1"));
    assert_eq!(frames[1]["result"]["stopReason"], "end_turn");
}

#[test]
fn prompt_with_the_fail_verdict_answers_with_an_error() {
    let mut stub = Stub::new();
    one(&mut stub, &request(1, "session/new", json!({})), "", "");
    let frames = stub.handle(
        &request(
            2,
            "session/prompt",
            json!({ "sessionId": "stub-session-1" }),
        ),
        " fail ",
        "",
    );
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["jsonrpc"], "2.0");
    assert_eq!(frames[0]["id"], json!(2));
    assert_eq!(frames[0]["error"]["code"], json!(-32000));
    assert_eq!(frames[0]["error"]["message"], "stub failure");
    assert!(frames[0].get("result").is_none());
}

#[test]
fn prompt_without_a_session_id_falls_back_to_the_first_known_session() {
    let mut stub = Stub::new();
    let empty = stub.handle(&request(1, "session/prompt", json!({})), "", "");
    assert_eq!(empty[0]["params"]["sessionId"], "");
    one(&mut stub, &request(2, "session/new", json!({})), "", "");
    one(&mut stub, &request(3, "session/new", json!({})), "", "");
    let fallback = stub.handle(
        &request(4, "session/prompt", json!({ "sessionId": "" })),
        "",
        "",
    );
    assert_eq!(fallback[0]["params"]["sessionId"], "stub-session-1");
}

#[test]
fn unknown_method_with_an_id_is_method_not_found() {
    let mut stub = Stub::new();
    let frame = one(&mut stub, &request(7, "session/unknown", json!({})), "", "");
    assert_eq!(frame["id"], json!(7));
    assert_eq!(frame["error"]["code"], json!(-32601));
    assert_eq!(frame["error"]["message"], "Method not found");
}

#[test]
fn unknown_notification_is_silent() {
    let mut stub = Stub::new();
    assert!(stub
        .handle(&notification("session/unknown", json!({})), "", "")
        .is_empty());
    assert!(stub
        .handle(
            &json!({ "jsonrpc": "2.0", "id": null, "method": "session/unknown" }),
            "",
            ""
        )
        .is_empty());
    assert!(stub.handle(&json!({ "params": {} }), "", "").is_empty());
}

#[test]
fn serve_skips_blank_and_non_object_lines_and_writes_one_line_per_frame() {
    let input = concat!(
        "\n",
        "not json\n",
        "[1,2]\n",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/new\",\"params\":{}}\n",
    );
    let mut output: Vec<u8> = Vec::new();
    serve(input.as_bytes(), &mut output, "", "a").unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.ends_with('\n'));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    let first: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["result"]["protocolVersion"], json!(1));
    let second: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(second["result"]["sessionId"], "stub-session-1");
    assert_eq!(second["result"]["configOptions"][0]["currentValue"], "a");
}

#[test]
fn serve_writes_every_frame_of_a_prompt_and_the_failure_error() {
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"session/new\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/prompt\",\"params\":{}}\n",
    );
    let mut output: Vec<u8> = Vec::new();
    serve(input.as_bytes(), &mut output, "", "").unwrap();
    let text = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    let update: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(update["method"], "session/update");
    assert_eq!(update["params"]["sessionId"], "stub-session-1");
    let end: Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(end["result"]["stopReason"], "end_turn");

    let mut failed: Vec<u8> = Vec::new();
    serve(input.as_bytes(), &mut failed, FAIL, "").unwrap();
    let failed = String::from_utf8(failed).unwrap();
    let lines: Vec<&str> = failed.lines().collect();
    assert_eq!(lines.len(), 2);
    let error: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(error["error"]["code"], json!(-32000));
}

#[test]
fn clean_document_is_the_empty_verdict() {
    let document = findings_document("");
    assert_eq!(document["findings"], json!([]));
    assert!(document["no_findings_reason"]
        .as_str()
        .unwrap()
        .contains("raised nothing"));
    assert_eq!(findings_document("anything else"), document);
}

#[test]
fn needs_human_document_carries_the_marker_finding() {
    let document = findings_document(" needs-human ");
    let finding = &document["findings"][0];
    assert_eq!(finding["id"], "stub-needs-human-1");
    assert_eq!(finding["severity"], "major");
    assert_eq!(finding["category"], "undocumented-decision");
    assert_eq!(finding["check"], "spec-redteam-needs-human");
    assert_eq!(finding["location"], "requirement.md");
    assert_eq!(finding["evidence"][0]["kind"], "input");
    assert_eq!(
        finding["evidence"][0]["value"],
        "STUB_ACP_VERDICT=needs-human"
    );
    assert!(finding["suggestion"]
        .as_str()
        .unwrap()
        .contains("manual check"));
}

#[test]
fn fenced_findings_wraps_the_pretty_document() {
    let fenced = fenced_findings("");
    let body = serde_json::to_string_pretty(&findings_document("")).unwrap();
    assert_eq!(fenced, format!("```json\n{body}\n```"));
    assert!(fenced.starts_with("```json\n{\n  \"findings\": []"));
    assert!(fenced.ends_with("\n```"));
    assert!(fenced_findings(NEEDS_HUMAN).contains("\"stub-needs-human-1\""));
}

#[test]
fn open_questions_document_carries_the_marker_question() {
    let document = questions_document(" open-questions ");
    let question = &document["open_questions"][0];
    assert_eq!(question["id"], "stub-question-1");
    assert!(question["question"]
        .as_str()
        .unwrap()
        .contains("manual check"));
}

#[test]
fn questions_document_is_empty_for_another_verdict() {
    assert_eq!(
        questions_document(""),
        json!({ "ok": true, "open_questions": [] })
    );
    assert_eq!(
        questions_document(NEEDS_HUMAN),
        json!({ "ok": true, "open_questions": [] })
    );
    assert_eq!(questions_document("anything else"), questions_document(""));
}

#[test]
fn fenced_questions_wraps_the_pretty_document() {
    let fenced = fenced_questions(OPEN_QUESTIONS);
    let body = serde_json::to_string_pretty(&questions_document(OPEN_QUESTIONS)).unwrap();
    assert_eq!(fenced, format!("```json\n{body}\n```"));
    assert!(fenced.starts_with("```json\n{\n  \"ok\": true,"));
    assert!(fenced.ends_with("\n```"));
    assert!(fenced_questions("clean").contains("\"open_questions\": []"));
}

#[test]
fn model_option_is_empty_without_models() {
    assert!(model_option("", None).is_empty());
    assert!(model_option(" , ,", None).is_empty());
}

#[test]
fn model_option_selects_the_current_model_or_the_first() {
    let picker = model_option("a,b", Some("b"));
    assert_eq!(picker.len(), 1);
    assert_eq!(picker[0]["currentValue"], "b");
    assert_eq!(
        picker[0]["options"],
        json!([{ "value": "a", "name": "a" }, { "value": "b", "name": "b" }])
    );
    assert_eq!(model_option("a,b", None)[0]["currentValue"], "a");
    assert_eq!(model_option("a,b", Some(""))[0]["currentValue"], "a");
}

#[test]
fn both_documents_carry_every_key_the_finding_schema_requires() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../skills/workstation/critique/schemas/finding.schema.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let schema: Value = serde_json::from_str(&text).unwrap();
    let clean = findings_document("");
    let needs_human = findings_document(NEEDS_HUMAN);
    for name in schema["required"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        assert!(clean.get(name).is_some(), "the clean document lacks {name}");
        assert!(
            needs_human.get(name).is_some(),
            "the needs-human document lacks {name}"
        );
    }
    let finding = &needs_human["findings"][0];
    for name in schema["$defs"]["finding"]["required"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        assert!(finding.get(name).is_some(), "the finding lacks {name}");
    }
    let evidence = &finding["evidence"][0];
    for name in schema["$defs"]["evidence"]["required"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        assert!(evidence.get(name).is_some(), "the evidence lacks {name}");
    }
}

#[test]
fn the_answer_role_answers_ok() {
    let mut stub = Stub::with_role(Role::Answer, String::new());
    assert_eq!(
        fenced_body(&prompt_text(&mut stub, "")),
        json!({ "ok": true })
    );
}

#[test]
fn the_questions_role_answers_the_open_questions_document() {
    let mut stub = Stub::with_role(Role::Questions, String::new());
    assert_eq!(
        fenced_body(&prompt_text(&mut stub, OPEN_QUESTIONS)),
        questions_document(OPEN_QUESTIONS)
    );
    assert_eq!(
        fenced_body(&prompt_text(&mut stub, "clean")),
        json!({ "ok": true, "open_questions": [] })
    );
}

#[test]
fn the_gate_role_answers_the_exit_code_the_wrapper_earned() {
    let dir = temp_dir("gate-pass");
    let mut stub = Stub::with_role(
        Role::Gate("build".to_string()),
        dir.to_string_lossy().into_owned(),
    );
    assert_eq!(
        fenced_body(&prompt_text(&mut stub, "clean")),
        json!({ "exit_code": 0 })
    );
    assert!(dir.join("build/gate-report.json").is_file());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_gate_role_answers_one_when_the_verdict_fails_it() {
    let dir = temp_dir("gate-fail");
    let mut stub = Stub::with_role(
        Role::Gate("build".to_string()),
        dir.to_string_lossy().into_owned(),
    );
    assert_eq!(
        fenced_body(&prompt_text(&mut stub, "gate-fail=build")),
        json!({ "exit_code": 1 })
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_gate_role_answers_two_when_the_report_directory_cannot_be_created() {
    let dir = temp_dir("gate-blocked");
    fs::write(dir.join("build"), "not a directory").expect("blocker written");
    let mut stub = Stub::with_role(
        Role::Gate("build".to_string()),
        dir.to_string_lossy().into_owned(),
    );
    assert_eq!(
        fenced_body(&prompt_text(&mut stub, "clean")),
        json!({ "exit_code": 2 })
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_copy_role_answers_the_report_the_gate_wrote() {
    let dir = temp_dir("copy-pass");
    let report_dir = dir.to_string_lossy().into_owned();
    let mut gate = Stub::with_role(Role::Gate("build".to_string()), report_dir.clone());
    assert_eq!(
        fenced_body(&prompt_text(&mut gate, "clean")),
        json!({ "exit_code": 0 })
    );
    let mut copy = Stub::with_role(Role::Copy("build".to_string()), report_dir);
    let report = fenced_body(&prompt_text(&mut copy, "clean"));
    assert_eq!(report["exit_code"], json!(0));
    assert_eq!(report["run_id"], "proof-run");
    assert_eq!(report["attempt"], json!(1));
    assert_eq!(report["command"], "true");
    assert_eq!(report["output_hash"].as_str().map(str::len), Some(64));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_copy_role_answers_one_when_the_gate_failed() {
    let dir = temp_dir("copy-failed");
    let report_dir = dir.to_string_lossy().into_owned();
    let mut gate = Stub::with_role(Role::Gate("build".to_string()), report_dir.clone());
    assert_eq!(
        fenced_body(&prompt_text(&mut gate, "gate-fail=build")),
        json!({ "exit_code": 1 })
    );
    let mut copy = Stub::with_role(Role::Copy("build".to_string()), report_dir);
    assert_eq!(
        fenced_body(&prompt_text(&mut copy, "gate-fail=build")),
        json!({
            "run_id": "",
            "attempt": 0,
            "command": "",
            "exit_code": 1,
            "output_hash": "",
            "timestamp": "",
        })
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_copy_role_answers_a_failing_report_without_a_report() {
    let dir = temp_dir("copy-missing");
    let mut copy = Stub::with_role(
        Role::Copy("build".to_string()),
        dir.to_string_lossy().into_owned(),
    );
    let report = fenced_body(&prompt_text(&mut copy, "clean"));
    assert_eq!(report["exit_code"], json!(1));
    assert_eq!(report["run_id"], "");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_fail_verdict_fails_every_role() {
    let dir = temp_dir("fail-roles");
    let report_dir = dir.to_string_lossy().into_owned();
    for role in [
        Role::Answer,
        Role::Questions,
        Role::Gate("build".to_string()),
        Role::Copy("build".to_string()),
    ] {
        let mut stub = Stub::with_role(role.clone(), report_dir.clone());
        let frames = stub.handle(&request(2, "session/prompt", json!({})), " fail ", "");
        assert_eq!(frames.len(), 1, "{role:?}");
        assert_eq!(frames[0]["error"]["code"], json!(-32000), "{role:?}");
        assert_eq!(frames[0]["error"]["message"], "stub failure", "{role:?}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn serve_with_role_answers_as_the_role_it_was_given() {
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"session/new\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/prompt\",\"params\":{}}\n",
    );
    let mut output: Vec<u8> = Vec::new();
    serve_with_role(input.as_bytes(), &mut output, "", "", &Role::Answer, "").unwrap();
    let text = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    let update: Value = serde_json::from_str(lines[1]).unwrap();
    let answer = update["params"]["update"]["content"]["text"]
        .as_str()
        .unwrap();
    assert_eq!(fenced_body(answer), json!({ "ok": true }));
}
