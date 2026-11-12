//! `worktrees clean` over a git that records what it is asked: what the node
//! runner's own goldens cannot reach, a kept file whose entries are not whole.

use super::scenarios::{FixedClock, Sandbox, FIXED_NOW};
use crate::adapters::fs::SystemFiles;
use crate::cli::run_cli;
use crate::json::{parse, Value};
use crate::ports::{FileStamp, Files, FsError, Git, GitRemote, LockTimings, Runtime};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A git that answers for no repository, fails the paths and branches it is told to, and notes every call.
struct RecordingGit {
    calls: Rc<RefCell<Vec<String>>>,
}

impl Git for RecordingGit {
    fn root(&self, _path: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn remotes(&self, _path: &str) -> Result<Vec<GitRemote>, String> {
        Ok(Vec::new())
    }

    fn remove_worktree(&self, repo: &str, path: &str) -> Result<(), String> {
        self.calls.borrow_mut().push(format!("remove {repo} {path}"));
        if path.contains("stuck") {
            return Err("fatal: stuck".to_string());
        }
        Ok(())
    }

    fn delete_branch(&self, repo: &str, branch: &str) -> Result<(), String> {
        self.calls.borrow_mut().push(format!("branch {repo} {branch}"));
        if branch.contains("keep") {
            return Err("error: branch is checked out".to_string());
        }
        Ok(())
    }
}

fn runtime(sandbox: &Sandbox, calls: &Rc<RefCell<Vec<String>>>) -> Runtime {
    let root = sandbox.text();
    std::fs::create_dir_all(format!("{root}/state/agent-kit/model-gateway")).expect("state");
    let env = HashMap::from([("HOME".to_string(), format!("{root}/home")), ("XDG_STATE_HOME".to_string(), format!("{root}/state"))]);
    Runtime {
        clock: Box::new(FixedClock(FIXED_NOW)),
        files: Box::new(SystemFiles { cwd: root }),
        lock: LockTimings::default(),
        process: Box::new(crate::adapters::process::SystemProcess { env: env.clone() }),
        git: Box::new(RecordingGit { calls: Rc::clone(calls) }),
        env,
        defaults: crate::SHIPPED_DEFAULTS.to_string(),
        defaults_path: "defaults.json".to_string(),
    }
}

fn clean(runtime: &Runtime, input: &str) -> Value {
    let argv: Vec<String> = ["worktrees", "clean", "--json", input].iter().map(|word| word.to_string()).collect();
    let result = run_cli(&argv, runtime);
    parse(result.stdout.trim()).expect("one JSON object")
}

fn kept_file(sandbox: &Sandbox) -> String {
    std::fs::read_to_string(format!("{}/state/agent-kit/model-gateway/kept-worktrees.json", sandbox.text())).expect("the kept file")
}

fn write_kept(sandbox: &Sandbox, text: &str) {
    std::fs::create_dir_all(format!("{}/state/agent-kit/model-gateway", sandbox.text())).expect("state");
    std::fs::write(format!("{}/state/agent-kit/model-gateway/kept-worktrees.json", sandbox.text()), text).expect("kept file");
}

#[test]
fn an_entry_without_a_path_fails_and_stays_kept_and_one_without_a_repository_is_handed_to_git_as_javascript_spells_it() {
    let sandbox = Sandbox::new();
    let calls = Rc::new(RefCell::new(Vec::new()));
    write_kept(
        &sandbox,
        r#"{"version":1,"worktrees":[null,{"run_id":"r1","workflow":"w","path":"/p","branch":["x",null,2]},{"run_id":"r2","workflow":"w","repo":"/r"},{"run_id":"r3","workflow":"w","repo":false,"path":"/ok","branch":{"a":1}},{"run_id":"r4","repo":null,"path":"/q","branch":true}]}"#,
    );

    let printed = clean(&runtime(&sandbox, &calls), r#"{"all":true}"#);

    let expected = parse(
        r#"{"branch_kept":[],"failed":[
            {"reason":"the kept worktree names no path"},
            {"reason":"the kept worktree names no path","run_id":"r2"}],
          "removed":[
            {"branch":["x",null,2],"run_id":"r1","workflow":"w","worktree":"/p"},
            {"branch":{"a":1},"run_id":"r3","workflow":"w","worktree":"/ok"},
            {"branch":true,"run_id":"r4","worktree":"/q"}],"status":"ok"}"#,
    )
    .expect("expected");
    assert_eq!(printed, expected);
    let asked: Vec<String> = ["remove undefined /p", "branch undefined x,,2", "remove false /ok", "branch false [object Object]", "remove null /q", "branch null true"].iter().map(|call| call.to_string()).collect();
    assert_eq!(*calls.borrow(), asked);
    assert!(kept_file(&sandbox).contains("\"run_id\": \"r2\"") && !kept_file(&sandbox).contains("\"run_id\": \"r3\""));
}

#[test]
fn a_named_workflow_leaves_entries_of_other_workflows_and_entries_without_one() {
    let sandbox = Sandbox::new();
    let calls = Rc::new(RefCell::new(Vec::new()));
    write_kept(
        &sandbox,
        r#"{"version":1,"worktrees":[{"run_id":"a","workflow":"w","repo":"/r","path":"/stuck","branch":"ba"},{"run_id":"b","workflow":"other","repo":"/r","path":"/b","branch":"bb"},{"run_id":"c","repo":"/r","path":"/c","branch":"bc"},{"run_id":"d","workflow":"w","repo":"/r","path":"/d","branch":"keep-d"}]}"#,
    );

    let printed = clean(&runtime(&sandbox, &calls), r#"{"workflow":"w"}"#);

    let expected = parse(
        r#"{"branch_kept":[{"branch":"keep-d","reason":"error: branch is checked out","run_id":"d"}],
          "failed":[{"reason":"fatal: stuck","run_id":"a","worktree":"/stuck"}],
          "removed":[{"branch":"keep-d","run_id":"d","workflow":"w","worktree":"/d"}],"status":"ok"}"#,
    )
    .expect("expected");
    assert_eq!(printed, expected);
    let kept = kept_file(&sandbox);
    assert!(kept.contains("\"run_id\": \"a\"") && kept.contains("\"run_id\": \"b\"") && kept.contains("\"run_id\": \"c\"") && !kept.contains("\"run_id\": \"d\""), "{kept}");
}

#[test]
fn an_entry_whose_run_id_is_not_text_is_removed_and_forgotten_like_any_other() {
    // The second entry has no run id and the first has the number 5; a number is spelled as JavaScript spells it.
    let sandbox = Sandbox::new();
    let calls = Rc::new(RefCell::new(Vec::new()));
    write_kept(&sandbox, r#"{"version":1,"worktrees":[{"run_id":5,"workflow":"w","repo":1.5,"path":"/p","branch":"b"},{"workflow":"w","repo":"/r","path":"/q"}]}"#);

    let printed = clean(&runtime(&sandbox, &calls), r#"{"all":true}"#);

    let removed = printed.as_object().and_then(|object| object.get("removed")).expect("removed");
    let Value::Arr(removed) = removed else { panic!("removed is a list") };
    assert_eq!(removed.len(), 2);
    assert_eq!(kept_file(&sandbox).matches("\"repo\"").count(), 0);
}

#[test]
fn a_kept_file_that_is_not_a_list_of_worktrees_is_refused() {
    let sandbox = Sandbox::new();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let runtime = runtime(&sandbox, &calls);
    for (text, message) in [
        ("not json", "kept-worktrees.json: not JSON: "),
        ("{}", "kept-worktrees.json: must be {\"version\": 1, \"worktrees\": [...]}"),
        ("[]", "kept-worktrees.json: must be {\"version\": 1, \"worktrees\": [...]}"),
        (r#"{"worktrees":{}}"#, "kept-worktrees.json: must be {\"version\": 1, \"worktrees\": [...]}"),
    ] {
        write_kept(&sandbox, text);
        let printed = clean(&runtime, r#"{"all":true}"#);
        let object = printed.as_object().expect("object");
        assert_eq!(object.get("code").and_then(Value::as_str), Some("invalid-config"), "{text}");
        assert!(object.get("message").and_then(Value::as_str).unwrap_or_default().contains(message), "{text}: {printed:?}");
    }
    assert!(calls.borrow().is_empty());
}

#[test]
fn a_kept_file_that_cannot_be_read_is_an_error() {
    let sandbox = Sandbox::new();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let runtime = runtime(&sandbox, &calls);
    // A directory where the file belongs reads as an error that is not "no such file".
    std::fs::create_dir_all(format!("{}/state/agent-kit/model-gateway/kept-worktrees.json", sandbox.text())).expect("directory");

    let printed = clean(&runtime, r#"{"all":true}"#);

    assert_eq!(printed.as_object().and_then(|object| object.get("code")).and_then(Value::as_str), Some("unexpected"));
}

/// The real files, except that a file can no longer be replaced: the disk is full.
struct FullDisk(SystemFiles);

impl Files for FullDisk {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        self.0.read_text(path)
    }

    fn write_atomic(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Err(FsError { code: "ENOSPC".into(), message: "ENOSPC: no space left on device, write".into() })
    }

    fn mkdir_all(&self, path: &str) -> Result<(), FsError> {
        self.0.mkdir_all(path)
    }

    fn create_exclusive(&self, path: &str) -> Result<bool, FsError> {
        self.0.create_exclusive(path)
    }

    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        self.0.remove_file(path)
    }

    fn modified_ms(&self, path: &str) -> Option<i64> {
        self.0.modified_ms(path)
    }

    fn exists(&self, path: &str) -> Result<bool, FsError> {
        self.0.exists(path)
    }

    fn append(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.append(path, content)
    }

    fn stamp(&self, path: &str) -> Option<FileStamp> {
        self.0.stamp(path)
    }

    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_private(path, content)
    }
}

#[test]
fn a_kept_file_that_cannot_be_written_is_an_error_after_the_worktrees_were_removed() {
    let sandbox = Sandbox::new();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = runtime(&sandbox, &calls);
    runtime.files = Box::new(FullDisk(SystemFiles { cwd: sandbox.text() }));
    write_kept(&sandbox, r#"{"version":1,"worktrees":[{"run_id":"a","workflow":"w","repo":"/r","path":"/p","branch":"b"}]}"#);

    let printed = clean(&runtime, r#"{"all":true}"#);

    let object = printed.as_object().expect("object");
    assert_eq!(object.get("code").and_then(Value::as_str), Some("unexpected"));
    assert_eq!(object.get("message").and_then(Value::as_str), Some("ENOSPC: no space left on device, write"));
    assert_eq!(*calls.borrow(), vec!["remove /r /p".to_string(), "branch /r b".to_string()]);
    assert!(kept_file(&sandbox).contains("\"run_id\":\"a\""), "the file is as it was");
}
