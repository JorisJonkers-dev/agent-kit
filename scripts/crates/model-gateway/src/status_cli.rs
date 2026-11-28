//! The command line of the status commands: `ps`, `status`, `logs`, `cancel` and `gc`. Each takes flags and prints
//! a table or text for the owner, or with `--json` one JSON object.

use crate::cli::{failure, CliResult, Named};
use crate::commands;
use crate::json::{self, Object, Value};
use crate::ports::Runtime;

/// The flags each command takes: those with a value, then those without.
fn flags_of(name: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match name {
        "ps" => (&["workflow", "state", "limit"], &["all", "json"]),
        "status" | "cancel" => (&[], &["json"]),
        "logs" => (&["types"], &["json"]),
        _ => (&["days"], &["dry-run", "json"]),
    }
}

fn usage(name: &str) -> String {
    let (values, switches) = flags_of(name);
    let positional = if matches!(name, "status" | "logs" | "cancel") { " RUN" } else { "" };
    let flags: Vec<String> = values.iter().map(|flag| format!("[--{flag} VALUE]")).chain(switches.iter().map(|flag| format!("[--{flag}]"))).collect();
    format!("{name} takes{positional} {}", flags.join(" "))
}

/// Runs a status command, or `None` when `named` is not one.
pub(crate) fn run(named: &Named<'_>, runtime: &Runtime) -> Option<CliResult> {
    if !matches!(named.name, "ps" | "status" | "logs" | "cancel" | "gc") {
        return None;
    }
    let (values, switches) = flags_of(named.name);
    let mut input = Object::new();
    let mut json_output = false;
    let mut positional: Vec<&String> = Vec::new();
    let mut args = named.args.iter();
    while let Some(arg) = args.next() {
        let Some(flag) = arg.strip_prefix("--") else {
            positional.push(arg);
            continue;
        };
        if switches.contains(&flag) {
            match flag {
                "json" => json_output = true,
                "dry-run" => input.set("dry_run", Value::Bool(true)),
                _ => input.set(flag, Value::Bool(true)),
            }
        } else if values.contains(&flag) {
            let Some(value) = args.next() else {
                return Some(failure("invalid-input", &usage(named.name), Some(named.name), &[]));
            };
            // The numbers are numbers; anything else stays text, which the command refuses.
            let parsed = value.parse::<f64>().ok().filter(|_| matches!(flag, "days" | "limit")).map_or_else(|| Value::str(value), Value::Num);
            input.set(flag, parsed);
        } else {
            return Some(failure("invalid-input", &usage(named.name), Some(named.name), &[]));
        }
    }
    let wants_run = matches!(named.name, "status" | "logs" | "cancel");
    match (wants_run, positional.as_slice()) {
        (true, [run]) => input.set("run", Value::str(run)),
        (false, []) => {}
        _ => return Some(failure("invalid-input", &usage(named.name), Some(named.name), &[])),
    }
    Some(match commands::run(named.command, named.name, &input, runtime) {
        Ok(result) => {
            let stdout = if json_output {
                format!("{}\n", json::pretty(&result))
            } else {
                match named.name {
                    "ps" => crate::run_status::ps_table(&result),
                    "status" => crate::run_status::status_text(&result),
                    "logs" => crate::run_status::logs_text(&result),
                    "cancel" => crate::run_cancel::text(&result),
                    _ => crate::run_status::gc_text(&result),
                }
            };
            CliResult { exit_code: 0, stdout, stderr: String::new() }
        }
        Err(error) => failure(&error.code, &error.message, Some(named.name), &error.problems),
    })
}
