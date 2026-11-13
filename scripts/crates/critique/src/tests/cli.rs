//! The command line's own paths: what an unported command and a hook answer, how a result exits, and standard input.

use crate::cli::{exit_code, run_cli, COMMANDS, USAGE};
use crate::git::Git;
use crate::Runtime;
use model_gateway::json::parse;
use model_gateway::ports::{Clock, GitRemote};
use std::collections::HashMap;

struct NoGit;

impl Git for NoGit {
    fn remotes(&self, _cwd: &str) -> Result<Vec<GitRemote>, String> {
        Err("no git here".to_string())
    }

    fn common_dir(&self, _cwd: &str) -> Result<String, String> {
        Err("no git here".to_string())
    }

    fn root(&self, _cwd: &str) -> Result<String, String> {
        Err("no git here".to_string())
    }

    fn resolve_commit(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn last_change(&self, _cwd: &str, _path: &str) -> Result<Option<crate::git::LastChange>, String> {
        Err("unused".to_string())
    }

    fn file_commits(&self, _cwd: &str, _revision: &str, _path: &str) -> Result<Vec<crate::git::LastChange>, String> {
        Err("unused".to_string())
    }

    fn file_at(&self, _cwd: &str, _commit: &str, _path: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn tree_files(&self, _cwd: &str, _commit: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn untracked_paths(&self, _cwd: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn merge_base(&self, _cwd: &str, _a: &str, _b: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn diff_commits(&self, _cwd: &str, _base: &str, _head: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn changed_paths(&self, _cwd: &str, _base: &str, _head: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn staged_entries(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn tree_of(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn write_index_tree(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn commit_log(&self, _cwd: &str, _revision: &str, _paths: &[String], _limit: usize) -> Result<Vec<crate::recovery::HistoryEntry>, String> {
        Err("unused".to_string())
    }

    fn commit_tree(&self, _cwd: &str, _tree: &str, _parent: &str, _message: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn commits_between(&self, _cwd: &str, _base: &str, _head: &str) -> Result<Vec<crate::git::CommitLink>, String> {
        Err("unused".to_string())
    }

    fn tree_entries(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn merge_tree(&self, _cwd: &str, _ours: &str, _theirs: &str) -> Result<crate::git::MergeTree, String> {
        Err("unused".to_string())
    }

    fn diff_against_head(&self, _cwd: &str, _staged: bool) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn paths_against_head(&self, _cwd: &str, _staged: bool) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn current_branch(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn porcelain_status(&self, _cwd: &str) -> Result<Vec<crate::git::PorcelainEntry>, String> {
        Err("unused".to_string())
    }
}

struct Frozen;

impl Clock for Frozen {
    fn now_ms(&self) -> i64 {
        0
    }

    fn monotonic_ms(&self) -> f64 {
        0.0
    }
}

pub fn runtime(stdin: &str) -> Runtime {
    let stdin = stdin.to_string();
    Runtime {
        gateway: model_gateway::default_runtime(),
        clock: Box::new(Frozen),
        files: Box::new(model_gateway::adapters::fs::SystemFiles { cwd: "/".to_string() }),
        dirs: Box::new(crate::dirs::SystemDirs),
        git: Box::new(NoGit),
        gh: Box::new(crate::gh::SystemGh { env: HashMap::new() }),
        env: HashMap::new(),
        assets_root: "/assets".to_string(),
        stdin: Box::new(move || stdin.clone()),
    }
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

#[test]
fn a_command_not_ported_yet_says_so() {
    for (name, _) in COMMANDS.iter().filter(|(_, command)| command.is_none()) {
        for flags in [vec![*name], vec![*name, "--json", "{}"], vec![*name, "--json", "{}", "--strict"]] {
            let result = run_cli(&argv(&flags), &runtime(""));
            assert_eq!((result.exit_code, result.stderr.as_str()), (1, format!("critique {name}: critique {name} is not ported to the Rust runner yet; the node runner answers it\n").as_str()));
            assert_eq!(result.stdout, format!("{{\"code\":\"not-ported\",\"message\":\"critique {name} is not ported to the Rust runner yet; the node runner answers it\",\"status\":\"error\"}}\n"));
        }
    }
}

#[test]
fn a_hook_it_cannot_name_prints_the_usage_and_never_exits_2() {
    for bad in [&["hook"][..], &["hook", "bogus"], &["hook", "critique-pr-gate", "x"], &["hook", "constructor"]] {
        let result = run_cli(&argv(bad), &runtime(""));
        assert_eq!((result.exit_code, result.stdout.as_str(), result.stderr.as_str()), (1, "", USAGE), "{bad:?}");
    }
}

#[test]
fn a_command_line_it_cannot_read_prints_the_usage_and_exits_2() {
    for bad in [&[][..], &["bogus"], &["allow", "bogus"], &["allow", "--json"], &["allow", "--json", "{}", "x"], &["--strict"], &["allow", "--strict", "--json", "{}"]] {
        let result = run_cli(&argv(bad), &runtime(""));
        assert_eq!((result.exit_code, result.stdout.as_str(), result.stderr.as_str()), (2, "", USAGE), "{bad:?}");
    }
    let help = run_cli(&argv(&["--help"]), &runtime(""));
    assert_eq!((help.exit_code, help.stdout.as_str(), help.stderr.as_str()), (0, USAGE, ""));
    assert_eq!(run_cli(&argv(&["-h", "--strict"]), &runtime("")).exit_code, 0);
}

#[test]
fn a_failed_check_exits_3_and_a_refusal_exits_4_only_under_strict() {
    let failed = parse("{\"status\":\"failed\"}").expect("json");
    let refused = parse("{\"status\":\"refused\"}").expect("json");
    let allowed = parse("{\"status\":\"allowed\"}").expect("json");
    let bare = parse("[]").expect("json");
    assert_eq!((exit_code(&failed, false), exit_code(&failed, true)), (3, 3));
    assert_eq!((exit_code(&refused, false), exit_code(&refused, true)), (0, 4));
    assert_eq!((exit_code(&allowed, true), exit_code(&bare, true)), (0, 0));
}

#[test]
fn without_a_flag_the_input_is_read_from_standard_input() {
    let result = run_cli(&argv(&["allow"]), &runtime("{\"families\":5}"));
    assert_eq!((result.exit_code, result.stderr.as_str()), (1, "critique allow: allow input: families must be a list\n"));
    let result = run_cli(&argv(&["allow"]), &runtime(""));
    assert!(result.stderr.contains("the input is not JSON: SyntaxError: Unexpected end of JSON input"), "{}", result.stderr);
}
