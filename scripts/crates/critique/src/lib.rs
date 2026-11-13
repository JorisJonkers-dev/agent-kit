//! The critique runner, ported from the node runner command by command: one JSON
//! object in, one JSON object out. The unit tests replay the golden scenarios the
//! node runner produced (tests/fixtures) byte for byte.

pub mod allow;
pub mod allowlist;
pub mod cli;
pub mod error;
pub mod fields;
pub mod git;
pub mod locations;
pub mod repository;
pub mod runtime;

pub use runtime::{default_runtime, Runtime};

use std::io::Write;

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
