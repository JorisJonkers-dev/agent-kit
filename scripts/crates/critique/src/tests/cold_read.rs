//! The cold read's reply parser and path joining, and the one branch the goldens cannot reach: a gateway ledger that
//! takes the plan reading but not the record.

use crate::cold_read::{answered_file, missing_answers, parse_cold_read_reply, run, unavailable_file};
use crate::paths::join;
use crate::Runtime;
use model_gateway::adapters::fs::SystemFiles;
use model_gateway::json::{parse, Value};
use model_gateway::ports::{FileStamp, Files, FsError, Process, ProcessCommand, ProcessResult};
use std::collections::HashMap;

fn answers(text: &str) -> Option<[String; 3]> {
    parse_cold_read_reply(text)
}

fn some(decided: &str, rejected: &str, revisit: &str) -> Option<[String; 3]> {
    Some([decided.to_string(), rejected.to_string(), revisit.to_string()])
}

#[test]
fn the_last_fenced_json_block_is_the_answer() {
    let reply = "```json\n{\"decided\":\"first\"}\n```\ntext\n```json\n{\"decided\":\" last \",\"rejected\":\"r\"}\n```\n";
    assert_eq!(answers(reply), some("last", "r", ""));
}

#[test]
fn a_fence_may_carry_blanks_and_carriage_returns_and_close_without_a_newline() {
    assert_eq!(answers("```json \t\r\n{\"decided\":\"a\"}\r\n```"), some("a", "", ""));
    assert_eq!(answers("```json\n{\"decided\":\"b\"}```"), some("b", "", ""));
    assert_eq!(answers("```json\n{\"decided\":\"c\"}\r```"), some("c", "", ""));
    assert_eq!(answers("```json\r\n{\"decided\":\"d\"}\n\r```"), some("d", "", ""));
}

#[test]
fn a_reply_without_a_readable_block_gives_no_answer() {
    for reply in [
        "",
        "{\"decided\":\"a\"}",
        "```json x\n{\"decided\":\"a\"}\n```",
        "```json\r{\"decided\":\"a\"}\n```",
        "```json\n{\"decided\":\"a\"}",
        "```json\n{nope}\n```",
        "```json\n[\"decided\"]\n```",
        "```json\n{\"decided\":null}\n```",
        "```json\n{\"revisit\":7}\n```",
    ] {
        assert_eq!(answers(reply), None, "{reply:?}");
    }
}

#[test]
fn an_unclosed_block_does_not_hide_an_earlier_closed_one() {
    let reply = "```json x\n{\"decided\":\"no\"}\n```\n```json\n{\"decided\":\"yes\"}\n```\n```json\n{\"decided\":\"open\"}";
    assert_eq!(answers(reply), some("yes", "", ""));
}

#[test]
fn answers_left_empty_are_missing_in_the_order_the_role_asks() {
    let all = ["a".to_string(), "b".to_string(), "c".to_string()];
    assert!(missing_answers(&all).is_empty());
    let some = ["".to_string(), "b".to_string(), "".to_string()];
    assert_eq!(missing_answers(&some), vec!["decided", "revisit"]);
    let file = answered_file("x.md", &some);
    assert_eq!(missing_names(&file), vec!["decided", "revisit"]);
    assert_eq!(unavailable_file("x.md", "why").as_object().and_then(|file| file.get("reason")), Some(&Value::str("why")));
}

fn missing_names(file: &Value) -> Vec<String> {
    match file.as_object().and_then(|file| file.get("missing")) {
        Some(Value::Arr(items)) => items.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn join_keeps_a_relative_path_relative_and_normalises_like_node() {
    assert_eq!(join(&["artifact", "a.md"]), "artifact/a.md");
    assert_eq!(join(&["/a/b", "../c", "./d"]), "/a/c/d");
    assert_eq!(join(&["a", "..", ".."]), "..");
    assert_eq!(join(&["/", ".."]), "/");
    assert_eq!(join(&["/a", ".."]), "/");
    assert_eq!(join(&["a", ".."]), ".");
    assert_eq!(join(&["", ""]), ".");
    assert_eq!(join(&["a/", "b/"]), "a/b/");
    assert_eq!(join(&["", "a"]), "a");
    assert_eq!(join(&["b", "..", "./"]), "./");
    assert_eq!(crate::paths::inside_repository("/a/", ".").as_deref(), Ok(""));
    assert_eq!(crate::paths::inside_repository("/a/", "x/y").as_deref(), Ok("x/y"));
}

/// A set of files that delegates to the system's, except that it will not append.
struct Refusing(SystemFiles);

impl Files for Refusing {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        self.0.read_text(path)
    }
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_atomic(path, content)
    }
    fn mkdir_all(&self, path: &str) -> Result<(), FsError> {
        self.0.mkdir_all(path)
    }
    fn create_exclusive(&self, path: &str) -> Result<bool, FsError> {
        self.0.create_exclusive(path)
    }
    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        self.0.remove_file(path)
    }
    fn modified_ms(&self, path: &str) -> Option<i64> {
        self.0.modified_ms(path)
    }
    fn exists(&self, path: &str) -> Result<bool, FsError> {
        self.0.exists(path)
    }
    fn append(&self, path: &str, _content: &str) -> Result<(), FsError> {
        Err(FsError { code: "EACCES".to_string(), message: format!("EACCES: permission denied, open '{path}'") })
    }
    fn stamp(&self, path: &str) -> Option<FileStamp> {
        self.0.stamp(path)
    }
    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_private(path, content)
    }
    fn write_text(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_text(path, content)
    }
    fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, FsError> {
        self.0.temp_dir(parent, prefix)
    }
    fn remove(&self, path: &str) -> Result<(), FsError> {
        self.0.remove(path)
    }
}

/// A `vibe` that writes the export beside its `--output-dir` and answers with a journal.
struct Answering;

impl Process for Answering {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }

    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Err("no keyring".to_string())
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        let at = command.args.iter().position(|arg| arg == "--output-dir").ok_or("no output dir")?;
        let dir = &command.args[at + 1];
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        let export = r#"{"config":{"enable_mcp":false,"enabled_tools":["read_file"],"include_project_context":false,"outside_workdir":"deny"}}"#;
        std::fs::write(format!("{dir}/export.json"), export).map_err(|error| error.to_string())?;
        let reply = "```json\n{\"decided\":\"a\"}\n```";
        let journal = format!(r#"[{{"type":"message","role":"assistant","content":[{{"text":{}}}]}}]"#, model_gateway::json::compact(&Value::str(reply)));
        Ok(ProcessResult { stdout: journal, stderr: String::new(), exit_code: 0, timed_out: false })
    }
}

#[test]
fn a_gateway_ledger_that_will_not_take_a_record_is_a_warning_in_the_result() {
    let sandbox = super::scenarios::Sandbox::new();
    let root = sandbox.text();
    let write = |path: &str, text: &str| {
        let target = sandbox.path.join(path);
        assert!(std::fs::create_dir_all(target.parent().unwrap_or(&sandbox.path)).is_ok());
        assert!(std::fs::write(target, text).is_ok());
    };
    write("repo/docs/a.md", "# A\n");
    write("home/.vibe/.env", "MISTRAL_API_KEY=k\n");
    write("home/.vibe/agents/critic-mistral-small.toml", "");
    write("bin/vibe", "");
    let manifest = format!(
        r#"{{"allowed_families":["mistral"],"artifact":{{"labels":[],"reference":"o/r#1","tracker":"github"}},"created_at":"2026-10-30T12:00:00.000Z","critics":[],"depth":"base","depth_choice":{{"chosen_by":"argument","measures":{{}},"signals":[]}},"distinct_families":true,"dropped":[],"flow":"f","limits":{{"antigravity_quota":{{"daily_cap":20,"reserve_threshold":10,"reserved_stages":["spec"]}},"search":{{"max_bytes":8192,"max_matches":50}}}},"orchestrator_family":"anthropic","packet_dir":"{root}/run/packet","packet_files":[],"repo_root":"{root}/repo","repository":"o/r","run_dir":"{root}/run","run_id":"r1","stage":"spec","version":1}}"#
    );
    write("run/manifest.json", &manifest);
    let mut runtime: Runtime = super::cli::runtime("");
    runtime.env = HashMap::from([
        ("HOME".to_string(), format!("{root}/home")),
        ("PATH".to_string(), format!("{root}/bin")),
        ("XDG_STATE_HOME".to_string(), format!("{root}/state")),
    ]);
    runtime.files = Box::new(SystemFiles { cwd: root.clone() });
    runtime.gateway.env = runtime.env.clone();
    runtime.gateway.files = Box::new(Refusing(SystemFiles { cwd: root.clone() }));
    runtime.gateway.process = Box::new(Answering);
    runtime.assets_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../skills/workstation/critique").to_string_lossy().into_owned();
    let input = parse(&format!(r#"{{"files":["docs/a.md"],"run_dir":"{root}/run"}}"#)).unwrap_or(Value::Null);
    let result = run(&input, &runtime).unwrap_or(Value::Null);
    let warnings = match result.as_object().and_then(|result| result.get("ledger_warnings")) {
        Some(Value::Arr(items)) => items.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    assert_eq!(warnings.len(), 1, "{}", model_gateway::json::pretty(&result));
    assert!(warnings[0].contains("EACCES"), "{warnings:?}");
}
