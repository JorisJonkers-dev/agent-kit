//! The packet readers' small parts over generated input, compared with what the node runner makes of the same input
//! (tests/gen-readers-corpus.mjs): plan files, decision docs, the decisions stage's texts, the architecture and
//! recovery rules, the tracked tree and the ranking note. `CRITIQUE_READERS_CORPUS` names a bigger corpus to replay.

use super::cli::runtime;
use super::scenarios::Sandbox;
use crate::architecture::{build_configs, is_layer_config};
use crate::decision_docs::{is_decision_record, named_paths};
use crate::decision_docs_reader::resolve_decision_docs;
use crate::decisions::{decision_summary, feature_statement};
use crate::error::CritiqueError;
use crate::git::Git;
use crate::plan_file::{draft_of, named_plan_path, plan_file_path_or_null};
use crate::ranking::{rank_index, ranked_paths, ranking_note, RankedFile, RepoRanking};
use crate::recovery::{cited_issues, is_source_path, issue_reference, render_history_log, source_lines, source_sample, HistoryEntry};
use crate::repository_packet::{default_packet_limits, TrackedFile};
use crate::tracked_tree::read_tracked_tree;
use crate::Runtime;
use model_gateway::json::{self, Value};
use model_gateway::ports::GitRemote;

fn canonical(value: &Value) -> Value {
    match value {
        Value::Arr(items) => Value::Arr(items.iter().map(canonical).collect()),
        Value::Obj(object) => {
            let mut entries = object.entries();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let mut sorted = json::Object::new();
            for (key, item) in entries {
                sorted.set(key, canonical(item));
            }
            Value::Obj(sorted)
        }
        other => other.clone(),
    }
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object().and_then(|object| object.get(key))
}

fn text(value: &Value, key: &str) -> String {
    field(value, key).and_then(Value::as_str).unwrap_or_else(|| panic!("{key} is not a string")).to_string()
}

fn list(value: &Value, key: &str) -> Vec<Value> {
    match field(value, key) {
        Some(Value::Arr(items)) => items.clone(),
        _ => panic!("{key} is not a list"),
    }
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    list(value, key).iter().map(|item| item.as_str().expect("a string").to_string()).collect()
}

fn texts(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

fn optional(found: Option<String>) -> Value {
    found.map_or(Value::Null, |found| Value::Str(found))
}

fn number(value: &Value, key: &str) -> f64 {
    match field(value, key) {
        Some(Value::Num(number)) => *number,
        _ => panic!("{key} is not a number"),
    }
}

/// A result as the corpus writes it: `{ok}` or `{error: {code, message}}`.
fn outcome(result: Result<Value, CritiqueError>) -> Value {
    match result {
        Ok(ok) => json::object(vec![("ok", ok)]),
        Err(error) => json::object(vec![("error", json::object(vec![("code", Value::str(&error.code)), ("message", Value::str(&error.message))]))]),
    }
}

struct TreeGit {
    files: Vec<(String, String)>,
}

impl Git for TreeGit {
    fn remotes(&self, _cwd: &str) -> Result<Vec<GitRemote>, String> {
        Err("unused".to_string())
    }
    fn common_dir(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn root(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
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
    fn file_at(&self, _cwd: &str, _commit: &str, path: &str) -> Result<String, String> {
        self.files.iter().find(|(name, _)| name == path).map(|(_, text)| text.clone()).ok_or_else(|| "unused".to_string())
    }
    fn tree_files(&self, _cwd: &str, _commit: &str) -> Result<Vec<String>, String> {
        Ok(self.files.iter().map(|(name, _)| name.clone()).collect())
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
}

fn tracked_files(value: &Value) -> Vec<TrackedFile> {
    list(value, "tracked").iter().map(|file| TrackedFile { path: text(file, "path"), text: field(file, "text").and_then(Value::as_str).map(str::to_string) }).collect()
}

fn history_entries(value: &Value) -> Vec<HistoryEntry> {
    list(value, "entries")
        .iter()
        .map(|entry| HistoryEntry { commit: text(entry, "commit"), date: text(entry, "date"), subject: text(entry, "subject"), body: text(entry, "body"), files: strings(entry, "files") })
        .collect()
}

fn replay_plan(input: &Value) -> Value {
    let repository = field(input, "repository").expect("repository");
    let (root, common) = (text(repository, "root"), text(repository, "commonDir"));
    let flow = field(input, "flow").and_then(Value::as_str);
    json::object(vec![
        ("path", optional(plan_file_path_or_null(&root, &common, &text(input, "tracker"), &text(input, "draft")))),
        ("draft", optional(draft_of(flow).map(str::to_string))),
    ])
}

fn replay_history(input: &Value) -> Value {
    let entries = history_entries(input);
    let (rendered, left_out) = render_history_log(&entries, &text(input, "commit"));
    let cited = cited_issues(&entries).into_iter().map(Value::Num).collect();
    json::object(vec![("text", Value::Str(rendered)), ("left_out", Value::count(left_out)), ("cited", Value::Arr(cited))])
}

fn replay_configs(input: &Value) -> Value {
    let files = strings(input, "files");
    let tracked = tracked_files(input);
    json::object(vec![
        ("configs", texts(&build_configs(&files))),
        ("layers", Value::Arr(files.iter().map(|name| Value::Bool(is_layer_config(name))).collect())),
        ("sample", texts(&source_sample(&tracked))),
        ("lines", Value::count(source_lines(&tracked))),
        ("source", Value::Arr(tracked.iter().map(|file| Value::Bool(is_source_path(&file.path))).collect())),
    ])
}

fn replay_docs(input: &Value) -> Value {
    let sandbox = Sandbox::new();
    let runtime: Runtime = runtime("");
    let Some(Value::Obj(files)) = field(input, "files") else { panic!("files") };
    for (name, body) in files.entries() {
        let path = format!("{}{}", sandbox.text(), name.strip_prefix("/r").expect("under /r"));
        runtime.files.write_text(&path, body.as_str().expect("text")).expect("write");
    }
    let found = resolve_decision_docs(&runtime, &sandbox.text()).map(|docs| texts(&docs)).map_err(|mut error| {
        error.message = error.message.replace(&sandbox.text(), "/r");
        error
    });
    outcome(found)
}

fn replay_tree(input: &Value) -> Value {
    let mut limits = default_packet_limits();
    let exclude = field(field(input, "limits").expect("limits"), "exclude").expect("exclude");
    limits.exclude = ["generated", "lock", "vendored"].iter().map(|kind| (kind.to_string(), strings(exclude, kind))).collect();
    let files = list(input, "tree").iter().map(|file| (text(file, "path"), text(file, "text"))).collect();
    let mut runtime = runtime("");
    runtime.git = Box::new(TreeGit { files });
    outcome(read_tracked_tree(&runtime, "/r", "c0ffee", &limits).map(|tree| {
        let tracked = tree.tracked.iter().map(|file| json::object(vec![("path", Value::str(&file.path)), ("text", optional(file.text.clone()))]));
        let excluded = json::object(vec![
            ("generated", Value::count(tree.excluded.generated)),
            ("lock", Value::count(tree.excluded.lock)),
            ("vendored", Value::count(tree.excluded.vendored)),
        ]);
        json::object(vec![("excluded", excluded), ("tracked", Value::Arr(tracked.collect()))])
    }))
}

fn replay_ranking(input: &Value) -> Value {
    let ranking = field(input, "ranking").expect("ranking");
    let ranking = if text(ranking, "status") == "ranked" {
        RepoRanking::Ranked { files: list(ranking, "files").iter().map(|file| RankedFile { path: text(file, "path"), refs: number(file, "refs") }).collect(), symbols: Vec::new() }
    } else {
        RepoRanking::Unavailable { detail: text(ranking, "detail") }
    };
    let ranked = ranked_paths(Some(&ranking));
    let rank = rank_index(&ranked);
    let places = ["a.ts", "b.ts", "c.ts", "d.ts"].iter().map(|path| rank(path).map_or(Value::Null, Value::count));
    json::object(vec![("note", Value::Str(ranking_note(&ranking))), ("ranked", texts(&ranked)), ("places", Value::Arr(places.collect()))])
}

fn replay(case: &Value) -> Value {
    let input = field(case, "input").expect("input");
    let kind = text(case, "kind");
    match kind.as_str() {
        "paths" => {
            let record = strings(input, "record").iter().map(|path| Value::Bool(is_decision_record(path))).collect();
            json::object(vec![("paths", texts(&named_paths(&text(input, "text")))), ("record", Value::Arr(record))])
        }
        "plan" => replay_plan(input),
        "named" => {
            let repository = field(input, "repository").expect("repository");
            outcome(named_plan_path(&text(repository, "root"), &text(repository, "commonDir"), &text(input, "path")).map(|path| Value::Str(path)))
        }
        "history" => replay_history(input),
        "configs" => replay_configs(input),
        "decisions" => {
            let source = text(input, "text");
            json::object(vec![
                ("feature", outcome(feature_statement(&source).map(Value::Str))),
                ("summary", outcome(decision_summary(&source).map(Value::Str))),
            ])
        }
        "docs" => replay_docs(input),
        "tree" => replay_tree(input),
        _ => replay_ranking(input),
    }
}

#[test]
fn readers_corpus() {
    let source = match std::env::var("CRITIQUE_READERS_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/readers.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 300);
    for (index, case) in cases.iter().enumerate() {
        let expected = field(case, "expected").expect("expected");
        let actual = replay(case);
        assert_eq!(json::compact(&canonical(&actual)), json::compact(&canonical(expected)), "case {index} differs\n  case: {}", json::compact(case));
    }
}

#[test]
fn a_cited_issue_is_named_the_way_javascript_prints_its_number() {
    assert_eq!(issue_reference(12.0), "#12");
    assert_eq!(issue_reference(1e21), "#1e+21");
}

#[test]
fn a_draft_that_names_no_folder_has_no_plan_file() {
    use crate::plan_file::{plan_file_path, PLAN_FILE};
    assert_eq!(plan_file_path("/r", "/r/.git", "local-markdown", "a/b").unwrap(), format!("/r/.scratch/a/b/{PLAN_FILE}"));
    assert_eq!(plan_file_path("/r", "/r/.git", "github", "a/b").unwrap(), format!("/r/.git/critique/plans/a~b/{PLAN_FILE}"));
    let error = plan_file_path("/r", "/r/.git", "github", "a/../b").unwrap_err();
    assert_eq!(error.code, "invalid-input");
    assert_eq!(error.message, "the draft name a/../b has an empty or dot segment, so it names no folder");
}

#[test]
fn a_tree_git_cannot_read_is_an_unexpected_failure() {
    let mut broken = runtime("");
    broken.git = Box::new(TreeGit { files: Vec::new() });
    let ok = read_tracked_tree(&broken, "/r", "c", &default_packet_limits()).expect("an empty tree");
    assert!(ok.tracked.is_empty());
    let mut refused = runtime("");
    refused.git = Box::new(TreeGit { files: vec![("a.ts".to_string(), "x".to_string())] });
    let mut limits = default_packet_limits();
    limits.exclude = vec![("lock".to_string(), vec!["{".to_string()])];
    assert_eq!(read_tracked_tree(&refused, "/r", "c", &limits).unwrap_err().code, "unexpected");
    // The default runtime's git fails on a tree it cannot read.
    assert_eq!(read_tracked_tree(&runtime(""), "/r", "c", &default_packet_limits()).unwrap_err().code, "unexpected");
}

#[test]
fn a_window_that_holds_ranges_is_frozen_with_them() {
    use crate::packet_files::frozen_window;
    use crate::tree_sample::{LineRange, SampledWindow};
    let prefix = frozen_window(&SampledWindow { last: 40, lines: 90, ranges: None });
    assert_eq!((prefix.last, prefix.lines, prefix.ranges), (40.0, 90.0, None));
    let sliced = frozen_window(&SampledWindow { last: 20, lines: 90, ranges: Some(vec![LineRange { first: 3, last: 5 }, LineRange { first: 9, last: 20 }]) });
    let held: Vec<(f64, f64)> = sliced.ranges.expect("ranges").iter().map(|range| (range.first, range.last)).collect();
    assert_eq!(held, vec![(3.0, 5.0), (9.0, 20.0)]);
}
