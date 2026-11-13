//! The critique runner, ported from the node runner command by command: one JSON
//! object in, one JSON object out. The unit tests replay the golden scenarios the
//! node runner produced (tests/fixtures) byte for byte.

pub mod adr_drift;
pub mod adr_log;
pub mod assets;
pub mod allow;
pub mod allowlist;
pub mod budget;
pub mod check_adrs;
pub mod check_plan;
pub mod check_rewrite;
pub mod cold_read;
pub mod collate;
pub mod cli;
pub mod clients;
pub mod critic_tools;
pub mod decision_docs;
pub mod depth;
pub mod dirs;
pub mod error;
pub mod family;
pub mod fields;
pub mod flow_budget;
pub mod gate;
pub mod gateway_ledger;
pub mod git;
pub mod limits;
pub mod ledger;
pub mod ledger_source;
pub mod locations;
pub mod manifest;
pub mod merge_patch;
pub mod paths;
pub mod packet_search;
pub mod plan;
pub mod pricing;
pub mod refactoring_plan;
pub mod repository;
pub mod repository_packet;
pub mod review_runs;
pub mod reviews;
pub mod roster;
pub mod routing;
pub mod routing_source;
pub mod runtime;
pub mod stage;
pub mod stage_table;
pub mod tickets;
pub mod triage;
pub mod vibe_client;

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
