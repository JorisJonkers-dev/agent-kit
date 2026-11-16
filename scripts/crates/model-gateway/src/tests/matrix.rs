//! The harness matrix: how its baseline is read, how a run becomes a cell and a cell a regression, and that the
//! docs carry the table the baseline promises. The stub proof itself is the `matrix-stub` scenario.

use crate::error::GatewayError;
use crate::json::{parse, Value};
use crate::matrix::{classify, expected_table, read_baseline, regression, render_table, Baseline, Expectation, Status, SHIPPED_BASELINE};

fn baseline(expected: &str) -> Result<Baseline, GatewayError> {
    read_baseline(&format!(
        r#"{{"version": 1, "workflow": "w", "prompt": "p", "roles": ["a", "b"], "harnesses": ["x", "y"], "expected": {expected}}}"#
    ))
}

fn refused(expected: &str) -> String {
    baseline(expected).expect_err("a refused baseline").message
}

fn result(text: &str) -> Result<Value, GatewayError> {
    Ok(parse(text).expect("JSON"))
}

#[test]
fn the_shipped_baseline_is_read_and_expects_hermes_to_pass_and_codex_blocked_live() {
    let shipped = read_baseline(SHIPPED_BASELINE).expect("shipped");
    assert_eq!(shipped.roles.len(), 6);
    assert_eq!(shipped.harnesses.len(), 7);
    for role in &shipped.roles {
        assert_eq!(shipped.expect(false, role, "hermes").status, Status::Pass);
        assert_eq!(shipped.expect(true, role, "hermes").status, Status::Pass);
        assert_eq!(shipped.expect(false, role, "claude"), Expectation { status: Status::Pass, why: None });
        assert_eq!(shipped.expect(true, role, "codex").status, Status::Blocked);
    }
}

#[test]
fn a_cell_entry_beats_a_harness_entry_and_a_cell_nobody_lists_passes() {
    let found = baseline(
        r#"{"stub": {"x": {"status": "blocked", "why": "all"}, "b/x": {"status": "pass"}}, "live": {}}"#,
    )
    .expect("baseline");
    assert_eq!(found.expect(false, "a", "x").status, Status::Blocked);
    assert_eq!(found.expect(false, "b", "x").status, Status::Pass);
    assert_eq!(found.expect(true, "a", "x").status, Status::Pass);
    assert_eq!(found.expect(false, "a", "y").why, None);
}

#[test]
fn a_malformed_baseline_is_refused_with_what_is_wrong() {
    assert!(read_baseline("[]").expect_err("array").message.contains("not a JSON object"));
    assert!(read_baseline(r#"{"roles": "a"}"#).expect_err("roles").message.contains("roles must be a list"));
    assert!(read_baseline(r#"{"roles": [], "harnesses": ["x"]}"#).expect_err("empty").message.contains("non-empty list"));
    assert!(read_baseline(r#"{"roles": ["a"], "harnesses": ["x"]}"#).expect_err("workflow").message.contains("workflow must be"));
    let no_expected = r#"{"workflow": "w", "prompt": "p", "roles": ["a"], "harnesses": ["x"]}"#;
    assert!(read_baseline(no_expected).expect_err("expected").message.contains("expected must be an object"));
    assert!(refused(r#"{"stub": {}}"#).contains("expected.live must be an object"));
    assert!(refused(r#"{"stub": {"z": {"status": "pass"}}, "live": {}}"#).contains("names z"));
    assert!(refused(r#"{"stub": {"c/x": {"status": "pass"}}, "live": {}}"#).contains("names c/x"));
    assert!(refused(r#"{"stub": {"x": "pass"}, "live": {}}"#).contains("expected.stub.x must be an object"));
    assert!(refused(r#"{"stub": {"x": {"status": "fail"}}, "live": {}}"#).contains("must be pass or blocked"));
    assert!(refused(r#"{"stub": {"x": {"status": "maybe"}}, "live": {}}"#).contains("must be pass or blocked"));
    assert!(refused(r#"{"stub": {"x": {"status": "blocked"}}, "live": {}}"#).contains("must say why"));
}

#[test]
fn a_run_becomes_the_cell_it_came_to() {
    let pass = classify("a", "x", result(r#"{"status": "ok", "client": "pi-vibe", "model": "m", "cost_usd": 0.5}"#), 12.0);
    assert_eq!((pass.status, pass.exit, pass.cost_usd, pass.model.as_deref(), pass.error.as_deref()), (Status::Pass, 0, 0.5, Some("pi-vibe:m"), None));
    assert_eq!(pass.latency_ms, 12.0);
    let free = classify("a", "x", result(r#"{"status": "ok"}"#), 0.0);
    assert_eq!((free.cost_usd, free.model), (0.0, None));
    let unavailable = classify("a", "x", result(r#"{"status": "unavailable", "reasons": []}"#), 0.0);
    assert_eq!(unavailable.status, Status::Blocked);
    let failed = classify("a", "x", result(r#"{"status": "failed", "reason": "exit 2"}"#), 0.0);
    assert_eq!((failed.status, failed.error.as_deref()), (Status::Fail, Some("exit 2")));
    let signed_out = classify("a", "x", result(r#"{"status": "failed", "reason": "Please Sign in first"}"#), 0.0);
    assert_eq!(signed_out.status, Status::Blocked);
    let silent = classify("a", "x", result(r#"{"status": "failed"}"#), 0.0);
    assert_eq!(silent.error.as_deref(), Some("the run failed with no reason"));
    let odd = classify("a", "x", result("[1]"), 0.0);
    assert_eq!((odd.status, odd.exit), (Status::Fail, 1));
}

#[test]
fn a_refusal_that_is_a_missing_prerequisite_is_blocked_and_any_other_error_fails() {
    for code in ["profile-refused", "no-grant", "grant-expiring", "unknown-model"] {
        assert_eq!(classify("a", "x", Err(GatewayError::new(code, "why")), 0.0).status, Status::Blocked, "{code}");
    }
    let cell = classify("a", "x", Err(GatewayError::new("unexpected", "boom")), 0.0);
    assert_eq!((cell.status, cell.exit, cell.error.as_deref()), (Status::Fail, 1, Some("unexpected: boom")));
}

#[test]
fn a_cell_that_does_not_do_what_was_expected_is_a_regression_and_a_stale_marker_says_so() {
    let pass = Expectation { status: Status::Pass, why: None };
    let blocked = Expectation { status: Status::Blocked, why: Some("w".into()) };
    let ok = classify("a", "x", result(r#"{"status": "ok"}"#), 0.0);
    let failed = classify("a", "x", result(r#"{"status": "failed", "reason": "r"}"#), 0.0);
    let bare = classify("a", "x", Err(GatewayError::new("unexpected", "e")), 0.0);
    assert_eq!(regression(&ok, &pass), None);
    assert_eq!(regression(&failed, &pass).as_deref(), Some("a/x: expected pass, got fail (r)"));
    assert!(regression(&ok, &blocked).expect("stale").ends_with("the baseline is stale, so list this cell as pass"));
    assert!(regression(&failed, &blocked).expect("fail").contains("expected blocked, got fail"));
    let mut quiet = bare;
    quiet.error = None;
    assert_eq!(regression(&quiet, &pass).as_deref(), Some("a/x: expected pass, got fail"));
}

#[test]
fn the_table_numbers_each_distinct_reason_once_and_marks_a_blocked_cell_with_none() {
    let found = baseline(r#"{"stub": {}, "live": {}}"#).expect("baseline");
    let table = render_table(&found, &|role, harness| match (role, harness) {
        ("a", "x") => (Status::Pass, None),
        ("a", "y") => (Status::Blocked, Some("no model".to_string())),
        ("b", "x") => (Status::Fail, None),
        _ => (Status::Blocked, Some("no model".to_string())),
    });
    assert_eq!(
        table,
        "| role | x | y |\n|---|---|---|\n| a | pass | blocked [1] |\n| b | fail | blocked [1] |\n\n[1] y: no model"
    );
}

#[test]
fn the_docs_carry_the_table_the_baseline_promises_for_the_owners_machine() {
    let shipped = read_baseline(SHIPPED_BASELINE).expect("shipped");
    let doc = include_str!("../../../../../docs/model-gateway/harness-matrix.md");
    let start = doc.find("<!-- matrix:start -->\n").expect("start") + "<!-- matrix:start -->\n".len();
    let end = doc.find("\n<!-- matrix:end -->").expect("end");
    assert_eq!(&doc[start..end], expected_table(&shipped, true));
    assert!(expected_table(&shipped, false).contains("hermes"));
}
