//! The estate skills' mechanical helpers, one binary with four subcommands
//! (#813, parent #802). Each replaces a Python script of the skill that called it.
//!
//! * `estate audit REPO_DIR [--latest-sha SHA] [--json]` is the offline audit of
//!   one estate repository (estate-bootstrap).
//! * `estate verdict [--signals FILE]` applies the delivery verdict to the
//!   signals SKILL.md gathers (estate-delivery-status).
//! * `estate intent scaffold ...` and `estate intent validate PATH [EXTRA...]`
//!   write and validate a deploy-kit Project Intent (estate-project-intent).
//! * `estate plan --project P --action pause|rollback|resume ...` prints the
//!   exact `gh workflow run` command for a pause, rollback or resume
//!   (estate-rollback). It never dispatches a workflow.
//!
//! Exit codes are each subcommand's own and documented in docs/RUST.md; 2 is
//! always bad usage. Output goes to the [Ctx] writers so the tests read it.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::time::Duration;

pub mod args;
pub mod audit;
pub mod intent;
pub mod plan;
pub mod proc;
pub mod text;
pub mod verdict;

#[cfg(test)]
#[path = "testkit/tests.rs"]
mod testkit;

/// Everything a subcommand touches outside its arguments.
pub struct Ctx<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    pub input: &'a mut dyn Read,
    /// The PATH the external tools (`gh`, `deploy-kit`) are looked up on.
    pub path: String,
    pub home: Option<String>,
    /// Overrides the subcommand's own time limit on an external tool.
    pub timeout: Option<Duration>,
}

impl Ctx<'_> {
    /// One line on stdout.
    pub fn say(&mut self, line: &str) {
        let _ = writeln!(self.out, "{line}");
    }

    /// One line on stderr.
    pub fn warn(&mut self, line: &str) {
        let _ = writeln!(self.err, "{line}");
    }
}

const USAGE: &str = "usage: estate audit REPO_DIR [--latest-sha SHA] [--json]
       estate verdict [--signals FILE]
       estate intent scaffold --project NAME --owner OWNER --host HOST --audience AUDIENCE --image IMAGE --process PROCESS --port N --out DIR
       estate intent validate PATH [EXTRA_PATH ...]
       estate plan --project P --action pause|rollback|resume [--target V] [--reason R] [--pin V] [--proof-names V | --no-proof]";

/// Runs one subcommand and returns the process exit status.
pub fn run(args: &[String], ctx: &mut Ctx) -> i32 {
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first().map(String::as_str) {
        Some("audit") => audit::run(rest, ctx),
        Some("verdict") => verdict::run(rest, ctx),
        Some("intent") => intent::run(rest, ctx),
        Some("plan") => plan::run(rest, ctx),
        Some("--help" | "-h") => {
            ctx.say(USAGE);
            0
        }
        _ => {
            ctx.warn(USAGE);
            2
        }
    }
}

/// [run] against the real process environment.
pub fn run_env(args: impl Iterator<Item = OsString>) -> i32 {
    let args: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    let mut ctx = Ctx {
        out: &mut std::io::stdout(),
        err: &mut std::io::stderr(),
        input: &mut std::io::stdin(),
        path: std::env::var("PATH").unwrap_or_default(),
        home: std::env::var("HOME").ok(),
        timeout: None,
    };
    run(&args, &mut ctx)
}

#[cfg(test)]
mod tests;
