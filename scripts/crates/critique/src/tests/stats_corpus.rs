//! `stats` over generated ledgers, thresholds and filters, compared with what the node runner's command answered.

use crate::cli::COMMANDS;
use crate::dirs::SystemDirs;
use crate::git::Git;
use crate::Runtime;
use model_gateway::json::{self, Value};
use model_gateway::ports::{Clock, GitRemote};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Frozen;

impl Clock for Frozen {
    fn now_ms(&self) -> i64 {
        1_793_707_200_000
    }

    fn monotonic_ms(&self) -> f64 {
        0.0
    }
}

/// A repository whose remotes are the case's.
struct Scripted(Vec<GitRemote>);

impl Git for Scripted {
    fn remotes(&self, _cwd: &str) -> Result<Vec<GitRemote>, String> {
        Ok(self.0.clone())
    }

    fn common_dir(&self, _cwd: &str) -> Result<String, String> {
        Ok("/repo/.git".to_string())
    }

    fn root(&self, _cwd: &str) -> Result<String, String> {
        Ok("/repo".to_string())
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

    fn porcelain_status(&self, _cwd: &str) -> Result<Vec<crate::git::PorcelainEntry>, String> {
        Err("unused".to_string())
    }

    fn tree_files(&self, _cwd: &str, _commit: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn untracked_paths(&self, _cwd: &str) -> Result<Vec<String>, String> {
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
}

fn temp_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("critique-stats-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, text).expect("write");
}

/// The keys of every object sorted, as the corpus keeps them.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Arr(items) => Value::Arr(items.iter().map(canonical).collect()),
        Value::Obj(object) => {
            let mut entries = object.entries();
            entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            let mut sorted = json::Object::new();
            for (key, item) in entries {
                sorted.set(key, canonical(item));
            }
            Value::Obj(sorted)
        }
        other => other.clone(),
    }
}

fn replay(index: usize, case: &Value) {
    let case = case.as_object().expect("case object");
    let home = temp_dir();
    if let Some(own) = case.get("own").and_then(Value::as_str) {
        put(&home.join(".local/share/agent-kit/critique/ledger.jsonl"), own);
    }
    if let Some(gateway) = case.get("gateway").and_then(Value::as_str) {
        put(&home.join("state/agent-kit/model-gateway/ledger.jsonl"), gateway);
    }
    if let Some(over) = case.get("override").and_then(Value::as_str) {
        put(&home.join(".config/agent-kit/critique-routing.json"), over);
    }
    let Some(Value::Arr(pairs)) = case.get("remotes") else { panic!("case has no remotes") };
    let remotes = pairs
        .iter()
        .map(|pair| match pair {
            Value::Arr(parts) => GitRemote { name: parts[0].as_str().unwrap_or_default().to_string(), url: parts[1].as_str().unwrap_or_default().to_string() },
            _ => panic!("remote is not a pair"),
        })
        .collect();
    let mut env = HashMap::new();
    env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
    env.insert("XDG_STATE_HOME".to_string(), home.join("state").to_string_lossy().into_owned());
    let mut gateway = model_gateway::default_runtime();
    gateway.env = env.clone();
    gateway.clock = Box::new(Frozen);
    let runtime = Runtime {
        gateway,
        clock: Box::new(Frozen),
        files: Box::new(model_gateway::adapters::fs::SystemFiles { cwd: "/".to_string() }),
        dirs: Box::new(SystemDirs),
        git: Box::new(Scripted(remotes)),
        env,
        assets_root: "/assets".to_string(),
        gh: Box::new(crate::gh::SystemGh { env: HashMap::new() }),
        stdin: Box::new(String::new),
    };
    let run = COMMANDS.iter().find(|(name, _)| *name == "stats").and_then(|(_, command)| *command).expect("stats is ported");
    let actual = match run(case.get("input").expect("input"), &runtime) {
        Ok(result) => json::object(vec![("ok", result)]),
        Err(error) => json::object(vec![("error", json::object(vec![("code", Value::str(&error.code)), ("message", Value::str(&error.message))]))]),
    };
    let actual_text = json::compact(&canonical(&actual)).replace(&home.to_string_lossy().into_owned(), "/h");
    let expected_text = json::compact(&canonical(case.get("expected").expect("expected")));
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(actual_text, expected_text, "case {index} differs\n  case: {}", json::compact(&Value::Obj(case.clone())));
}

/// Replays the corpus; `CRITIQUE_STATS_CORPUS` names a bigger one.
#[test]
fn stats_corpus() {
    let text = match std::env::var("CRITIQUE_STATS_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/stats-read.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&text) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 100);
    for (index, case) in cases.iter().enumerate() {
        replay(index, case);
    }
}
