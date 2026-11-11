//! The launcher: starts one agent id with its reach level's tool
//! configuration, and refuses, before anything starts, a level its agent
//! cannot hold on this OS (spec #392, stories 44-46, 51, 52).
//!
//! The support record is a YAML file (`registry/reach-levels.yaml`): for each
//! agent the command that starts it and, for each reach level, its tool
//! arguments and a per-OS cell, `verified`, `unverified` or `unsupported`;
//! and the agent ids, each naming one agent at one reach level (and an
//! optional skill set), the OSes it runs on and, for `research` and
//! `write-scoped`, its output directory. Only a verified cell may be used.
//!
//! Usage:
//! * `agent-launcher validate <RECORD>`: exit 0 clean, 1 rule problems on
//!   stderr, 2 bad usage or an unreadable or malformed file;
//! * `agent-launcher run <RECORD> <ID> [-- ARGS...]`: starts the id's agent
//!   with its tool arguments then ARGS, with `AGENT_KIT_REACH` (and
//!   `AGENT_KIT_OUTPUT_DIR` when the id has one) in its environment, and
//!   exits with its status. A refused id exits 1 and starts nothing.
//!
//! The operating-system write limit (ADR-0013) is a separate interface that
//! plugs in here; this crate holds the reach levels and the support record.

mod record;

use std::ffi::{OsStr, OsString};
use std::fs;
use std::process::Command;

pub use record::{cell, parse, problems, Record, LEVELS, OSES};

/// The crate's name, which prefixes every message line.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The record's name for the OS this runs on.
pub fn this_os() -> &'static str {
    os_name(std::env::consts::OS)
}

/// The record's name for a Rust OS name: `mac` for macOS, `linux` otherwise.
pub fn os_name(rust_os: &str) -> &'static str {
    if rust_os == "macos" {
        "mac"
    } else {
        "linux"
    }
}

/// Why an id may not start on `os`, or the command line it starts with.
pub fn plan(record: &Record, id: &str, os: &str) -> Result<Plan, String> {
    let entry = record
        .ids
        .get(id)
        .ok_or_else(|| format!("unknown agent id {id:?}"))?;
    let (Some(agent), Some(level)) = (entry.agent.as_deref(), entry.reach.as_deref()) else {
        return Err(format!("{id}: has no agent or no reach level"));
    };
    let spec = record
        .agents
        .get(agent)
        .ok_or_else(|| format!("{id}: unknown agent {agent:?}"))?;
    let state = cell(record, agent, level, os);
    if state != "verified" {
        return Err(format!(
            "{id}: {agent} cannot hold reach level {level} on {os} ({state}); nothing started"
        ));
    }
    if !entry.hosts.iter().any(|h| h == os) {
        return Err(format!("{id}: not declared for {os}; nothing started"));
    }
    if spec.command.is_empty() {
        return Err(format!("{id}: agent {agent} has no command"));
    }
    if matches!(level, "research" | "write-scoped") && entry.output_dir.is_none() {
        return Err(format!("{id}: reach level {level} has no output_dir"));
    }
    let mut argv = spec.command.clone();
    argv.extend(spec.levels[level].args.iter().cloned());
    Ok(Plan {
        argv,
        level: level.to_string(),
        output_dir: entry.output_dir.clone(),
    })
}

/// What a permitted id starts.
#[derive(Debug, PartialEq)]
pub struct Plan {
    pub argv: Vec<String>,
    pub level: String,
    pub output_dir: Option<String>,
}

fn usage() -> String {
    format!("usage: {NAME} validate <RECORD> | {NAME} run <RECORD> <ID> [-- ARGS...]")
}

/// Runs one command line and returns its exit code.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    run_on(args, this_os())
}

/// [`run`] for a named OS, so tests can stand in for either.
pub fn run_on<I>(args: I, os: &str) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    match args.as_slice() {
        [cmd, file] if cmd == OsStr::new("validate") => validate_file(file),
        [cmd, file, id, rest @ ..] if cmd == OsStr::new("run") => {
            let extra = match rest {
                [] => &[][..],
                [dash, extra @ ..] if dash == OsStr::new("--") => extra,
                _ => return bad_usage(),
            };
            launch(file, &id.to_string_lossy(), extra, os)
        }
        _ => bad_usage(),
    }
}

fn bad_usage() -> i32 {
    eprintln!("{}", usage());
    2
}

fn load(file: &OsStr) -> Result<Record, i32> {
    let text = fs::read_to_string(file).map_err(|error| {
        eprintln!("{NAME}: cannot read {}: {error}", file.to_string_lossy());
        2
    })?;
    parse(&text).map_err(|error| {
        eprintln!("{NAME}: {error}");
        2
    })
}

fn validate_file(file: &OsStr) -> i32 {
    let record = match load(file) {
        Ok(record) => record,
        Err(code) => return code,
    };
    let found = problems(&record);
    if found.is_empty() {
        println!("ok ({} ids)", record.ids.len());
        0
    } else {
        for problem in &found {
            eprintln!("{NAME}: {problem}");
        }
        1
    }
}

fn launch(file: &OsStr, id: &str, extra: &[OsString], os: &str) -> i32 {
    let record = match load(file) {
        Ok(record) => record,
        Err(code) => return code,
    };
    let plan = match plan(&record, id, os) {
        Ok(plan) => plan,
        Err(refusal) => {
            eprintln!("{NAME}: {refusal}");
            return 1;
        }
    };
    let mut command = Command::new(&plan.argv[0]);
    command
        .args(&plan.argv[1..])
        .args(extra)
        .env("AGENT_KIT_REACH", &plan.level);
    if let Some(dir) = &plan.output_dir {
        command.env("AGENT_KIT_OUTPUT_DIR", dir);
    }
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("{NAME}: cannot start {}: {error}", plan.argv[0]);
            1
        }
    }
}

#[cfg(test)]
mod tests;
