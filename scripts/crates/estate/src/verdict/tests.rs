use super::*;
use crate::testkit::{run_in, run_plain, Dir, Out};
use serde_json::{json, Value};

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn signals(compose: &str, status: &str, deployment: &str, publish: &str) -> Value {
    json!({
        "project": "home-portal",
        "release": "v1.2.3",
        "sha": SHA,
        "signals": {
            "publish": {"state": publish, "url": "https://example.test/publish/1", "headSha": SHA},
            "compose": {"state": compose, "url": "https://example.test/compose/2", "headSha": SHA, "codes": []},
            "status": {"state": status, "url": "https://example.test/status/3", "headSha": SHA},
            "deployment": {"state": deployment, "url": "https://example.test/deploy/4", "headSha": SHA},
            "issues": [],
        },
    })
}

fn good() -> Value {
    signals("clean", "success", "success", "success")
}

fn feed(data: &Value) -> Out {
    run_in(&["verdict"], &data.to_string(), "/nonexistent", None, None)
}

fn verdict_of(data: &Value) -> String {
    let out = feed(data);
    assert_eq!(out.code, 0, "{}", out.err);
    out.out
        .lines()
        .next()
        .unwrap()
        .strip_prefix("VERDICT: ")
        .unwrap()
        .to_string()
}

#[test]
fn all_signals_agreeing_is_landed() {
    assert_eq!(verdict_of(&good()), "landed");
}

#[test]
fn the_deployment_and_publish_never_change_the_verdict() {
    for deployment in ["success", "failure", "in_progress", "missing"] {
        for publish in ["success", "failure", "missing"] {
            assert_eq!(
                verdict_of(&signals("clean", "success", deployment, publish)),
                "landed"
            );
            assert_eq!(
                verdict_of(&signals("clean", "pending", deployment, publish)),
                "mixed"
            );
            assert_eq!(
                verdict_of(&signals("errors", "pending", deployment, publish)),
                "not landed"
            );
        }
    }
}

#[test]
fn nothing_good_is_not_landed_whatever_the_deployment_says() {
    assert_eq!(
        verdict_of(&signals("errors", "failure", "missing", "success")),
        "not landed"
    );
    assert_eq!(
        verdict_of(&signals("errors", "failure", "success", "success")),
        "not landed"
    );
}

#[test]
fn the_verdict_rule_covers_every_pairing() {
    assert_eq!(verdict("clean", "success"), "landed");
    assert_eq!(verdict("errors", "failure"), "not landed");
    assert_eq!(verdict("clean", "pending"), "mixed");
    assert_eq!(verdict("clean", "failure"), "mixed");
    assert_eq!(verdict("errors", "success"), "mixed");
    assert_eq!(verdict("missing", "success"), "mixed");
}

#[test]
fn a_pending_status_with_a_clean_compose_is_mixed_with_every_link() {
    let out = feed(&signals("clean", "pending", "success", "success"));
    let lines: Vec<&str> = out.out.lines().collect();
    assert_eq!(lines[0], "VERDICT: mixed");
    for expected in [
        "status: pending  https://example.test/status/3",
        "deployment: success  https://example.test/deploy/4",
        "compose: clean  https://example.test/compose/2",
        "publish: success  https://example.test/publish/1",
        "  codes: none",
        "issues: none open",
    ] {
        assert!(lines.contains(&expected), "{expected}");
    }
}

#[test]
fn the_opposite_disagreement_is_mixed_too() {
    let mut data = signals("errors", "success", "missing", "success");
    data["signals"]["compose"]["codes"] = json!(["E_X", "E_Y"]);
    let out = feed(&data);
    assert_eq!(out.out.lines().next().unwrap(), "VERDICT: mixed");
    assert!(out
        .out
        .contains("compose: errors  https://example.test/compose/2"));
    assert!(out.out.contains("codes: E_X, E_Y"));
}

#[test]
fn a_head_sha_mismatch_counts_as_missing() {
    let mut data = good();
    data["signals"]["status"]["headSha"] = json!(OTHER);
    let out = feed(&data);
    assert_eq!(out.out.lines().next().unwrap(), "VERDICT: mixed");
    assert!(out.out.contains(&format!(
        "status: missing  https://example.test/status/3  (counted as missing: headSha {OTHER} differs from {SHA})"
    )));
}

#[test]
fn a_null_empty_or_absent_head_sha_is_not_a_mismatch() {
    let mut data = good();
    data["signals"]["status"]["headSha"] = Value::Null;
    data["signals"]["compose"]
        .as_object_mut()
        .unwrap()
        .remove("headSha");
    data["signals"]["publish"]["headSha"] = json!("");
    data["signals"]["compose"]
        .as_object_mut()
        .unwrap()
        .remove("codes");
    let out = feed(&data);
    assert_eq!(out.out.lines().next().unwrap(), "VERDICT: landed");
    assert!(!out.out.contains("counted as missing"));
}

#[test]
fn head_sha_is_compared_case_insensitively() {
    let mut data = good();
    data["signals"]["status"]["headSha"] = json!(SHA.to_uppercase());
    assert_eq!(verdict_of(&data), "landed");
    data["sha"] = json!(SHA.to_uppercase());
    data["signals"]["status"]["headSha"] = json!(SHA);
    assert_eq!(verdict_of(&data), "landed");
}

#[test]
fn open_issues_are_listed_with_links_and_do_not_change_the_verdict() {
    let mut data = good();
    data["signals"]["issues"] =
        json!([{"title": "home-portal/isolated", "url": "https://example.test/issues/9"}]);
    let out = feed(&data);
    let lines: Vec<&str> = out.out.lines().collect();
    assert_eq!(lines[0], "VERDICT: landed");
    assert!(lines.contains(&"issues: 1 open"));
    assert!(lines.contains(&"  home-portal/isolated  https://example.test/issues/9"));
}

#[test]
fn a_missing_issues_key_is_none_open_and_a_missing_url_is_an_empty_link() {
    let mut data = good();
    data["signals"].as_object_mut().unwrap().remove("issues");
    data["signals"]["status"]
        .as_object_mut()
        .unwrap()
        .remove("url");
    let out = feed(&data);
    assert!(out.out.contains("issues: none open"));
    assert!(out.out.contains("status: success  \n"));
}

#[test]
fn the_signals_can_come_from_a_file() {
    let dir = Dir::new();
    let path = dir.write("signals.json", &good().to_string());
    let out = run_plain(&["verdict", "--signals", path.to_str().unwrap()]);
    assert_eq!(out.code, 0);
    assert!(out.out.starts_with("VERDICT: landed"));
}

fn mutations() -> Vec<(&'static str, Value)> {
    let edit = |f: &dyn Fn(&mut Value)| {
        let mut d = good();
        f(&mut d);
        d
    };
    vec![
        ("not an object", json!(["list"])),
        ("empty project", edit(&|d| d["project"] = json!(""))),
        (
            "no release",
            edit(&|d| {
                d.as_object_mut().unwrap().remove("release");
            }),
        ),
        ("sha not a string", edit(&|d| d["sha"] = json!(5))),
        ("signals not an object", edit(&|d| d["signals"] = json!([]))),
        (
            "missing signal",
            edit(&|d| {
                d["signals"].as_object_mut().unwrap().remove("status");
            }),
        ),
        (
            "signal not an object",
            edit(&|d| d["signals"]["deployment"] = json!("success")),
        ),
        (
            "unknown state",
            edit(&|d| d["signals"]["status"]["state"] = json!("green")),
        ),
        (
            "url not a string",
            edit(&|d| d["signals"]["status"]["url"] = json!(3)),
        ),
        (
            "head sha not a string",
            edit(&|d| d["signals"]["status"]["headSha"] = json!(3)),
        ),
        (
            "head sha not 40 hex",
            edit(&|d| d["signals"]["status"]["headSha"] = json!("abc123")),
        ),
        (
            "head sha 40 non-hex",
            edit(&|d| d["signals"]["status"]["headSha"] = json!("z".repeat(40))),
        ),
        ("sha not 40 hex", edit(&|d| d["sha"] = json!("abc123"))),
        (
            "array state",
            edit(&|d| d["signals"]["status"]["state"] = json!([])),
        ),
        (
            "object state",
            edit(&|d| d["signals"]["compose"]["state"] = json!({})),
        ),
        (
            "codes not a list",
            edit(&|d| d["signals"]["compose"]["codes"] = json!("E_X")),
        ),
        (
            "codes not strings",
            edit(&|d| d["signals"]["compose"]["codes"] = json!([1])),
        ),
        (
            "issues not a list",
            edit(&|d| d["signals"]["issues"] = json!("none")),
        ),
        (
            "issue without url",
            edit(&|d| d["signals"]["issues"] = json!([{"title": "t"}])),
        ),
        (
            "issue not an object",
            edit(&|d| d["signals"]["issues"] = json!(["t"])),
        ),
    ]
}

#[test]
fn malformed_input_exits_2_with_a_message_and_no_verdict() {
    for (name, data) in mutations() {
        let out = feed(&data);
        assert_eq!(out.code, 2, "{name}");
        assert!(
            out.err.starts_with("estate verdict: malformed input: "),
            "{name}"
        );
        assert_eq!(out.out, "", "{name}");
        assert!(evaluate(&data).is_err(), "{name}");
    }
}

#[test]
fn the_state_error_lists_the_allowed_states() {
    let mut data = good();
    data["signals"]["status"]["state"] = json!("green");
    assert!(feed(&data).err.contains(
        "signals.status.state must be one of ['failure', 'missing', 'pending', 'success']"
    ));
}

#[test]
fn unreadable_input_exits_2() {
    let dir = Dir::new();
    let latin = dir.write_bytes("latin.json", b"{\"project\": \"caf\xe9\"}");
    let out = run_plain(&["verdict", "--signals", latin.to_str().unwrap()]);
    assert_eq!(out.code, 2);
    assert!(out.err.starts_with("estate verdict: malformed input: "));
    assert_eq!(run_in(&["verdict"], "{nope", "", None, None).code, 2);
    let absent = dir.join("absent.json");
    let out = run_plain(&["verdict", "--signals", absent.to_str().unwrap()]);
    assert_eq!(out.code, 2);
    assert!(out.err.contains("malformed input"));
}

#[test]
fn usage_mistakes_and_help() {
    assert_eq!(run_plain(&["verdict", "extra"]).code, 2);
    assert_eq!(run_plain(&["verdict", "-h"]).code, 0);
}

#[test]
fn a_stdin_that_cannot_be_read_is_malformed() {
    struct Broken;
    impl std::io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed"))
        }
    }
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = crate::run(
        &["verdict".to_string()],
        &mut crate::Ctx {
            out: &mut out,
            err: &mut err,
            input: &mut Broken,
            path: String::new(),
            home: None,
            timeout: None,
        },
    );
    assert_eq!(code, 2);
    assert!(String::from_utf8_lossy(&err).contains("closed"));
}
