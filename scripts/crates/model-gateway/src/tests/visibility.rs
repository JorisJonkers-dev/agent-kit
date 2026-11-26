//! Tests for a repository's visibility: what gh's answer reads as, that every
//! failure reads as unknown, and that a known answer is cached for its TTL.

use super::scenarios::{FixedClock, Sandbox};
use crate::adapters::fs::SystemFiles;
use crate::git_remote::RemoteLocation;
use crate::ports::{Git, GitRemote, LockTimings, Process, ProcessCommand, ProcessResult, Runtime};
use crate::policy::{resolve_scope, DataScope};
use crate::visibility::{parse_visibility, repository_visibility, visibility_cache_path, Visibility, CACHE_TTL_MS};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A gh that answers every call the same way and records what it was asked.
#[derive(Clone)]
struct Gh {
    answer: Result<ProcessResult, String>,
    calls: Arc<Mutex<Vec<Vec<String>>>>,
}

impl Gh {
    fn says(stdout: &str, exit_code: i32) -> Gh {
        Gh { answer: Ok(ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code, timed_out: false }), calls: Arc::default() }
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().expect("the calls").clone()
    }
}

impl Process for Gh {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }

    fn exec(&self, command: &str, args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.calls.lock().expect("the calls").push(std::iter::once(command).chain(args.iter().copied()).map(str::to_string).collect());
        self.answer.clone()
    }

    fn run(&self, _command: &ProcessCommand) -> Result<ProcessResult, String> {
        unreachable!("visibility starts no launch")
    }
}

/// A git that finds one repository at `/work/repo`, with the given `origin`.
struct OneRepository(Option<String>);

impl Git for OneRepository {
    fn root(&self, _path: &str) -> Result<Option<String>, String> {
        Ok(Some("/work/repo".to_string()))
    }
    fn remotes(&self, _path: &str) -> Result<Vec<GitRemote>, String> {
        Ok(self.0.iter().map(|url| GitRemote { name: "origin".to_string(), url: url.clone() }).collect())
    }
    fn remove_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        unreachable!()
    }
    fn delete_branch(&self, _repo: &str, _branch: &str) -> Result<(), String> {
        unreachable!()
    }
    fn common_dir(&self, _path: &str) -> Result<String, String> {
        unreachable!()
    }
    fn add_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        unreachable!()
    }
    fn git_dir(&self, _path: &str) -> Result<String, String> {
        unreachable!()
    }
    fn create_worktree(&self, _repo: &str, _path: &str, _branch: &str, _base: &str) -> Result<(), String> {
        unreachable!()
    }
    fn stage_all(&self, _path: &str) -> Result<(), String> {
        unreachable!()
    }
    fn staged_files(&self, _path: &str, _base: Option<&str>) -> Result<Vec<String>, String> {
        unreachable!()
    }
    fn list_worktrees(&self, _repo: &str) -> Result<Vec<String>, String> {
        unreachable!()
    }
    fn status(&self, _path: &str) -> Result<String, String> {
        unreachable!()
    }
}

fn machine(sandbox: &Sandbox, gh: &Gh, now: i64) -> Runtime {
    machine_with_origin(sandbox, gh, now, None)
}

fn machine_with_origin(sandbox: &Sandbox, gh: &Gh, now: i64, origin: Option<&str>) -> Runtime {
    let root = sandbox.text();
    Runtime {
        clock: Box::new(FixedClock(now)),
        files: Box::new(SystemFiles { cwd: root.clone() }),
        lock: LockTimings::default(),
        process: Box::new(gh.clone()),
        git: Box::new(OneRepository(origin.map(str::to_string))),
        env: HashMap::from([("HOME".to_string(), format!("{root}/home"))]),
        defaults: crate::SHIPPED_DEFAULTS.to_string(),
        defaults_path: "defaults.json".to_string(),
        routing: crate::SHIPPED_ROUTING.to_string(),
    }
}

fn github(owner: &str, repo: &str) -> RemoteLocation {
    RemoteLocation { host: "github.com".to_string(), owner: owner.to_string(), repo: repo.to_string() }
}

#[test]
fn only_the_three_words_github_uses_read_as_a_visibility() {
    assert_eq!(parse_visibility("public\n"), Visibility::Public);
    assert_eq!(parse_visibility("private\n"), Visibility::Private);
    assert_eq!(parse_visibility("internal\n"), Visibility::Private);
    for other in ["", "null", "Public", "public private", "{\"message\":\"Not Found\"}"] {
        assert_eq!(parse_visibility(other), Visibility::Unknown, "{other}");
    }
    assert!(Visibility::Public.is_public());
    assert!(!Visibility::Private.is_public() && !Visibility::Unknown.is_public());
}

#[test]
fn gh_is_asked_for_the_repository_and_a_known_answer_is_cached() {
    let sandbox = Sandbox::new();
    let gh = Gh::says("public\n", 0);
    let runtime = machine(&sandbox, &gh, 1_000);
    assert_eq!(repository_visibility(&runtime, &github("acme", "widgets")), Visibility::Public);
    assert_eq!(gh.calls(), vec![vec!["gh", "api", "repos/acme/widgets", "--jq", ".visibility"]]);
    // A second look within the TTL reads the cache, and gh is not asked again.
    assert_eq!(repository_visibility(&runtime, &github("acme", "widgets")), Visibility::Public);
    assert_eq!(gh.calls().len(), 1);
    let cache = std::fs::read_to_string(visibility_cache_path(&runtime.env).expect("a cache path")).expect("the cache");
    assert!(cache.contains("\"github.com/acme/widgets\"") && cache.contains("\"visibility\": \"public\""), "{cache}");
}

#[test]
fn a_cached_answer_past_its_ttl_is_asked_again() {
    let sandbox = Sandbox::new();
    let gh = Gh::says("private\n", 0);
    assert_eq!(repository_visibility(&machine(&sandbox, &gh, 1_000), &github("acme", "widgets")), Visibility::Private);
    assert_eq!(repository_visibility(&machine(&sandbox, &gh, 1_000 + CACHE_TTL_MS - 1), &github("acme", "widgets")), Visibility::Private);
    assert_eq!(gh.calls().len(), 1);
    assert_eq!(repository_visibility(&machine(&sandbox, &gh, 1_000 + CACHE_TTL_MS), &github("acme", "widgets")), Visibility::Private);
    assert_eq!(gh.calls().len(), 2);
}

#[test]
fn every_failure_reads_as_unknown_and_is_never_cached() {
    let sandbox = Sandbox::new();
    for gh in [
        Gh::says("{\"message\":\"Not Found\"}", 1),
        Gh::says("", 0),
        Gh { answer: Err("spawn gh ENOENT".to_string()), calls: Arc::default() },
        Gh { answer: Ok(ProcessResult { stdout: "public\n".to_string(), stderr: String::new(), exit_code: 0, timed_out: true }), calls: Arc::default() },
    ] {
        let runtime = machine(&sandbox, &gh, 1_000);
        assert_eq!(repository_visibility(&runtime, &github("acme", "widgets")), Visibility::Unknown);
        assert!(!std::path::Path::new(&visibility_cache_path(&runtime.env).expect("a cache path")).exists());
    }
}

#[test]
fn a_remote_not_on_github_com_is_unknown_without_asking() {
    let sandbox = Sandbox::new();
    let gh = Gh::says("public\n", 0);
    let runtime = machine(&sandbox, &gh, 1_000);
    for location in [
        RemoteLocation { host: "gitlab.com".to_string(), owner: "acme".to_string(), repo: "widgets".to_string() },
        RemoteLocation { host: "github-work".to_string(), owner: "acme".to_string(), repo: "widgets".to_string() },
        RemoteLocation { host: "github.com".to_string(), owner: "acme/sub".to_string(), repo: "widgets".to_string() },
    ] {
        assert_eq!(repository_visibility(&runtime, &location), Visibility::Unknown);
    }
    assert!(gh.calls().is_empty());
}

#[test]
fn a_cache_that_says_something_else_is_no_cache() {
    let sandbox = Sandbox::new();
    let gh = Gh::says("private\n", 0);
    let runtime = machine(&sandbox, &gh, 1_000);
    let path = visibility_cache_path(&runtime.env).expect("a cache path");
    std::fs::create_dir_all(std::path::Path::new(&path).parent().expect("a parent")).expect("dirs");
    // A cached word other than public or private, and a check from the future, are both asked again.
    std::fs::write(&path, r#"{"version":1,"repositories":{"github.com/acme/widgets":{"checked_at_ms":900,"visibility":"unknown"},"github.com/acme/later":{"checked_at_ms":5000,"visibility":"public"}}}"#).expect("cache");
    assert_eq!(repository_visibility(&runtime, &github("acme", "widgets")), Visibility::Private);
    assert_eq!(repository_visibility(&runtime, &github("acme", "later")), Visibility::Private);
    assert_eq!(gh.calls().len(), 2);
    std::fs::write(&path, "not json").expect("cache");
    assert_eq!(repository_visibility(&runtime, &github("acme", "widgets")), Visibility::Private);
    // A check time that is not a number, and a cache of another version, are no cache either.
    std::fs::write(&path, r#"{"version":1,"repositories":{"github.com/acme/widgets":{"checked_at_ms":"soon","visibility":"public"}}}"#).expect("cache");
    assert_eq!(repository_visibility(&runtime, &github("acme", "widgets")), Visibility::Private);
    std::fs::write(&path, r#"{"version":2,"repositories":{"github.com/acme/widgets":{"checked_at_ms":900,"visibility":"public"}}}"#).expect("cache");
    assert_eq!(repository_visibility(&runtime, &github("acme", "widgets")), Visibility::Private);
    assert_eq!(gh.calls().len(), 5);
}

#[test]
fn a_machine_with_no_cache_directory_asks_github_each_time() {
    let sandbox = Sandbox::new();
    let gh = Gh::says("public\n", 0);
    let mut runtime = machine(&sandbox, &gh, 1_000);
    runtime.env.clear();
    assert_eq!(repository_visibility(&runtime, &github("acme", "widgets")), Visibility::Public);
    assert_eq!(repository_visibility(&runtime, &github("acme", "widgets")), Visibility::Public);
    assert_eq!(gh.calls().len(), 2);
}

#[test]
fn the_cache_lives_under_xdg_cache_home_when_it_is_absolute() {
    let env = HashMap::from([("XDG_CACHE_HOME".to_string(), "/c".to_string()), ("HOME".to_string(), "/h".to_string())]);
    assert_eq!(visibility_cache_path(&env).as_deref(), Some("/c/model-gateway/repository-visibility.json"));
    let relative = HashMap::from([("XDG_CACHE_HOME".to_string(), "c".to_string()), ("HOME".to_string(), "/h".to_string())]);
    assert_eq!(visibility_cache_path(&relative).as_deref(), Some("/h/.cache/model-gateway/repository-visibility.json"));
    assert_eq!(visibility_cache_path(&HashMap::new()), None);
}

#[test]
fn a_repository_github_says_is_public_is_its_own_scope_whatever_the_allowlist_says() {
    let sandbox = Sandbox::new();
    let gh = Gh::says("public\n", 0);
    let runtime = machine_with_origin(&sandbox, &gh, 1_000, Some("git@github.com:Acme/Widgets.git"));
    assert_eq!(resolve_scope(&runtime, Some("/work/repo")), Ok(DataScope::Public { key: "github.com/acme/widgets".to_string() }));
}

#[test]
fn a_private_or_unknown_repository_is_held_to_its_entry() {
    for gh in [Gh::says("private\n", 0), Gh::says("internal\n", 0), Gh::says("", 1)] {
        let sandbox = Sandbox::new();
        let runtime = machine_with_origin(&sandbox, &gh, 1_000, Some("git@github.com:acme/widgets.git"));
        assert_eq!(resolve_scope(&runtime, Some("/work/repo")), Ok(DataScope::Repository { key: "github.com/acme/widgets".to_string(), entry: None }));
    }
}

#[test]
fn a_public_nedap_repository_is_still_nedap_and_github_is_not_asked() {
    let sandbox = Sandbox::new();
    let gh = Gh::says("public\n", 0);
    let runtime = machine_with_origin(&sandbox, &gh, 1_000, Some("https://github.com/nedap/open-thing.git"));
    assert_eq!(resolve_scope(&runtime, Some("/work/repo")), Ok(DataScope::Nedap));
    assert!(gh.calls().is_empty());
}

#[test]
fn a_repository_with_no_remote_asks_github_nothing() {
    let sandbox = Sandbox::new();
    let gh = Gh::says("public\n", 0);
    let runtime = machine_with_origin(&sandbox, &gh, 1_000, None);
    assert_eq!(resolve_scope(&runtime, Some("/work/repo")), Ok(DataScope::NoRemote));
    assert!(gh.calls().is_empty());
}
