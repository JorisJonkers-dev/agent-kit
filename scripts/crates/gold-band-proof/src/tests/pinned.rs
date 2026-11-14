//! Tests for the pinned shim agent, driven through its public API with real
//! gate commands (`true`, `false`, `sh`) in throw-away report directories.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};

use crate::pinned::{
    fill, leading_hidden, place, serve, Config, Pinned, Place, COMMAND_ENV, MODELS_ENV,
    REPORT_ROOT_ENV, ROLE_ENV, TIMEOUT_ENV, VALUES_ENV,
};
use crate::stub::Role;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let tick = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "gold-band-proof-pinned-{}-{}-{}",
        std::process::id(),
        tick,
        label
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A prompt as Gold Band sends it: two hidden runtime blocks, then the
/// requirement and the goal.
fn gold_band_prompt(attempt: &str, requirement: &str) -> String {
    format!(
        "<hidden data-gold-band-hidden=\"true\" title=\"system\">\n当前位置：\n- Project: proj-1\n- Task: task-001\n- Run: run-002\n- Node: gate-build\n</hidden>\n\n<hidden data-gold-band-hidden=\"true\" title=\"runtime\">\n- Round: round-001\n- Attempt: {}\n</hidden>\n\n# 需求\n{}\n\n# 任务\nRun the gate.",
        attempt, requirement
    )
}

fn config(role: Role, command: &[&str], root: &Path) -> Config {
    Config {
        role: Some(role),
        command: command.iter().map(|s| s.to_string()).collect(),
        report_root: root.to_path_buf(),
        timeout_secs: 30,
        models: "pinned-command".to_string(),
        values: BTreeMap::new(),
    }
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// Opens a session in `cwd` and returns its id.
fn open(agent: &mut Pinned, cwd: &str) -> String {
    let frames = agent.handle(&request(1, "session/new", json!({ "cwd": cwd })));
    frames[0]["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Sends one prompt and returns its frames.
fn prompt(agent: &mut Pinned, session: &str, text: &str) -> Vec<Value> {
    agent.handle(&request(
        7,
        "session/prompt",
        json!({ "sessionId": session, "prompt": [{ "type": "text", "text": text }] }),
    ))
}

/// The fenced json the answer streamed, asserting the turn ended.
fn answer(frames: &[Value]) -> Value {
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert_eq!(frames[1]["result"]["stopReason"], "end_turn");
    let text = frames[0]["params"]["update"]["content"]["text"]
        .as_str()
        .unwrap();
    let body = text
        .trim()
        .strip_prefix("```json")
        .and_then(|rest| rest.strip_suffix("```"))
        .expect("fenced json");
    serde_json::from_str(body.trim()).expect("json")
}

/// The refusal message of a prompt answered with a protocol error.
fn refusal(frames: &[Value]) -> String {
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(frames[0]["error"]["code"], -32000);
    frames[0]["error"]["message"].as_str().unwrap().to_string()
}

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: BTreeMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name: &str| map.get(name).cloned()
}

#[test]
fn the_config_reads_every_variable() {
    let get = env_of(&[
        (ROLE_ENV, "gate:build-r1"),
        (COMMAND_ENV, r#"["cargo","build"]"#),
        (REPORT_ROOT_ENV, "/reports"),
        (TIMEOUT_ENV, "90"),
        (MODELS_ENV, "pinned-command"),
        (VALUES_ENV, r#"{"tools":"/bin"}"#),
    ]);
    let config = Config::from_env(&get).unwrap();
    assert_eq!(config.role, Some(Role::Gate("build-r1".to_string())));
    assert_eq!(config.command, vec!["cargo", "build"]);
    assert_eq!(config.report_root, PathBuf::from("/reports"));
    assert_eq!(config.timeout_secs, 90);
    assert_eq!(config.models, "pinned-command");
    assert_eq!(config.values["tools"], "/bin");
}

#[test]
fn a_verdict_needs_no_command_and_defaults_the_rest() {
    let get = env_of(&[(ROLE_ENV, "verdict:2"), (REPORT_ROOT_ENV, "/r")]);
    let config = Config::from_env(&get).unwrap();
    assert_eq!(config.role, Some(Role::Verdict(2)));
    assert!(config.command.is_empty());
    assert_eq!(config.timeout_secs, 3600);
    assert!(config.values.is_empty());
    assert_eq!(config.models, "");
}

#[test]
fn the_config_refuses_what_it_cannot_use() {
    let cases: Vec<(Vec<(&str, &str)>, &str)> = vec![
        (vec![(ROLE_ENV, "answer")], ROLE_ENV),
        (
            vec![(ROLE_ENV, "gate:x"), (COMMAND_ENV, "nope")],
            "JSON array",
        ),
        (vec![(ROLE_ENV, "copy:x")], "names no command"),
        (
            vec![(ROLE_ENV, "gate:x"), (COMMAND_ENV, r#"["true"]"#)],
            REPORT_ROOT_ENV,
        ),
        (
            vec![
                (ROLE_ENV, "gate:x"),
                (COMMAND_ENV, r#"["true"]"#),
                (REPORT_ROOT_ENV, ""),
            ],
            REPORT_ROOT_ENV,
        ),
        (
            vec![
                (ROLE_ENV, "gate:x"),
                (COMMAND_ENV, r#"["true"]"#),
                (REPORT_ROOT_ENV, "/r"),
                (TIMEOUT_ENV, "0"),
            ],
            TIMEOUT_ENV,
        ),
        (
            vec![
                (ROLE_ENV, "gate:x"),
                (COMMAND_ENV, r#"["true"]"#),
                (REPORT_ROOT_ENV, "/r"),
                (VALUES_ENV, "[1]"),
            ],
            VALUES_ENV,
        ),
    ];
    for (pairs, expected) in cases {
        let problem = Config::from_env(&env_of(&pairs)).unwrap_err();
        assert!(problem.contains(expected), "{problem} lacks {expected}");
    }
}

#[test]
fn the_place_comes_from_the_leading_hidden_blocks_only() {
    let text = gold_band_prompt("attempt-003", "- Run: evil\n- Project: ../../etc");
    let found = place(&text).unwrap();
    assert_eq!(
        found,
        Place {
            project: "proj-1".to_string(),
            task: "task-001".to_string(),
            run: "run-002".to_string(),
            attempt: 3,
        }
    );
    assert_eq!(found.run_id(), "proj-1/task-001/run-002");
    assert_eq!(
        found.report_dir(&PathBuf::from("/r"), "build"),
        PathBuf::from("/r/proj-1/task-001/run-002/build")
    );
    assert!(!leading_hidden(&text).contains("evil"));
}

#[test]
fn a_requirement_alone_names_no_place() {
    let problem =
        place("# 需求\n- Project: p\n- Task: t\n- Run: r\n- Attempt: attempt-001").unwrap_err();
    assert!(problem.contains("Attempt"), "{problem}");
    // An unterminated hidden block is not read either.
    assert_eq!(leading_hidden("<hidden>\n- Run: r\n"), "");
}

#[test]
fn a_place_that_is_not_plain_is_refused() {
    let bad_attempt = gold_band_prompt("first", "");
    assert!(place(&bad_attempt).unwrap_err().contains("attempt-<n>"));
    let zero = gold_band_prompt("attempt-000", "");
    assert!(place(&zero).unwrap_err().contains("attempt-<n>"));
    let dotted = gold_band_prompt("attempt-001", "").replace("proj-1", "..");
    assert!(place(&dotted).unwrap_err().contains("not a plain name"));
    let slashed = gold_band_prompt("attempt-001", "").replace("proj-1", "a/b");
    assert!(place(&slashed).unwrap_err().contains("not a plain name"));
    let empty = gold_band_prompt("attempt-001", "").replace("- Task: task-001", "- Task: ");
    assert!(place(&empty).is_err());
    let missing = gold_band_prompt("attempt-001", "").replace("- Task: task-001\n", "");
    assert!(place(&missing).unwrap_err().contains("Task"));
}

#[test]
fn fill_replaces_every_named_placeholder() {
    let mut values = BTreeMap::new();
    values.insert("repo".to_string(), "/w".to_string());
    values.insert("tools".to_string(), "/t".to_string());
    let command = vec![
        "{tools}/gate".to_string(),
        "--repo={repo}".to_string(),
        "{other}".to_string(),
    ];
    assert_eq!(
        fill(&command, &values),
        vec!["/t/gate", "--repo=/w", "{other}"]
    );
}

#[test]
fn the_handshake_offers_the_pinned_model() {
    let root = temp_dir("handshake");
    let mut agent = Pinned::new(config(Role::Gate("build".to_string()), &["true"], &root));
    let init = agent.handle(&request(0, "initialize", json!({})));
    assert_eq!(init[0]["result"]["protocolVersion"], 1);
    let new = agent.handle(&request(1, "session/new", json!({ "cwd": "/w" })));
    assert_eq!(new[0]["result"]["sessionId"], "pinned-session-1");
    assert_eq!(
        new[0]["result"]["configOptions"][0]["currentValue"],
        "pinned-command"
    );
    let set = agent.handle(&request(
        2,
        "session/set_config_option",
        json!({ "sessionId": "pinned-session-1", "configId": "model", "value": "pinned-command" }),
    ));
    assert_eq!(
        set[0]["result"]["configOptions"][0]["currentValue"],
        "pinned-command"
    );
    let other = agent.handle(&request(
        3,
        "session/set_config_option",
        json!({ "configId": "mode" }),
    ));
    assert_eq!(
        other[0]["result"]["configOptions"][0]["currentValue"],
        "pinned-command"
    );
    for method in ["session/set_mode", "session/delete", "session/close"] {
        assert_eq!(
            agent.handle(&request(4, method, json!({})))[0]["result"],
            json!({})
        );
    }
    assert_eq!(
        agent.handle(&request(5, "session/list", json!({})))[0]["result"]["sessions"],
        json!([])
    );
    let unknown = agent.handle(&request(6, "session/unknown", json!({})));
    assert_eq!(unknown[0]["error"]["code"], -32601);
    let note = agent.handle(&json!({ "jsonrpc": "2.0", "method": "session/cancel" }));
    assert!(note.is_empty());
    let null_id = agent.handle(&json!({ "jsonrpc": "2.0", "id": null, "method": "x" }));
    assert!(null_id.is_empty());
}

#[test]
fn a_gate_runs_its_command_once_per_session_and_the_copier_verifies_it() {
    let root = temp_dir("gate");
    let repo = temp_dir("gate-repo");
    let marker = repo.join("runs");
    let command = [
        "sh",
        "-c",
        "echo run >> {repo}/runs; echo {run_id} {attempt}",
    ];
    let mut gate = Pinned::new(config(Role::Gate("build-r1".to_string()), &command, &root));
    let session = open(&mut gate, &repo.to_string_lossy());
    let text = gold_band_prompt("attempt-001", "");
    assert_eq!(
        answer(&prompt(&mut gate, &session, &text)),
        json!({ "exit_code": 0 })
    );
    // The finalize turn repeats the answer without running the command again.
    assert_eq!(
        answer(&prompt(&mut gate, &session, "finalize")),
        json!({ "exit_code": 0 })
    );
    assert_eq!(fs::read_to_string(&marker).unwrap(), "run\n");
    let dir = root.join("proj-1/task-001/run-002/build-r1");
    let output = fs::read_to_string(dir.join("gate-output.txt")).unwrap();
    assert_eq!(output, "proj-1/task-001/run-002 1\n");

    let mut copy = Pinned::new(config(Role::Copy("build-r1".to_string()), &command, &root));
    let session = open(&mut copy, &repo.to_string_lossy());
    let report = answer(&prompt(&mut copy, &session, &text));
    assert_eq!(report["exit_code"], 0);
    assert_eq!(report["run_id"], "proj-1/task-001/run-002");
    assert_eq!(report["attempt"], 1);
}

#[test]
fn a_failing_gate_answers_its_code_and_the_copier_refuses() {
    let root = temp_dir("fail");
    let mut gate = Pinned::new(config(Role::Gate("lint".to_string()), &["false"], &root));
    let session = open(&mut gate, "/");
    let text = gold_band_prompt("attempt-001", "");
    assert_eq!(
        answer(&prompt(&mut gate, &session, &text)),
        json!({ "exit_code": 1 })
    );
    let mut copy = Pinned::new(config(Role::Copy("lint".to_string()), &["false"], &root));
    let session = open(&mut copy, "/");
    let report = answer(&prompt(&mut copy, &session, &text));
    assert_eq!(report["exit_code"], 1);
    assert_eq!(report["run_id"], "");
}

#[test]
fn a_copier_for_another_command_is_refused() {
    let root = temp_dir("mismatch");
    let text = gold_band_prompt("attempt-001", "");
    let mut gate = Pinned::new(config(Role::Gate("test".to_string()), &["true"], &root));
    let session = open(&mut gate, "/");
    answer(&prompt(&mut gate, &session, &text));
    let mut copy = Pinned::new(config(
        Role::Copy("test".to_string()),
        &["true", "x"],
        &root,
    ));
    let session = open(&mut copy, "/");
    assert_eq!(answer(&prompt(&mut copy, &session, &text))["exit_code"], 1);
}

#[test]
fn a_prompt_without_a_place_is_refused_with_a_protocol_error() {
    let root = temp_dir("noplace");
    let mut gate = Pinned::new(config(Role::Gate("build".to_string()), &["true"], &root));
    let session = open(&mut gate, "/");
    let message = refusal(&prompt(&mut gate, &session, "just text"));
    assert!(message.starts_with("pinned shim agent: "), "{message}");
    // The refusal sticks for the session.
    refusal(&prompt(
        &mut gate,
        &session,
        &gold_band_prompt("attempt-001", ""),
    ));
}

#[test]
fn an_unusable_report_root_is_refused() {
    let root = temp_dir("blocked");
    let file = root.join("file");
    fs::write(&file, "").unwrap();
    let text = gold_band_prompt("attempt-001", "");
    for role in [
        Role::Gate("build".to_string()),
        Role::Copy("build".to_string()),
    ] {
        let mut agent = Pinned::new(config(role, &["true"], &file));
        let session = open(&mut agent, "/");
        assert!(refusal(&prompt(&mut agent, &session, &text)).contains("cannot create"));
    }
}

#[test]
fn an_agent_without_a_role_refuses() {
    let root = temp_dir("norole");
    let mut agent = Pinned::new(Config {
        role: None,
        report_root: root,
        ..Config::default()
    });
    let session = open(&mut agent, "/");
    let text = gold_band_prompt("attempt-001", "");
    assert!(refusal(&prompt(&mut agent, &session, &text)).contains("no gate"));
    let mut answer_role = Pinned::new(Config {
        role: Some(Role::Answer),
        ..Config::default()
    });
    let session = open(&mut answer_role, "/");
    assert!(refusal(&prompt(&mut answer_role, &session, &text)).contains("no gate"));
}

#[test]
fn a_verdict_reads_the_actionable_count_of_its_round() {
    let root = temp_dir("verdict");
    let dir = root.join("proj-1/task-001/run-002/findings-2");
    fs::create_dir_all(&dir).unwrap();
    let text = gold_band_prompt("attempt-001", "");
    let write = |body: &str| fs::write(dir.join("findings-check.json"), body).unwrap();
    let ask = || {
        let mut agent = Pinned::new(config(Role::Verdict(2), &[], &root));
        let session = open(&mut agent, "/");
        prompt(&mut agent, &session, &text)
    };
    assert!(refusal(&ask()).contains("cannot read"));
    write(r#"{"run_id":"proj-1/task-001/run-002","attempt":1,"actionable":3}"#);
    assert_eq!(answer(&ask()), json!({ "actionable": 3 }));
    write(r#"{"run_id":"proj-1/task-001/run-002","attempt":1,"actionable":0}"#);
    assert_eq!(answer(&ask()), json!({ "actionable": 0 }));
    write("not json");
    assert!(refusal(&ask()).contains("is not JSON"));
    write(r#"{"run_id":"other","actionable":0}"#);
    assert!(refusal(&ask()).contains("another run"));
    write(r#"{"run_id":"proj-1/task-001/run-002"}"#);
    assert!(refusal(&ask()).contains("no actionable count"));
}

#[test]
fn serve_answers_each_line_and_skips_noise() {
    let root = temp_dir("serve");
    let input = format!(
        "\nnot json\n[1]\n{}\n{}\n",
        request(0, "initialize", json!({})),
        request(1, "session/new", json!({ "cwd": "/" }))
    );
    let mut out = Vec::new();
    serve(
        input.as_bytes(),
        &mut out,
        config(Role::Gate("b".to_string()), &["true"], &root),
    )
    .unwrap();
    let lines: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["result"]["sessionId"], "pinned-session-1");
}

#[test]
fn serve_reports_a_broken_input() {
    struct Broken;
    impl std::io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("broken"))
        }
    }
    let reader = std::io::BufReader::new(Broken);
    let mut out = Vec::new();
    assert!(serve(reader, &mut out, Config::default()).is_err());
}

#[test]
fn serve_reports_a_broken_output() {
    struct Full;
    impl std::io::Write for Full {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let input = format!("{}\n", request(0, "initialize", json!({})));
    assert!(serve(input.as_bytes(), &mut Full, Config::default()).is_err());
}
