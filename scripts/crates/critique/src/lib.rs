//! The critique runner, ported from the node runner command by command: one JSON
//! object in, one JSON object out. The unit tests replay the golden scenarios the
//! node runner produced (tests/fixtures) byte for byte.

pub mod adjudication;
pub mod agy_client;
pub mod adr_drift;
pub mod adr_log;
pub mod adr_packet;
pub mod architecture;
pub mod architecture_packet;
pub mod assets;
pub mod agent_hooks;
pub mod allow;
pub mod allowlist;
pub mod budget;
pub mod budget_command;
pub mod check_adrs;
pub mod checks;
pub mod check_plan;
pub mod check_rewrite;
pub mod check_tickets;
pub mod code_change;
pub mod code_facts;
pub mod code_nav;
pub mod code_packet;
pub mod code_sample;
pub mod cold_read;
pub mod collate;
pub mod context_slices;
pub mod decisions_packet;
pub mod diff_filter;
pub mod evidence;
pub mod cli;
pub mod clients;
pub mod critic;
pub mod critic_run;
pub mod critic_tools;
pub mod decision_docs;
pub mod decode;
pub mod decision_docs_reader;
pub mod decisions;
pub mod depth;
pub mod diff_window;
pub mod dirs;
pub mod drop_summary;
pub mod error;
pub mod facts;
pub mod fan_out_command;
pub mod fan_out_result;
pub mod feature_gate;
pub mod family;
pub mod fields;
pub mod feature_review;
pub mod findings;
pub mod flow;
pub mod flow_budget;
pub mod gate;
pub mod gate_command;
pub mod gateway_ledger;
pub mod gh;
pub mod git;
pub mod import_scan;
pub mod js_regex;
pub mod limits;
pub mod ledger;
pub mod ledger_source;
pub mod locations;
pub mod manifest;
pub mod merge_commit;
pub mod merge;
pub mod merge_patch;
pub mod packet_files;
pub mod paths;
pub mod packet_search;
pub mod plan;
pub mod plan_file;
pub mod prompt;
pub mod plan_packet;
pub mod prepare;
pub mod prepared_json;
pub mod quota;
pub mod ranking;
pub mod recovery;
pub mod pricing;
pub mod record;
pub mod recovery_packet;
pub mod refactoring_plan;
pub mod refute;
pub mod repo_map;
pub mod refute_command;
pub mod refute_round;
pub mod report;
pub mod repository;
pub mod repository_packet;
pub mod repository_sample;
pub mod review_runs;
pub mod reviews;
pub mod roster;
pub mod roster_resolve;
pub mod routing;
pub mod routing_source;
pub mod run_ledger;
pub mod runtime;
pub mod shell_command;
pub mod search_client;
pub mod search_server;
pub mod source_scan;
pub mod stage;
pub mod stage_table;
pub mod staged_change;
pub mod stats;
pub mod session_start;
pub mod setup;
pub mod setup_checks;
pub mod state_command;
pub mod stats_command;
pub mod ticket_review;
pub mod tickets;
pub mod tracked_tree;
pub mod tripwire;
pub mod tracker;
pub mod tracker_resolver;
pub mod tree_sample;
pub mod work_client;
pub mod triage;
pub mod triage_command;
pub mod triage_index;
pub mod verdicts;
pub mod vibe_client;
pub mod vibe_search;

pub use runtime::{default_runtime, Runtime};

use std::io::Write;

/// Runs the command line in `runtime` and writes its output; the exit code.
pub fn run_with(args: impl Iterator<Item = String>, runtime: &Runtime, stdin: &mut dyn std::io::BufRead, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let argv: Vec<String> = args.collect();
    // The search server answers a line at a time while its client waits, so it reads standard input as a stream.
    if argv.first().map(String::as_str) == Some("search-server") {
        return search_server::serve(&argv[1..], stdin, stdout, stderr);
    }
    let result = cli::run_cli(&argv, runtime);
    let _ = stderr.write_all(result.stderr.as_bytes());
    let _ = stdout.write_all(result.stdout.as_bytes());
    result.exit_code
}

#[cfg(test)]
mod tests;
