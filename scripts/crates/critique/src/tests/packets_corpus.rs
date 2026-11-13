//! The packet readers (ADR review, decisions, decision recovery, architecture review) and code-nav's map over generated
//! repositories, compared with what the node runner makes of the same repository (tests/gen-packets-corpus.mjs): its
//! working tree, git tree, history, issues and what code-nav answers. `CRITIQUE_PACKETS_CORPUS` names a bigger corpus.

use super::cli::runtime;
use super::scenarios::Sandbox;
use crate::adr_packet::{read_adr_packet, sample_tree, AdrRequest, DocumentPacket};
use crate::architecture_packet::read_architecture_packet;
use crate::code_nav;
use crate::decisions_packet::{read_decisions_packet, DecisionsRequest};
use crate::error::CritiqueError;
use crate::gh::{Gh, GhIssue, IssueLocator, IssueRequest, LinkedIssue};
use crate::git::Git;
use crate::packet_files::{Packet, PreparedRepository, RepositoryPacket, UnfrozenFile};
use crate::ranking::RepoRanking;
use crate::recovery::{source_sample, HistoryEntry};
use crate::recovery_packet::{read_recovery_packet, RecoveryRequest};
use crate::repository::Resolved;
use crate::repository_packet::{default_packet_limits, PacketLimits, TrackedFile};
use crate::tracker_resolver::{resolve_tracker, TrackerResolution};
use model_gateway::json::{self, number_text, Value};
use model_gateway::ports::{GitRemote, Process, ProcessCommand, ProcessResult};
use std::sync::{Arc, Mutex};

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

fn number(value: &Value, key: &str) -> f64 {
    match field(value, key) {
        Some(Value::Num(number)) => *number,
        _ => panic!("{key} is not a number"),
    }
}

fn texts(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

fn entries(value: &Value, key: &str) -> Vec<(String, Value)> {
    match field(value, key) {
        Some(Value::Obj(object)) => object.entries().into_iter().map(|(name, item)| (name.to_string(), item.clone())).collect(),
        _ => panic!("{key} is not an object"),
    }
}

/// The model repository as git sees it: its tree at the commit, and its history.
struct ModelGit {
    tree: Vec<(String, String)>,
    history: Vec<HistoryEntry>,
}

impl Git for ModelGit {
    fn remotes(&self, _cwd: &str) -> Result<Vec<GitRemote>, String> {
        Err("unused".to_string())
    }
    fn common_dir(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn root(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn resolve_commit(&self, _cwd: &str, revision: &str) -> Result<String, String> {
        if revision == "HEAD" || revision == "bad" {
            return Err("git rev-parse failed".to_string());
        }
        Ok(format!("{revision}-commit"))
    }
    fn last_change(&self, _cwd: &str, _path: &str) -> Result<Option<crate::git::LastChange>, String> {
        Ok(None)
    }
    fn file_commits(&self, _cwd: &str, _revision: &str, _path: &str) -> Result<Vec<crate::git::LastChange>, String> {
        Err("unused".to_string())
    }
    fn file_at(&self, _cwd: &str, _commit: &str, path: &str) -> Result<String, String> {
        self.tree.iter().find(|(name, _)| name == path).map(|(_, text)| text.clone()).ok_or_else(|| "unused".to_string())
    }
    fn tree_files(&self, _cwd: &str, _commit: &str) -> Result<Vec<String>, String> {
        Ok(self.tree.iter().map(|(name, _)| name.clone()).collect())
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
    fn commit_log(&self, _cwd: &str, revision: &str, paths: &[String], limit: usize) -> Result<Vec<HistoryEntry>, String> {
        let mut entries = self.history.clone();
        entries.push(HistoryEntry { commit: "args".to_string(), date: revision.to_string(), subject: format!("{}|{limit}", paths.join(",")), body: String::new(), files: Vec::new() });
        Ok(entries)
    }
    fn commit_tree(&self, _cwd: &str, _tree: &str, _parent: &str, _message: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn porcelain_status(&self, _cwd: &str) -> Result<Vec<crate::git::PorcelainEntry>, String> {
        Err("unused".to_string())
    }
}

/// The issues of the model repository, by number.
struct ModelGh {
    issues: Vec<(String, Value)>,
}

impl Gh for ModelGh {
    fn view_issue(&self, request: &IssueRequest) -> Result<GhIssue, String> {
        let number = number_text(request.locator.number);
        let Some((_, issue)) = self.issues.iter().find(|(name, _)| *name == number) else {
            return Err("gh issue view exited 1: not found".to_string());
        };
        Ok(GhIssue {
            number: request.locator.number,
            title: text(issue, "title"),
            body: text(issue, "body"),
            url: format!("https://github.com/o/r/issues/{number}"),
            labels: strings(issue, "labels"),
            comments: Vec::new(),
            state: None,
        })
    }
    fn sub_issues(&self, _locator: &IssueLocator) -> Result<Vec<GhIssue>, String> {
        Err("unused".to_string())
    }
    fn blocked_by(&self, _locator: &IssueLocator) -> Result<Vec<LinkedIssue>, String> {
        Err("unused".to_string())
    }
}

/// What code-nav answers, and the launches it was asked for.
struct ModelNav {
    answer: Value,
    calls: Arc<Mutex<Vec<ProcessCommand>>>,
    root: String,
}

impl Process for ModelNav {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }
    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Err("unused".to_string())
    }
    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.calls.lock().expect("calls").push(command.clone());
        let answer = &self.answer;
        let result = |stdout: String, stderr: String, exit_code: i32, timed_out: bool| Ok(ProcessResult { stdout, stderr, exit_code, timed_out });
        match text(answer, "outcome").as_str() {
            "throws" => Err(text(answer, "message")),
            "timeout" => result(String::new(), String::new(), 0, true),
            "exit" => result(String::new(), text(answer, "stderr"), number(answer, "code") as i32, false),
            _ => result(text(answer, "stdout").replace("/r/", &format!("{}/", self.root)).replace("/r (", &format!("{} (", self.root)), String::new(), 0, false),
        }
    }
}

fn limits(value: &Value) -> PacketLimits {
    let mut limits = default_packet_limits();
    limits.cap_bytes = number(value, "cap_bytes");
    limits.facts_cap_bytes = number(value, "facts_cap_bytes");
    limits.manifest_cap_bytes = number(value, "manifest_cap_bytes");
    limits.repo_map_cap_bytes = number(value, "repo_map_cap_bytes");
    limits.window_lines = number(value, "window_lines");
    let exclude = field(value, "exclude").expect("exclude");
    limits.exclude = ["generated", "lock", "vendored"].iter().map(|kind| (kind.to_string(), strings(exclude, kind))).collect();
    limits
}

fn file_value(file: &UnfrozenFile) -> Value {
    let mut pairs = vec![("kind", Value::str(file.kind)), ("path", Value::str(&file.path)), ("source", Value::str(&file.source)), ("text", Value::str(&file.text))];
    if let Some(window) = &file.window {
        let mut held = vec![("last", Value::Num(window.last)), ("lines", Value::Num(window.lines))];
        if let Some(ranges) = &window.ranges {
            let ranges = ranges.iter().map(|range| json::object(vec![("first", Value::Num(range.first)), ("last", Value::Num(range.last))]));
            held.push(("ranges", Value::Arr(ranges.collect())));
        }
        pairs.push(("window", json::object(held)));
    }
    json::object(pairs)
}

fn files_value(files: &[UnfrozenFile]) -> Value {
    Value::Arr(files.iter().map(file_value).collect())
}

fn repository_value(repository: &PreparedRepository) -> Value {
    json::object(vec![
        ("cap_bytes", Value::Num(repository.cap_bytes)),
        ("dropped_files", Value::count(repository.dropped_files)),
        ("head_commit", Value::str(&repository.head_commit)),
        ("packet_bytes", Value::count(repository.packet_bytes)),
        ("tree_files", Value::count(repository.tree_files)),
    ])
}

fn artifact_value(reference: &str, artifact_text: &str) -> Value {
    json::object(vec![("labels", Value::Arr(Vec::new())), ("reference", Value::str(reference)), ("text", Value::str(artifact_text)), ("tracker", Value::str("none"))])
}

fn repository_packet_value(packet: &RepositoryPacket) -> Value {
    let mut pairs = vec![
        ("artifact", artifact_value(&packet.reference, &packet.artifact_text)),
        ("files", files_value(&packet.files)),
        ("repository", repository_value(&packet.repository)),
        ("status", Value::str("read")),
    ];
    for (name, value) in [("adrs", packet.adrs), ("context_maps", packet.context_maps), ("source_lines", packet.source_lines)] {
        if let Some(value) = value {
            pairs.push((name, Value::count(value)));
        }
    }
    json::object(pairs)
}

fn ranking_value(ranking: &RepoRanking) -> Value {
    match ranking {
        RepoRanking::Unavailable { detail } => json::object(vec![("detail", Value::str(detail)), ("status", Value::str("unavailable"))]),
        RepoRanking::Ranked { files, symbols } => {
            let files = files.iter().map(|file| json::object(vec![("path", Value::str(&file.path)), ("refs", Value::Num(file.refs))]));
            let symbols = symbols.iter().map(|symbol| {
                json::object(vec![
                    ("kind", Value::str(&symbol.kind)),
                    ("line", Value::Num(symbol.line)),
                    ("name", Value::str(&symbol.name)),
                    ("path", Value::str(&symbol.path)),
                    ("refs", Value::Num(symbol.refs)),
                ])
            });
            json::object(vec![("files", Value::Arr(files.collect())), ("status", Value::str("ranked")), ("symbols", Value::Arr(symbols.collect()))])
        }
    }
}

fn packet_value<T>(packet: Result<Packet<T>, CritiqueError>, shown: impl Fn(&T) -> Value) -> Value {
    match packet {
        Ok(Packet::Read(read)) => json::object(vec![("ok", shown(&read))]),
        Ok(Packet::Refused(refusal)) => json::object(vec![("ok", refusal)]),
        Err(error) => json::object(vec![("error", json::object(vec![("code", Value::str(&error.code)), ("message", Value::str(&error.message))]))]),
    }
}

fn replay(case: &Value) -> Value {
    let sandbox = Sandbox::new();
    let root = sandbox.text();
    let model = field(case, "model").expect("model");
    let request = field(case, "request").expect("request");
    let worktree = field(model, "worktree").expect("worktree");
    let Value::Obj(files) = worktree else { panic!("worktree") };
    let mut rt = runtime("");
    for (name, body) in files.entries() {
        rt.files.write_text(&format!("{root}/{name}"), body.as_str().expect("text")).expect("write");
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    rt.git = Box::new(ModelGit {
        tree: list(model, "tree").iter().map(|file| (text(file, "path"), text(file, "text"))).collect(),
        history: list(model, "history")
            .iter()
            .map(|entry| HistoryEntry { commit: text(entry, "commit"), date: text(entry, "date"), subject: text(entry, "subject"), body: text(entry, "body"), files: strings(entry, "files") })
            .collect(),
    });
    rt.gh = Box::new(ModelGh { issues: entries(model, "issues") });
    rt.gateway.process = Box::new(ModelNav { answer: field(model, "nav").expect("nav").clone(), calls: Arc::clone(&calls), root: root.clone() });
    let head = field(request, "head").and_then(Value::as_str).map(str::to_string);
    let paths = |key: &str| field(request, key).map(|_| strings(request, key).iter().map(|path| path.replace("/r/", &format!("{root}/"))).collect::<Vec<_>>());
    let actual = match text(case, "kind").as_str() {
        "adr" => {
            let (adrs, references) = (paths("adrs"), paths("references"));
            let code = field(request, "code").map(|code| (text(code, "head"), limits(field(code, "limits").expect("limits"))));
            let request = AdrRequest { adrs: adrs.as_deref(), references: references.as_deref(), code: code.as_ref().map(|(head, limits)| (head.as_str(), limits)) };
            packet_value(read_adr_packet(&rt, &root, &request), |packet| {
                let mut pairs = vec![
                    ("adrs", Value::count(packet.adrs)),
                    ("artifact", artifact_value("adrs", &packet.artifact_text)),
                    ("files", files_value(&packet.files)),
                    ("status", Value::str("read")),
                ];
                if let Some(repository) = &packet.repository {
                    pairs.push(("repository", repository_value(repository)));
                }
                json::object(pairs)
            })
        }
        "decisions" => {
            let references = paths("references");
            let feature = field(request, "feature").and_then(Value::as_str);
            let summary = field(request, "summary").and_then(Value::as_str);
            let request = DecisionsRequest { feature, summary, references: references.as_deref() };
            packet_value(read_decisions_packet(&rt, &root, &request), |packet| {
                json::object(vec![
                    ("artifact", artifact_value(packet.reference, &packet.artifact_text)),
                    ("files", files_value(&packet.files)),
                    ("holdsAdr", Value::Bool(packet.holds_adr)),
                    ("status", Value::str("read")),
                ])
            })
        }
        "recovery" => {
            let repository = Resolved { root: root.clone(), common_dir: format!("{root}/.git"), key: "github.com/o/r".to_string() };
            let resolved = resolve_tracker(&rt, &repository).expect("a tracker");
            let tracker = match &resolved {
                TrackerResolution::Resolved(tracker) => Some(tracker),
                TrackerResolution::Refused(_) => None,
            };
            let history = field(request, "history").and_then(Value::as_object).map(|history| match history.get("tracker") {
                Some(Value::Bool(true)) => tracker,
                _ => None,
            });
            let limits = limits(field(request, "limits").expect("limits"));
            let request = RecoveryRequest { head: head.as_deref().expect("head"), limits: &limits, history };
            packet_value(read_recovery_packet(&rt, &root, &request), repository_packet_value)
        }
        "architecture" => {
            let limits = limits(field(request, "limits").expect("limits"));
            packet_value(read_architecture_packet(&rt, &root, head.as_deref().expect("head"), &limits), repository_packet_value)
        }
        "sample" => {
            let limits = limits(field(request, "limits").expect("limits"));
            let documents = [UnfrozenFile::new("reference", "references/CONTEXT.md", "CONTEXT.md", "See src/a.ts and index.ts.\n")];
            let packet = DocumentPacket { artifact_text: &documents[0].text, files: &documents };
            let rest = (field(request, "rest") == Some(&Value::Bool(true))).then_some(source_sample as fn(&[TrackedFile]) -> Vec<String>);
            let ranked = field(request, "ranked") == Some(&Value::Bool(true));
            packet_value(sample_tree(&rt, &root, &packet, head.as_deref().expect("head"), &limits, rest, ranked), |sampled| {
                json::object(vec![("files", files_value(&sampled.files)), ("repository", repository_value(&sampled.repository)), ("status", Value::str("sampled")), ("tracked", Value::Arr(sampled.tracked.iter().map(|file| json::object(vec![("path", Value::str(&file.path)), ("text", file.text.as_deref().map_or(Value::Null, Value::str))])).collect()))])
            })
        }
        _ => json::object(vec![("ok", ranking_value(&code_nav::rank(&rt, &root)))]),
    };
    let recorded: Vec<Value> = calls
        .lock()
        .expect("calls")
        .iter()
        .map(|call| {
            json::object(vec![
                ("args", texts(&call.args)),
                ("command", Value::str(&call.command)),
                ("cwd", Value::str(call.cwd.as_deref().unwrap_or_default())),
                ("stdin", Value::str(call.stdin.as_deref().unwrap_or_default())),
                ("timeoutMs", Value::Num(call.timeout_ms.unwrap_or_default() as f64)),
            ])
        })
        .collect();
    let shown = json::compact(&json::object(vec![("calls", Value::Arr(recorded)), ("result", actual)])).replace(&root, "/r");
    json::parse(&shown).expect("the replay's own JSON")
}

#[test]
fn packets_corpus() {
    let source = match std::env::var("CRITIQUE_PACKETS_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/packets.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 200);
    for (index, case) in cases.iter().enumerate() {
        let actual = replay(case);
        let expected = json::object(vec![("calls", Value::Arr(list(case, "calls"))), ("result", field(case, "expected").expect("expected").clone())]);
        // The corpus names a result `{ok}` or `{error}`; a refusal or a packet is the `ok` value.
        let wanted = json::compact(&canonical(&expected));
        assert_eq!(json::compact(&canonical(&actual)), wanted, "case {index} differs\n  case: {}", json::compact(case));
    }
}

#[test]
fn a_root_that_cannot_be_resolved_is_asked_about_as_named() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut rt = runtime("");
    let answer = json::parse(r#"{"outcome":"timeout"}"#).expect("json");
    rt.gateway.process = Box::new(ModelNav { answer, calls: Arc::clone(&calls), root: "/nowhere".to_string() });
    let ranking = code_nav::rank(&rt, "/nonexistent/critique-root");
    assert_eq!(ranking, RepoRanking::Unavailable { detail: "code-nav did not answer in time".to_string() });
    let recorded = calls.lock().expect("calls");
    assert_eq!(recorded[0].cwd.as_deref(), Some("/nonexistent/critique-root"));
    assert_eq!(recorded[0].args[1], "/nonexistent/critique-root");
}
