//! What the scenarios cannot reach: a decision slower than the deadline, and routing with no Vibe client.

use crate::session_start::{decide_in_time, setup_check_ids};
use model_gateway::json::Value;
use std::time::Duration;

#[test]
fn a_decision_slower_than_the_deadline_fails_open_and_is_logged() {
    let home = std::env::temp_dir().join(format!("critique-session-start-{}", std::process::id()));
    let mut runtime = crate::runtime::default_runtime();
    runtime.env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
    let result = decide_in_time(&Value::Null, &runtime, Duration::ZERO);
    let field = |key: &str| result.as_object().and_then(|object| object.get(key)).and_then(Value::as_str).map(str::to_string);
    assert_eq!(field("failure").as_deref(), Some("deadline"));
    assert_eq!(field("detail").as_deref(), Some("the session-start hook did not decide within 700 ms"));
    let log = std::fs::read_to_string(home.join(".local/share/agent-kit/critique/gate.log.jsonl")).expect("the gate log");
    assert!(log.starts_with(r#"{"cwd":null,"detail":"the session-start hook did not decide within 700 ms","failure":"deadline","gate":"session-start","#), "{log}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn routing_with_no_critic_client_leaves_the_hook_checks() {
    assert_eq!(setup_check_ids(&[]), vec!["hooks-installed".to_string(), "kill-switch".to_string()]);
}

#[test]
fn with_no_home_it_fails_open_with_nowhere_to_log() {
    let mut runtime = crate::runtime::default_runtime();
    runtime.env.remove("HOME");
    let result = decide_in_time(&Value::Null, &runtime, Duration::from_secs(60));
    let detail = result.as_object().and_then(|object| object.get("detail")).and_then(Value::as_str);
    assert_eq!(detail, Some("required environment variable is missing: HOME"));
}
