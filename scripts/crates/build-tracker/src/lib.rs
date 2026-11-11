//! build-tracker: the state file of a spec's tickets and the page rendered
//! from it, with the command line (`init`, `sync`, `path`, `refresh`,
//! `render`, `set`) the /flow skill drives.
//!
//! Real files, clock, gh and git sit behind the traits in `ports`; the
//! command line takes them from a `Runtime`, so tests replace any of them.

pub mod adapters;
pub mod assignments;
pub mod board;
pub mod cli;
pub mod commands;
pub mod json;
pub mod local_tickets;
pub mod page;
pub mod ports;
pub mod seed;
pub mod spec;
pub mod specs;
pub mod state;
pub mod state_file;
pub mod text;

use adapters::clock::SystemClock;
use adapters::fs::SystemFiles;
use adapters::gh::GhCliIssues;
use adapters::git::GitRepositories;
use adapters::process::SystemProcess;
use ports::Runtime;
use std::collections::HashMap;
use std::io::Write;

/// The real runtime: the system's files, clock, `gh` and `git`, the working directory and the environment.
pub fn default_runtime() -> Runtime {
    let cwd = std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
    Runtime {
        clock: Box::new(SystemClock),
        files: Box::new(SystemFiles { cwd: cwd.clone() }),
        cwd,
        env: std::env::vars_os().map(|(key, value)| (key.to_string_lossy().into_owned(), value.to_string_lossy().into_owned())).collect::<HashMap<_, _>>(),
        issues: Box::new(GhCliIssues { process: SystemProcess { env: Vec::new() } }),
        repositories: Box::new(GitRepositories { process: SystemProcess { env: Vec::new() } }),
    }
}

/// Runs the command line in `runtime` and writes its output; the exit code.
pub fn run_with(args: impl Iterator<Item = String>, runtime: &Runtime, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let argv: Vec<String> = args.collect();
    let result = cli::run_cli(&argv, runtime);
    let _ = stderr.write_all(result.stderr.as_bytes());
    let _ = stdout.write_all(result.stdout.as_bytes());
    result.exit_code
}

#[cfg(test)]
mod tests;
