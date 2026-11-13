//! What the gate scenarios cannot reach: a flow state file that cannot be read, and a failed-open gate with no home
//! to log to.

use crate::gate_command::{decide_in_time, read_flow, Failure};
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
