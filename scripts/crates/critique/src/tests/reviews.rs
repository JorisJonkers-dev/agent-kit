//! `reviews` over generated run directories and ledgers, compared with what the node runner's command answered.

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

/// A repository whose remotes and directories are the case's.
struct Scripted {
    remotes: Vec<GitRemote>,
    common: String,
}

impl Git for Scripted {
    fn remotes(&self, _cwd: &str) -> Result<Vec<GitRemote>, String> {
        Ok(self.remotes.clone())
    }

    fn common_dir(&self, _cwd: &str) -> Result<String, String> {
        Ok(self.common.clone())
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

    fn tree_files(&self, _cwd: &str, _commit: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn untracked_paths(&self, _cwd: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }
}

fn temp_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("critique-reviews-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, text).expect("write");
}

fn text<'a>(object: &'a json::Object, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

/// What the case's machine holds, under a real directory whose path is `/h` in the node runner's messages.
fn machine(case: &json::Object) -> (PathBuf, Runtime) {
    let home = temp_dir();
    if let Some(own) = text(case, "own") {
        put(&home.join(".local/share/agent-kit/critique/ledger.jsonl"), own);
    }
    if let Some(gateway) = text(case, "gateway") {
        put(&home.join("state/agent-kit/model-gateway/ledger.jsonl"), gateway);
    }
    let common = home.join("repo/.git");
    let critique = common.join("critique");
    std::fs::create_dir_all(&critique).expect("critique dir");
    let Some(Value::Arr(dirs)) = case.get("dirs") else { panic!("case has no dirs") };
    for dir in dirs {
        let dir = dir.as_object().expect("dir object");
        let path = critique.join(text(dir, "name").expect("name"));
        if text(dir, "kind") == Some("file") {
            put(&path, "a file\n");
            continue;
        }
        std::fs::create_dir_all(&path).expect("run dir");
        if let Some(manifest) = text(dir, "manifest") {
            put(&path.join("manifest.json"), manifest);
        }
        if dir.get("report") == Some(&Value::Bool(true)) {
            put(&path.join("report.md"), "# report\n");
        }
    }
    let remotes = match case.get("remotes") {
        Some(Value::Arr(pairs)) => pairs
            .iter()
            .map(|pair| match pair {
                Value::Arr(parts) => GitRemote { name: parts[0].as_str().unwrap_or_default().to_string(), url: parts[1].as_str().unwrap_or_default().to_string() },
                _ => panic!("remote is not a pair"),
            })
            .collect(),
        _ => panic!("case has no remotes"),
    };
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
        git: Box::new(Scripted { remotes, common: common.to_string_lossy().into_owned() }),
        env,
        assets_root: "/assets".to_string(),
        stdin: Box::new(String::new),
    };
    (home, runtime)
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
    let (home, runtime) = machine(case);
    let input = case.get("input").expect("input");
    let run = COMMANDS.iter().find(|(name, _)| *name == "reviews").and_then(|(_, command)| *command).expect("reviews is ported");
    let actual = match run(input, &runtime) {
        Ok(result) => json::object(vec![("ok", result)]),
        Err(error) => json::object(vec![("error", json::object(vec![("code", Value::str(&error.code)), ("message", Value::str(&error.message))]))]),
    };
    let actual_text = json::compact(&canonical(&actual)).replace(&home.to_string_lossy().into_owned(), "/h");
    let expected_text = json::compact(&canonical(case.get("expected").expect("expected")));
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(actual_text, expected_text, "case {index} differs\n  case: {}", json::compact(&Value::Obj(case.clone())));
}

/// Replays the corpus; `CRITIQUE_REVIEWS_CORPUS` names a bigger one.
#[test]
fn reviews_corpus() {
    let text = match std::env::var("CRITIQUE_REVIEWS_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/reviews-read.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&text) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 100);
    for (index, case) in cases.iter().enumerate() {
        replay(index, case);
    }
}

#[test]
fn the_stage_table_keys_a_stage_by_its_flow_and_judges_it_by_its_removal_rule() {
    use crate::stage::CRITIQUE_STAGES;
    use crate::stage_table::{flow_key, is_review_stage, removal_rule, FlowKey, RemovalRule};
    let reviews: Vec<&str> = CRITIQUE_STAGES.into_iter().filter(|stage| is_review_stage(stage)).collect();
    assert_eq!(reviews, ["adr-review", "decision-recovery", "architecture-review"]);
    for stage in CRITIQUE_STAGES {
        let review = reviews.contains(&stage);
        assert_eq!(removal_rule(stage), if review { RemovalRule::Runs } else { RemovalRule::Features }, "{stage}");
        let expected = match stage {
            _ if review => FlowKey::Run,
            "plan" | "decisions" => FlowKey::Draft,
            _ => FlowKey::Spec,
        };
        assert_eq!(flow_key(stage), expected, "{stage}");
    }
}
