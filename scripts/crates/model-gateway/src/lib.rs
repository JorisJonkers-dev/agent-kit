//! model-gateway: one JSON object in, one JSON object out, ported from the node
//! runner command by command. It carries the command line itself, `models`,
//! `usage`, `grant write`, `grant reserve`, `grant list`, `worktrees clean` and the three
//! workflow commands; `run` and `batch` answer `not-ported`.
//!
//! Real files, the clock and the environment sit behind the traits in `ports`;
//! the command line takes them from a `Runtime`, so tests replace any of them.

pub mod adapters;
pub mod allowlist;
pub mod caps;
pub mod client_run;
pub mod cli;
pub mod commands;
pub mod config;
pub mod config_parse;
pub mod error;
pub mod extras;
pub mod git_remote;
pub mod grants;
pub mod grants_file;
pub mod home_mcp;
pub mod json;
pub mod kept_file;
pub mod launchers;
pub mod ledger;
pub mod ledger_file;
pub mod lock;
pub mod locations;
pub mod models;
pub mod outputs;
pub mod policy;
pub mod ports;
pub mod reserve;
pub mod run;
pub mod sha256;
pub mod sources;
pub mod state_file;
pub mod status;
pub mod text;
pub mod timestamps;
pub mod toml;
pub mod usage;
pub mod vibe_sign_in;
pub mod workflows;
pub mod worktrees;

use ports::Runtime;
use std::collections::HashMap;
use std::io::Write;

/// The shipped `defaults.json`, the one the node runner reads beside itself.
pub const SHIPPED_DEFAULTS: &str = include_str!("../../../../skills/workstation/model-gateway/defaults.json");

/// The real runtime: the system's clock and files, the environment and the working directory.
pub fn default_runtime() -> Runtime {
    let cwd = std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
    let env = std::env::vars_os()
        .map(|(key, value)| (key.to_string_lossy().into_owned(), value.to_string_lossy().into_owned()))
        .collect::<HashMap<_, _>>();
    Runtime {
        clock: Box::new(adapters::clock::SystemClock),
        files: Box::new(adapters::fs::SystemFiles { cwd }),
        lock: ports::LockTimings::default(),
        process: Box::new(adapters::process::SystemProcess { env: env.clone() }),
        git: Box::new(adapters::git::SystemGit { env: env.clone(), timeout_ms: adapters::git::TIMEOUT_MS }),
        env,
        defaults: SHIPPED_DEFAULTS.to_string(),
        defaults_path: "defaults.json".to_string(),
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
