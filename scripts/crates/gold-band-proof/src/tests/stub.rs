//! Tests for the stub ACP agent, driven through its public API.

use serde_json::{json, Value};

use crate::stub::{
    fenced_findings, findings_document, model_option, serve, Stub, FAIL, MODELS_ENV, NEEDS_HUMAN,
    VERDICT_ENV,
};

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

#[test]
fn the_environment_names_are_the_ones_gold_band_sets() {
    assert_eq!(VERDICT_ENV, "STUB_ACP_VERDICT");
    assert_eq!(MODELS_ENV, "STUB_ACP_MODELS");
    assert_eq!(NEEDS_HUMAN, "needs-human");
    assert_eq!(FAIL, "fail");
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
