//! `check-adrs`: the deterministic decision-log checker. It reads every record in
//! `docs/adr/` and reports what a script can judge. It sends nothing to any critic
//! and writes nothing, so it needs no family allowlist entry; a repository whose
//! remotes are refused is still refused.

use crate::adr_drift::{drift_and_stale, named_by, Change, CodeChecked, CodeFacts, Drift, Stale};
use crate::adr_log::{adr_check_status, check_adr_log, linked_paths, AdrSource, AdrStructure};
use crate::decision_docs::ADR_DIRECTORY;
use crate::error::CritiqueError;
use crate::fields::Fields;
use crate::repository::{refusal, resolve, Resolution};
use crate::runtime::Runtime;
use model_gateway::json::{self, Value};
use model_gateway::text::join;
use std::collections::{HashMap, HashSet};

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let repo = Fields::of(raw, "").and_then(|fields| fields.string("repo")).map_err(|error| error.into_error("invalid-input", "check-adrs input"))?;
    let root = match resolve(runtime, &repo)? {
        Resolution::Refused(refusal) => return Ok(refusal),
        Resolution::Resolved(repository) => repository.root,
    };
    let paths: Vec<String> = runtime
        .dirs
        .list_files(&join(&[&root, ADR_DIRECTORY]))
        .into_iter()
        .filter(|name| name.ends_with(".md"))
        .map(|name| format!("{ADR_DIRECTORY}/{name}"))
        .collect();
    if paths.is_empty() {
        return Ok(refusal("no-adrs", &format!("found no decision records in {ADR_DIRECTORY}/ to review")));
    }
    check_log(runtime, &root, &paths)
}

/// Reads the decision log's records and checks its structure, then what the tree and its history say about the code the records name.
fn check_log(runtime: &Runtime, root: &str, paths: &[String]) -> Result<Value, CritiqueError> {
    let mut sources = Vec::new();
    for path in paths {
        let text = runtime.files.read_text(&join(&[root, path])).map_err(CritiqueError::from_fs)?;
        sources.push(AdrSource { path: path.clone(), text });
    }
    let mut found = HashSet::new();
    for path in linked_paths(&sources) {
        if runtime.files.exists(&join(&[root, &path])).map_err(CritiqueError::from_fs)? {
            found.insert(path);
        }
    }
    let structure = check_adr_log(&sources, &found);
    let in_force: HashSet<&str> = structure.records.iter().filter(|record| record.in_force && record.number.is_some()).map(|record| record.path.as_str()).collect();
    let checked: Vec<(String, String)> = sources.iter().filter(|source| in_force.contains(source.path.as_str())).map(|source| (source.path.clone(), source.text.clone())).collect();
    let named = named_by(&checked);
    let (drifted, stale) = match code_facts(runtime, root, &named)? {
        Some(facts) => drift_and_stale(&named, &facts),
        None => (Vec::new(), Vec::new()),
    };
    Ok(report(&structure, &drifted, &stale))
}

/// A tracked file a symbol is searched in: not the decision log itself, and not vendored dependencies.
fn searched(path: &str) -> bool {
    !path.starts_with(&format!("{ADR_DIRECTORY}/")) && !path.split('/').any(|part| part == "node_modules")
}

/// Every word of the text, as `/[A-Za-z_]\w*/g` reads them: a word may start inside a run of digits.
fn words(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
            let start = at;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
            found.push(&text[start..at]);
        } else {
            at += 1;
        }
    }
    found
}

/// What the repository holds for the records that name code: the paths in the working tree, the symbols in the
/// tracked and untracked files outside the log, and the last change of each existing path. `None` when the
/// repository has no commit yet, since there is then no tree to judge a record against.
fn code_facts(runtime: &Runtime, root: &str, records: &[CodeChecked]) -> Result<Option<CodeFacts>, CritiqueError> {
    let Ok(head) = runtime.git.resolve_commit(root, "HEAD") else {
        return Ok(None);
    };
    let mut names: Vec<&str> = Vec::new();
    for name in records.iter().flat_map(|record| record.code.paths.iter()) {
        if !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    let mut existing = HashSet::new();
    for name in names {
        if runtime.files.exists(&join(&[root, name])).map_err(CritiqueError::from_fs)? {
            existing.insert(name.to_string());
        }
    }
    let mut changes = HashMap::new();
    for path in &existing {
        if let Some(change) = runtime.git.last_change(root, path).map_err(CritiqueError::unexpected)? {
            changes.insert(path.clone(), Change { commit: change.commit, date: change.date });
        }
    }
    let wanted: HashSet<&str> = records.iter().flat_map(|record| record.code.symbols.iter().map(String::as_str)).collect();
    let mut present = HashSet::new();
    let mut incomplete = false;
    if !wanted.is_empty() {
        let mut tracked = runtime.git.tree_files(root, &head).map_err(CritiqueError::unexpected)?;
        tracked.extend(runtime.git.untracked_paths(root).map_err(CritiqueError::unexpected)?);
        for path in tracked.iter().filter(|path| searched(path)) {
            match runtime.files.read_text(&join(&[root, path])) {
                Ok(text) => present.extend(words(&text).into_iter().filter(|word| wanted.contains(word)).map(str::to_string)),
                // A tracked file deleted from the working tree holds no symbol; any other failure leaves the search incomplete.
                Err(error) => incomplete |= error.code != "ENOENT",
            }
        }
    }
    Ok(Some(CodeFacts { existing, changes, present, incomplete }))
}

fn strings(items: impl Iterator<Item = String>) -> Value {
    Value::Arr(items.map(|item| Value::Str(item)).collect())
}

/// The report, its keys in the order the node runner builds them.
fn report(structure: &AdrStructure, drifted: &[Drift], stale: &[Stale]) -> Value {
    let records = structure.records.iter().map(|record| {
        json::object(vec![
            ("in_force", Value::Bool(record.in_force)),
            ("number", record.number.map_or(Value::Null, |number| Value::count(number as usize))),
            ("path", Value::str(&record.path)),
            ("status", record.status.as_deref().map_or(Value::Null, Value::str)),
            ("title", Value::str(&record.title)),
        ])
    });
    let numbering = &structure.numbering;
    let duplicates = numbering.duplicates.iter().map(|dup| json::object(vec![("number", Value::count(dup.number as usize)), ("records", strings(dup.records.iter().cloned()))]));
    let links = structure.unresolved_links.iter().map(|link| json::object(vec![("kind", Value::str(link.kind)), ("record", Value::str(&link.record)), ("target", Value::str(&link.target))]));
    json::object(vec![
        ("status", Value::str(adr_check_status(structure))),
        ("cites_superseded", Value::Arr(structure.cites_superseded.iter().map(|c| json::object(vec![("cites", Value::str(&c.cites)), ("record", Value::str(&c.record))])).collect())),
        (
            "numbering",
            json::object(vec![
                ("duplicates", Value::Arr(duplicates.collect())),
                ("gaps", Value::Arr(numbering.gaps.iter().map(|gap| Value::count(*gap as usize)).collect())),
                ("unnumbered", strings(numbering.unnumbered.iter().cloned())),
            ]),
        ),
        ("records", Value::Arr(records.collect())),
        ("superseded", strings(structure.superseded.iter().cloned())),
        ("unknown_statuses", Value::Arr(structure.unknown_statuses.iter().map(|u| json::object(vec![("record", Value::str(&u.record)), ("status", Value::str(&u.status))])).collect())),
        ("unresolved_links", Value::Arr(links.collect())),
        ("drifted", Value::Arr(drifted.iter().map(|d| json::object(vec![("named", Value::str(&d.named)), ("reason", Value::str(d.reason)), ("record", Value::str(&d.record))])).collect())),
        (
            "stale",
            Value::Arr(
                stale
                    .iter()
                    .map(|s| {
                        json::object(vec![
                            ("decision_date", Value::str(&s.decision_date)),
                            ("last_change", Value::str(&s.last_change)),
                            ("last_commit", Value::str(&s.last_commit)),
                            ("path", Value::str(&s.path)),
                            ("record", Value::str(&s.record)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}
