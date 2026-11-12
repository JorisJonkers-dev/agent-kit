//! The `grant-check` subcommand: one Codex node start reserved through the
//! gateway, refused when it does not fit or the grant has lapsed.

use super::*;

const OK: &str = r#"{"status":"ok","reservation":{"run_id":"start-1","workflow":"w","client":"codex","node":"codex-build","grant":"grant-7","reserved_at":"2026-11-11T09:30:00Z","cap":3,"spent":0,"reserved":1,"remaining":2}}"#;

fn error_of(code: &str, message: &str) -> String {
    format!(r#"{{"status":"error","code":"{code}","message":"{message}"}}"#)
}

/// A work-instance grant-check case whose gateway answers `answer`.
fn check_case(tag: &str, answer: Result<(i32, String, String), String>) -> Case {
    let mut c = case(tag, "gold-band", &[NEDAP]);
    c.host.gateway = answer;
    c.unset("instance");
    c.unset("workflow");
    c.unset("families");
    c.set("node", "codex-build");
    c.set("gateway", "/opt/fake-gateway");
    c
}

fn grant_line(c: &Case) -> Vec<String> {
    let mut line = vec!["grant-check".to_string()];
    for (flag, value) in &c.args {
        line.push(format!("--{flag}"));
        line.push(value.clone());
    }
    line
}

fn go(c: &Case) -> Outcome {
    c.host.out.borrow_mut().clear();
    c.host.err.borrow_mut().clear();
    let code = run_with(grant_line(c).iter().map(OsString::from), &c.host);
    let report = gold_band_report::read(&c.dir.join("reports").join(GRANT_CHECK_REPORT_FILE)).ok();
    Outcome {
        code,
        out: c.host.out.borrow().clone(),
        err: c.host.err.borrow().clone(),
        report,
    }
}

#[test]
fn a_reserved_start_passes_and_names_the_grant_and_what_is_left() {
    let c = check_case("gc-pass", Ok((0, OK.to_string(), String::new())));
    let out = go(&c);
    assert_eq!(out.code, 0, "{}", out.err);
    assert_eq!(
        out.out,
        "gold-band-run-start: grant-check: ok reserved a start of grant grant-7 for node codex-build (2 left)\n"
    );
    let report = out.report.expect("a report is written");
    assert_eq!((report.exit_code, report.failed_check), (0, None));
    assert_eq!(report.run_id, "run-9");
    assert_eq!(report.attempt, 3);
    assert!(report
        .command
        .starts_with("gold-band-run-start grant-check "));
}

#[test]
fn the_gateway_gets_the_runs_workflow_name_the_codex_client_and_the_node() {
    let c = check_case("gc-call", Ok((0, OK.to_string(), String::new())));
    assert_eq!(go(&c).code, 0);
    let calls = c.host.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "/opt/fake-gateway");
    assert_eq!(&calls[0].1[..3], ["grant", "reserve", "--json"]);
    let sent: Value = serde_json::from_str(&calls[0].1[3]).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({
            "workflow": "build-feature@github.com/nedap/healthcare@run-9",
            "client": "codex",
            "node": "codex-build",
            "attempt": 3
        })
    );
}

#[test]
fn the_gateway_defaults_to_the_environment_then_to_model_gateway() {
    let mut c = check_case("gc-env", Ok((0, OK.to_string(), String::new())));
    c.unset("gateway");
    c.host.env.push((GATEWAY_ENV, "/from/env".to_string()));
    assert_eq!(go(&c).code, 0);
    assert_eq!(c.host.calls.borrow()[0].0, "/from/env");
    c.host.env.clear();
    c.host.calls.borrow_mut().clear();
    assert_eq!(go(&c).code, 0);
    assert_eq!(c.host.calls.borrow()[0].0, "model-gateway");
}

#[test]
fn a_lapsed_or_missing_grant_fails_the_node() {
    let body = error_of("no-grant", "grant g lapsed (expired)");
    let c = check_case("gc-lapsed", Ok((1, body, String::new())));
    let out = go(&c);
    let err = out.refused("grant-lapsed").to_string();
    assert!(err.contains("grant g lapsed (expired)"), "{err}");
    assert!(
        err.contains("build-feature@github.com/nedap/healthcare@run-9"),
        "{err}"
    );
    assert!(err.contains("node codex-build"), "{err}");
}

#[test]
fn a_grant_with_no_start_left_refuses_the_reservation() {
    let body = error_of("cap-exceeded", "no start left");
    let c = check_case("gc-full", Ok((1, body, String::new())));
    go(&c).refused("grant-no-start-left");
}

#[test]
fn any_other_gateway_error_refuses_and_never_passes() {
    let body = error_of("ledger-unreadable", "the ledger cannot be read");
    let c = check_case("gc-other", Ok((1, body, String::new())));
    let out = go(&c);
    assert!(out
        .refused("grant-reserve")
        .contains("the ledger cannot be read"));
    // An error object with no code or message is still a refusal.
    let c = check_case(
        "gc-bare",
        Ok((1, r#"{"status":"error"}"#.to_string(), "boom".to_string())),
    );
    go(&c).refused("grant-reserve");
    // Exit 0 with a status that is not ok never passes.
    let c = check_case(
        "gc-notok",
        Ok((
            0,
            r#"{"status":"error","code":"no-grant"}"#.to_string(),
            String::new(),
        )),
    );
    go(&c).refused("grant-lapsed");
}

#[test]
fn an_ok_answer_without_a_reservation_never_passes() {
    for (tag, body) in [
        ("gc-bare-ok", r#"{"status":"ok"}"#),
        (
            "gc-no-grant",
            r#"{"status":"ok","reservation":{"remaining":1}}"#,
        ),
        (
            "gc-empty-grant",
            r#"{"status":"ok","reservation":{"grant":"","remaining":1}}"#,
        ),
        (
            "gc-no-left",
            r#"{"status":"ok","reservation":{"grant":"g"}}"#,
        ),
    ] {
        let c = check_case(tag, Ok((0, body.to_string(), String::new())));
        go(&c).refused("grant-reserve");
    }
}

#[test]
fn a_nonzero_exit_with_an_ok_status_never_passes() {
    let c = check_case("gc-exit", Ok((1, OK.to_string(), String::new())));
    go(&c).refused("grant-reserve");
}

#[test]
fn a_gateway_that_cannot_start_or_prints_no_json_refuses() {
    let c = check_case(
        "gc-start",
        Err("cannot run /opt/fake-gateway: not found".to_string()),
    );
    assert!(go(&c).refused("grant-reserve").contains("not found"));
    let c = check_case(
        "gc-garbage",
        Ok((1, "  \n".to_string(), "it broke\n".to_string())),
    );
    assert!(go(&c).refused("grant-reserve").contains("it broke"));
    let c = check_case("gc-nothing", Ok((1, String::new(), String::new())));
    assert!(go(&c).refused("grant-reserve").contains("nothing"));
    let c = check_case(
        "gc-text",
        Ok((0, "not json\nat all".to_string(), String::new())),
    );
    assert!(go(&c).refused("grant-reserve").contains("not json at all"));
}

#[test]
fn a_repository_with_no_usable_remote_cannot_name_the_run() {
    let mut c = check_case("gc-noremote", Ok((0, OK.to_string(), String::new())));
    c.host.remotes = Ok(Vec::new());
    go(&c).refused("repository-rule");
    assert!(c.host.calls.borrow().is_empty());
}

#[test]
fn an_unreadable_repository_is_exit_2_with_a_failed_report() {
    let mut c = check_case("gc-norepo", Ok((0, OK.to_string(), String::new())));
    c.host.remotes = Err("not a repository".to_string());
    go(&c).bad_input("input");
}

#[test]
fn bad_usage_exits_2_and_still_reports_when_it_can() {
    let mut c = check_case("gc-usage", Ok((0, OK.to_string(), String::new())));
    c.set("bogus", "1");
    let out = go(&c);
    assert!(out
        .bad_input("usage")
        .contains("usage: gold-band-run-start grant-check"));
    // A missing node, a bad attempt and a repeated flag.
    let mut c = check_case("gc-nonode", Ok((0, OK.to_string(), String::new())));
    c.unset("node");
    assert!(go(&c).bad_input("usage").contains("--node is required"));
    let mut c = check_case("gc-attempt", Ok((0, OK.to_string(), String::new())));
    c.set("attempt", "many");
    assert!(go(&c).bad_input("usage").contains("--attempt must be"));
    let c = check_case("gc-repeat", Ok((0, OK.to_string(), String::new())));
    let mut line = grant_line(&c);
    line.extend(["--node".to_string(), "again".to_string()]);
    c.host.err.borrow_mut().clear();
    assert_eq!(run_with(line.iter().map(OsString::from), &c.host), 2);
    // No report directory: nothing to write, still exit 2.
    let mut c = check_case("gc-nodir", Ok((0, OK.to_string(), String::new())));
    c.unset("report-dir");
    assert_eq!(go(&c).code, 2);
    assert!(c.host.calls.borrow().is_empty());
}

#[test]
fn an_unusable_report_directory_is_exit_2_whatever_the_gateway_said() {
    let mut c = check_case("gc-baddir", Ok((0, OK.to_string(), String::new())));
    let file = c.path("a-file");
    fs::write(&file, "x").unwrap();
    c.set("report-dir", &c.path("a-file/sub"));
    assert_eq!(go(&c).code, 2);
}

#[test]
fn the_other_subcommand_name_is_a_usage_error() {
    let c = check_case("gc-sub", Ok((0, OK.to_string(), String::new())));
    let code = run_with(["nope".to_string()].iter().map(OsString::from), &c.host);
    assert_eq!(code, 2);
    assert!(c
        .host
        .err
        .borrow()
        .contains("the subcommands are check and grant-check"));
}

#[test]
fn a_stale_report_never_survives_a_refused_check() {
    let c = check_case(
        "gc-stale",
        Ok((1, error_of("no-grant", "lapsed"), String::new())),
    );
    fs::create_dir_all(c.dir.join("reports")).unwrap();
    fs::write(
        c.dir.join("reports").join(GRANT_CHECK_REPORT_FILE),
        r#"{"exit_code":0}"#,
    )
    .unwrap();
    let out = go(&c);
    assert_eq!(out.report.expect("fresh report").exit_code, 1);
}
