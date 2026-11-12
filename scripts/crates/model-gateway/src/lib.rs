//! model-gateway: one JSON object in, one JSON object out, ported from the node
//! runner command by command. This slice carries the command line itself and
//! the three workflow commands; the rest answer `not-ported`.
//!
//! Real files, the clock and the environment sit behind the traits in `ports`;
//! the command line takes them from a `Runtime`, so tests replace any of them.

pub mod adapters;
pub mod cli;
pub mod commands;
pub mod error;
pub mod json;
pub mod lock;
pub mod ports;
pub mod state_file;
pub mod text;
pub mod timestamps;
pub mod workflows;

use ports::Runtime;
use std::collections::HashMap;
use std::io::Write;

/// The real runtime: the system's clock and files, the environment and the working directory.
pub fn default_runtime() -> Runtime {
    let cwd = std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
    Runtime {
        clock: Box::new(adapters::clock::SystemClock),
        files: Box::new(adapters::fs::SystemFiles { cwd }),
        env: std::env::vars_os()
            .map(|(key, value)| (key.to_string_lossy().into_owned(), value.to_string_lossy().into_owned()))
            .collect::<HashMap<_, _>>(),
        lock: ports::LockTimings::default(),
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
