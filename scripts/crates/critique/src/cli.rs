//! The runner's command line: one command, one JSON object in, one JSON object out.

use crate::{allow, budget_command, check_adrs, check_plan, check_rewrite, check_tickets, cold_read, facts, gate_command, prepare, record, reviews, session_start, state_command, stats_command, triage_command};
use crate::error::CritiqueError;
use crate::runtime::Runtime;
use model_gateway::json::{self, Object, Value};

/// What the node runner prints for `--help` and for a command line it cannot read.
pub const USAGE: &str = include_str!("usage.txt");

/// The exit code of a result whose status is `failed`: a check failed, though the command did not.
pub const FAILED_EXIT_CODE: i32 = 3;

/// The exit code of a refused result under `--strict`: a Gold Band gate node reads any non-zero exit as red.
pub const REFUSED_EXIT_CODE: i32 = 4;

/// What one run prints and how it exits.
pub struct CliResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub type Command = fn(&Value, &Runtime) -> Result<Value, CritiqueError>;

/// The commands, each with its port: `None` until it is ported, when the runner says so rather than guess.
pub const COMMANDS: [(&str, Option<Command>); 19] = [
    ("allow", Some(allow::run)),
    ("budget", Some(budget_command::run)),
    ("check-adrs", Some(check_adrs::run)),
    ("check-plan", Some(check_plan::run)),
    ("check-rewrite", Some(check_rewrite::run)),
    ("check-tickets", Some(check_tickets::run)),
    ("cold-read", Some(cold_read::run)),
    ("facts", Some(facts::run)),
    ("fan-out", None),
    ("gate", Some(gate_command::run)),
    ("prepare", Some(prepare::run)),
    ("record", Some(record::run)),
    ("refute", None),
    ("reviews", Some(reviews::run)),
    ("session-start", Some(session_start::run)),
    ("setup", None),
    ("state", Some(state_command::run)),
    ("stats", Some(stats_command::run)),
    ("triage", Some(triage_command::run)),
];


fn usage(exit_code: i32) -> CliResult {
    CliResult { exit_code, stdout: String::new(), stderr: USAGE.to_string() }
}

/// One error, as the contract prints it: one JSON object on stdout, and a line for the owner on stderr.
fn failure(name: &str, error: &CritiqueError) -> CliResult {
    let mut object = Object::new();
    object.set("code", Value::str(&error.code));
    object.set("message", Value::str(&error.message));
    object.set("status", Value::str("error"));
    CliResult {
        exit_code: 1,
        stdout: format!("{}\n", json::compact(&Value::Obj(object))),
        stderr: format!("critique {name}: {}\n", error.message),
    }
}

fn not_ported(what: &str) -> CritiqueError {
    CritiqueError::new("not-ported", format!("{what} is not ported to the Rust runner yet; the node runner answers it"))
}

/// The runner's command line.
pub fn run_cli(argv: &[String], runtime: &Runtime) -> CliResult {
    let strict = argv.len() > 1 && argv.last().map(String::as_str) == Some("--strict") && argv[0] != "hook";
    let args = if strict { &argv[..argv.len() - 1] } else { argv };
    let name = args.first().map(String::as_str);
    let flag = args.get(1).map(String::as_str);
    let value = args.get(2).map(String::as_str);
    if matches!(name, Some("--help" | "-h")) {
        return CliResult { exit_code: 0, stdout: USAGE.to_string(), stderr: String::new() };
    }
    if name == Some("hook") {
        // Exit 1, never 2: an agent treats exit 2 from a PreToolUse hook as a block.
        return match flag {
            Some("critique-session-start") if value.is_none() => session_start_hook(runtime),
            Some("critique-commit-gate") if value.is_none() => gate_hook("commit", runtime),
            Some("critique-pr-gate") if value.is_none() => gate_hook("pr", runtime),
            _ => usage(1),
        };
    }
    let command = COMMANDS.iter().find(|(known, _)| Some(*known) == name);
    let flag_ok = match flag {
        None => true,
        Some(flag) => (flag == "--input" || flag == "--json") && value.is_some(),
    };
    let (Some((name, command)), true, true) = (command, flag_ok, args.len() <= 3) else {
        return usage(2);
    };
    let Some(command) = command else {
        return failure(name, &not_ported(&format!("critique {name}")));
    };
    // The gate fails open: input that is not JSON is the gate's to log and allow, not an error.
    let lenient = matches!(*name, "gate" | "session-start");
    let parse = |text: String| if lenient { Ok(parse_leniently(text)) } else { parse_input(&text) };
    match read_input(flag, value, runtime).and_then(parse).and_then(|input| command(&input, runtime)) {
        Ok(result) => CliResult { exit_code: exit_code(&result, strict), stdout: format!("{}\n", json::pretty(&result)), stderr: String::new() },
        Err(error) => failure(name, &error),
    }
}

/// How a result exits: a failed check exits 3, and a refusal exits 4 under `--strict`; anything else is 0.
pub fn exit_code(result: &Value, strict: bool) -> i32 {
    match result.as_object().and_then(|object| object.get("status")).and_then(Value::as_str) {
        Some("failed") => FAILED_EXIT_CODE,
        Some("refused") if strict => REFUSED_EXIT_CODE,
        _ => 0,
    }
}

/// The text of the input: `--json`'s value, the file `--input` names, or standard input.
fn read_input(flag: Option<&str>, value: Option<&str>, runtime: &Runtime) -> Result<String, CritiqueError> {
    match (flag, value) {
        (Some("--json"), Some(text)) => Ok(text.to_string()),
        // Node opens an empty path and fails; resolving it against the working directory would find the directory.
        (Some("--input"), Some("")) => Err(CritiqueError::unexpected("ENOENT: no such file or directory, open ''")),
        (Some("--input"), Some(path)) => runtime.files.read_text(path).map_err(CritiqueError::from_fs),
        _ => Ok((runtime.stdin)()),
    }
}

/// The input as JSON, or as the text itself when it is not JSON.
fn parse_leniently(text: String) -> Value {
    json::parse(&text).unwrap_or(Value::Str(text))
}

/// A gate hook: the gate's `hook_output` when it blocks, or nothing; it always exits 0 (ADR-0002).
fn gate_hook(gate: &str, runtime: &Runtime) -> CliResult {
    CliResult { exit_code: 0, stdout: gate_command::hook_output(gate, parse_leniently((runtime.stdin)()), runtime), stderr: String::new() }
}

/// The session-start hook: the one line it decided on, or nothing; it always exits 0 (ADR-0002, #197).
fn session_start_hook(runtime: &Runtime) -> CliResult {
    let result = session_start::decide_in_time(&parse_leniently((runtime.stdin)()), runtime, std::time::Duration::from_millis(session_start::GATE_DEADLINE_MS));
    let line = result.as_object().and_then(|object| object.get("line")).and_then(Value::as_str).map(|line| format!("{line}\n"));
    CliResult { exit_code: 0, stdout: line.unwrap_or_default(), stderr: String::new() }
}

fn parse_input(text: &str) -> Result<Value, CritiqueError> {
    json::parse(text).map_err(|message| CritiqueError::new("invalid-input", format!("the input is not JSON: SyntaxError: {message}")))
}
