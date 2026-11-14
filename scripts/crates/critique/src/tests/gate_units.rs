//! What the gate scenarios cannot reach: a flow state file that cannot be read, state files and index lines of the
//! wrong shape, and a failed-open gate with no home to log to.

use super::scenarios::Sandbox;
use crate::gate_command::{decide_in_time, parse_ticket_review_record, parse_triage_index_record, read_flow, Failure};
use model_gateway::json::Value;
use std::time::Duration;

#[test]
fn a_flow_state_file_that_cannot_be_read_is_an_error_not_unreadable_state() {
    let runtime = crate::runtime::default_runtime();
    match read_flow(&runtime, "/nonexistent/critique/flows/x.json") {
        Err(Failure::Error(message)) => assert!(message.starts_with("ENOENT: no such file or directory"), "{message}"),
        other => panic!("not an error: {other:?}"),
    }
}

#[test]
fn with_no_home_the_gate_fails_open_with_nowhere_to_log() {
    let mut runtime = crate::runtime::default_runtime();
    runtime.env.remove("HOME");
    runtime.env.remove("CRITIQUE_OFF");
    let result = decide_in_time(&Value::Null, &runtime, Duration::from_secs(60));
    let detail = result.as_object().and_then(|object| object.get("detail")).and_then(Value::as_str);
    assert_eq!(detail, Some("required environment variable is missing: HOME"));
    let late = decide_in_time(&Value::Null, &runtime, Duration::ZERO);
    assert_eq!(late.as_object().and_then(|object| object.get("failure")).and_then(Value::as_str), Some("deadline"));
}

#[test]
fn a_flow_state_file_of_the_wrong_shape_is_unreadable_state() {
    let sandbox = Sandbox::new();
    let runtime = crate::runtime::default_runtime();
    let unreadable = |text: &str| {
        let path = format!("{}/flow.json", sandbox.text());
        std::fs::write(&path, text).expect("write the flow state");
        match read_flow(&runtime, &path) {
            Err(Failure::Unreadable(message)) => message.trim_start_matches(&format!("{path}: ")).to_string(),
            other => panic!("not unreadable state: {other:?}"),
        }
    };
    assert_eq!(unreadable("[]"), "the value must be a JSON object");
    assert_eq!(unreadable(r#"{"phase":"planning"}"#), "repository must be a non-empty string");
}

#[test]
fn an_index_line_that_is_not_an_object_is_refused() {
    assert_eq!(parse_triage_index_record("[]").err().as_deref(), Some("the value must be a JSON object"));
    assert_eq!(parse_ticket_review_record("[]").err().as_deref(), Some("the value must be a JSON object"));
}

#[test]
fn a_configuration_that_is_not_an_object_fails_the_gate_open() {
    let sandbox = Sandbox::new();
    let config = format!("{}/.config/agent-kit", sandbox.text());
    std::fs::create_dir_all(&config).expect("create the configuration directory");
    std::fs::write(format!("{config}/critique.json"), "[]").expect("write the configuration");
    let mut runtime = crate::runtime::default_runtime();
    runtime.env.insert("HOME".to_string(), sandbox.text());
    runtime.env.remove("CRITIQUE_OFF");
    let result = decide_in_time(&Value::Null, &runtime, Duration::from_secs(60));
    let text = |key: &str| result.as_object().and_then(|object| object.get(key)).and_then(Value::as_str).map(str::to_string);
    assert_eq!(text("failure").as_deref(), Some("unreadable-state"));
    assert_eq!(text("detail"), Some(format!("{config}/critique.json: the value must be a JSON object")));
}
