//! Tests for the command line of `ps`, `status`, `logs`, `cancel` and `gc`.

use super::machine::{Machine, DEFAULTS};
use crate::cli::run_cli;
use crate::json::parse;
use crate::run_events::{RunEvents, RunIdentity, RunStart};

fn cli(machine: &Machine, args: &[&str]) -> (i32, String, String) {
    let argv: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let result = run_cli(&argv, &machine.runtime);
    (result.exit_code, result.stdout, result.stderr)
}

fn start<'a>(machine: &'a Machine, id: &str) -> RunEvents<'a> {
    let dir = format!("{}/state/agent-kit/model-gateway/runs/{id}", machine.sandbox.text());
    std::fs::create_dir_all(&dir).expect("run dir");
    let identity = RunIdentity { run_id: id.into(), workflow: "w".into(), client: "pi".into(), model: "free/hy4-flash".into(), family: "tencent".into() };
    let index = crate::locations::run_index_path(&machine.runtime.env).expect("index path");
    RunEvents::start(machine.runtime.files.as_ref(), machine.runtime.clock.as_ref(), &dir, identity, &RunStart { mode: "read-only", time_limit_s: 900.0 }).indexed(Some(&index))
}

#[test]
fn the_status_commands_print_tables_and_text_and_with_json_one_object() {
    let machine = Machine::new(DEFAULTS);
    let events = start(&machine, "run-one");
    events.finish(parse(r#"{"status":"ok","exit_code":0}"#).expect("JSON"));
    let (code, table, _) = cli(&machine, &["ps"]);
    assert_eq!(code, 0);
    assert!(table.lines().nth(1).expect("a row").starts_with("run-one  ok"), "{table}");
    let (_, json, _) = cli(&machine, &["ps", "--json", "--workflow", "w", "--state", "ok", "--limit", "3", "--all"]);
    assert!(parse(&json).is_ok() && json.contains("\"runs\""), "{json}");
    let (_, text, _) = cli(&machine, &["status", "run-one"]);
    assert!(text.contains("state: ok\n"), "{text}");
    let (_, json, _) = cli(&machine, &["status", "--json", "run-one"]);
    assert!(json.contains("\"state\": \"ok\""), "{json}");
    let (_, lines, _) = cli(&machine, &["logs", "run-one", "--types", "run.finished"]);
    assert_eq!(lines.lines().count(), 1);
    let (_, text, _) = cli(&machine, &["cancel", "run-one"]);
    assert_eq!(text, "run-one: ok (the run had already finished)\n");
    let (_, text, _) = cli(&machine, &["gc", "--dry-run", "--days", "5"]);
    assert_eq!(text, "would remove 0 run directories and 0 rotated index files; 0 left alone\n");
    let (_, text, _) = cli(&machine, &["gc"]);
    assert!(text.starts_with("removed 0 run directories"), "{text}");
}

#[test]
fn a_status_command_with_the_wrong_arguments_says_what_it_takes() {
    let machine = Machine::new(DEFAULTS);
    for (args, usage) in [
        (&["ps", "extra"][..], "ps takes [--workflow VALUE] [--state VALUE] [--limit VALUE] [--all] [--json]"),
        (&["ps", "--nope"], "ps takes"),
        (&["ps", "--limit"], "ps takes"),
        (&["status"], "status takes RUN [--json]"),
        (&["status", "a", "b"], "status takes RUN"),
        (&["cancel"], "cancel takes RUN [--json]"),
        (&["logs", "--types"], "logs takes RUN [--types VALUE] [--json]"),
        (&["gc", "--all"], "gc takes [--days VALUE] [--dry-run] [--json]"),
    ] {
        let (code, stdout, stderr) = cli(&machine, args);
        assert_eq!(code, 1, "{args:?}");
        assert!(stdout.contains("invalid-input") && stderr.contains(usage), "{args:?}: {stderr}");
    }
    // A name no run answers to is the command's own error.
    let (code, stdout, _) = cli(&machine, &["status", "nope"]);
    assert_eq!(code, 1);
    assert!(stdout.contains("unknown-run"), "{stdout}");
    let (code, stdout, _) = cli(&machine, &["gc", "--days", "soon"]);
    assert_eq!(code, 1);
    assert!(stdout.contains("invalid-input"), "{stdout}");
    // The first word of an unknown command is named, with the new commands in the list.
    let (_, _, stderr) = cli(&machine, &["nope"]);
    assert!(stderr.contains("ps, status, logs, tail, cancel, gc"), "{stderr}");
}
