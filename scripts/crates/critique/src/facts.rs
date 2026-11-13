//! `facts`: the deterministic history and import-graph facts (ADR-0004). For the named files, the
//! last commit that changed each and how many commits did; for the named modules, what each imports
//! and how the named set depends on itself. Everything is read at HEAD's commit and its own
//! committer dates, so the same commit gives byte-identical facts. The facts hold no author and no
//! commit message; the command only prints them: it sends nothing to any critic and writes nothing.

use crate::code_facts::{code_facts, code_unit_order, history_facts, CodeFacts, FileHistory, HistoryFact, ModuleFact, ModuleSource};
use crate::error::{CritiqueError, ShapeError};
use crate::fields::Fields;
use crate::repository::{refusal, resolve, Resolution};
use crate::runtime::Runtime;
use model_gateway::json::{self, Value};
use std::collections::HashSet;

struct Input {
    files: Vec<String>,
    modules: Vec<String>,
    repo: String,
}

fn read_input(raw: &Value) -> Result<Input, ShapeError> {
    let fields = Fields::of(raw, "")?;
    let files = if fields.has("files") { fields.strings("files")? } else { Vec::new() };
    let modules = if fields.has("modules") { fields.strings("modules")? } else { Vec::new() };
    Ok(Input { files, modules, repo: fields.string("repo")? })
}

/// The distinct paths in JavaScript's default order (by UTF-16 code unit).
fn distinct_sorted(paths: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut distinct: Vec<String> = paths.iter().filter(|path| seen.insert(path.as_str())).cloned().collect();
    distinct.sort_by(|a, b| code_unit_order(a, b));
    distinct
}

fn optional(text: &Option<String>) -> Value {
    text.as_deref().map_or(Value::Null, Value::str)
}

pub fn history_value(fact: &HistoryFact) -> Value {
    json::object(vec![
        ("churn", Value::count(fact.churn)),
        ("last_commit", optional(&fact.last_commit)),
        ("last_date", optional(&fact.last_date)),
        ("path", Value::str(&fact.path)),
    ])
}

pub fn module_value(fact: &ModuleFact) -> Value {
    let imports = fact.imports.iter().map(|import| json::object(vec![("resolved", optional(&import.resolved)), ("specifier", Value::str(&import.specifier))]));
    json::object(vec![
        ("fan_in", Value::count(fact.fan_in)),
        ("fan_out", Value::count(fact.fan_out)),
        ("imports", Value::Arr(imports.collect())),
        ("instability", fact.instability.map_or(Value::Null, Value::Num)),
        ("language", fact.language.map_or(Value::Null, |language| Value::str(language.name()))),
        ("path", Value::str(&fact.path)),
    ])
}

fn strings(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

fn result(commit: &str, history: &[HistoryFact], facts: &CodeFacts) -> Value {
    // The node runner prints the keys in the order its object literal and spread give them.
    json::object(vec![
        ("commit", Value::str(commit)),
        ("history", Value::Arr(history.iter().map(history_value).collect())),
        ("status", Value::str("reported")),
        ("cycles", Value::Arr(facts.cycles.iter().map(|cycle| strings(cycle)).collect())),
        ("modules", Value::Arr(facts.modules.iter().map(module_value).collect())),
        ("notes", strings(&facts.notes)),
    ])
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let input = read_input(raw).map_err(|error| error.into_error("invalid-input", "facts input"))?;
    let root = match resolve(runtime, &input.repo)? {
        Resolution::Refused(refusal) => return Ok(refusal),
        Resolution::Resolved(repository) => repository.root,
    };
    let git = &runtime.git;
    let Ok(commit) = git.resolve_commit(&root, "HEAD") else {
        return Ok(refusal("no-commit", "the repository has no commit to read facts at"));
    };
    let mut histories = Vec::new();
    for path in distinct_sorted(&input.files) {
        let commits = git.file_commits(&root, &commit, &path).map_err(CritiqueError::unexpected)?;
        histories.push(FileHistory { commits, path });
    }
    let tree: HashSet<String> = git.tree_files(&root, &commit).map_err(CritiqueError::unexpected)?.into_iter().collect();
    let mut sources = Vec::new();
    for path in distinct_sorted(&input.modules) {
        let text = if tree.contains(&path) { Some(git.file_at(&root, &commit, &path).map_err(CritiqueError::unexpected)?) } else { None };
        sources.push(ModuleSource { path, text });
    }
    Ok(result(&commit, &history_facts(&histories), &code_facts(&sources, &tree)))
}
