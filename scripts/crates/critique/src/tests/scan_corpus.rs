//! The replay of the corpus the node runner's scans produced over generated sources
//! (tests/gen-scan-corpus.mjs): the import scan, the source scan, the code facts and the word search.
//! The environment's `CRITIQUE_SCAN_CORPUS` names a bigger corpus file to replay instead of the committed one.

use crate::code_facts::{code_facts, history_facts, FileHistory, ModuleSource};
use crate::facts::{history_value, module_value};
use crate::git::LastChange;
use crate::import_scan::{language_of, resolve_import, scan_imports, ImportKind};
use crate::source_scan::{mention_lines, mentions, normalize, posix_dirname, posix_extname, posix_join, resolve_import as resolve_source_import, scan_source};
use model_gateway::json::{self, Object, Value};
use std::collections::HashSet;

fn canonical(value: &Value) -> Value {
    match value {
        Value::Obj(object) => {
            let mut pairs: Vec<(String, Value)> = object.entries().into_iter().map(|(key, value)| (key.to_string(), canonical(value))).collect();
            pairs.sort_by(|left, right| left.0.cmp(&right.0));
            let mut sorted = Object::new();
            for (key, value) in pairs {
                sorted.set(&key, value);
            }
            Value::Obj(sorted)
        }
        Value::Arr(items) => Value::Arr(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).unwrap_or_else(|| panic!("the case has no {key}"))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    field(value, key).as_str().unwrap_or_else(|| panic!("{key} is not a string"))
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match field(value, key) {
        Value::Arr(items) => items,
        _ => panic!("{key} is not a list"),
    }
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    list(value, key).iter().map(|item| item.as_str().expect("a string").to_string()).collect()
}

fn optional(text: Option<String>) -> Value {
    text.map_or(Value::Null, |text| Value::Str(text))
}

fn scan_expected(case: &Value) -> Value {
    let path = text(case, "path");
    let source = text(case, "text");
    let tree: HashSet<String> = strings(case, "tree").into_iter().collect();
    let language = language_of(path);
    let found = language.map_or_else(Vec::new, |language| scan_imports(language, source));
    let imports = found.iter().map(|import| {
        let kind = if import.kind == ImportKind::Module { "module" } else { "import" };
        json::object(vec![("kind", Value::str(kind)), ("specifier", Value::str(&import.specifier))])
    });
    let resolved = language.map_or_else(Vec::new, |language| found.iter().map(|import| optional(resolve_import(language, path, import, &tree))).collect());
    let scanned = scan_source(path, source);
    let source_value = scanned.as_ref().map_or(Value::Null, |scan| {
        let symbols = scan.symbols.iter().map(|symbol| {
            json::object(vec![
                ("first", Value::count(symbol.first)),
                ("kind", Value::str(&symbol.kind)),
                ("last", Value::count(symbol.last)),
                ("name", Value::str(&symbol.name)),
                ("signature", Value::str(&symbol.signature)),
            ])
        });
        json::object(vec![
            ("imports", Value::Arr(scan.imports.iter().map(|import| Value::str(import)).collect())),
            ("language", Value::str(scan.language)),
            ("symbols", Value::Arr(symbols.collect())),
        ])
    });
    let source_resolved = scanned.map_or_else(Vec::new, |scan| scan.imports.iter().map(|import| optional(resolve_source_import(path, import, &tree))).collect());
    json::object(vec![
        ("imports", Value::Arr(imports.collect())),
        ("language", language.map_or(Value::Null, |language| Value::str(language.name()))),
        ("resolved", Value::Arr(resolved)),
        ("source", source_value),
        ("source_resolved", Value::Arr(source_resolved)),
    ])
}

fn facts_expected(case: &Value) -> Value {
    let sources: Vec<ModuleSource> = list(case, "sources")
        .iter()
        .map(|source| ModuleSource { path: text(source, "path").to_string(), text: field(source, "text").as_str().map(str::to_string) })
        .collect();
    let tree: HashSet<String> = strings(case, "tree").into_iter().collect();
    let histories: Vec<FileHistory> = list(case, "histories")
        .iter()
        .map(|history| FileHistory {
            path: text(history, "path").to_string(),
            commits: list(history, "commits").iter().map(|change| LastChange { commit: text(change, "commit").to_string(), date: text(change, "date").to_string() }).collect(),
        })
        .collect();
    let facts = code_facts(&sources, &tree);
    let cycles = facts.cycles.iter().map(|cycle| Value::Arr(cycle.iter().map(|path| Value::str(path)).collect()));
    let facts_value = json::object(vec![
        ("cycles", Value::Arr(cycles.collect())),
        ("modules", Value::Arr(facts.modules.iter().map(module_value).collect())),
        ("notes", Value::Arr(facts.notes.iter().map(|note| Value::str(note)).collect())),
    ]);
    json::object(vec![("facts", facts_value), ("history", Value::Arr(history_facts(&histories).iter().map(history_value).collect()))])
}

fn mention_expected(case: &Value) -> Value {
    let (source, name) = (text(case, "text"), text(case, "name"));
    json::object(vec![("lines", Value::Arr(mention_lines(source, name).into_iter().map(Value::count).collect())), ("mentions", Value::Bool(mentions(source, name)))])
}

fn posix_expected(case: &Value) -> Value {
    let (path, base, other) = (text(case, "path"), text(case, "base"), text(case, "other"));
    json::object(vec![
        ("dirname", Value::str(&posix_dirname(path))),
        ("extname", Value::str(posix_extname(path))),
        ("join", Value::str(&posix_join(base, other))),
        ("normalize", Value::str(&normalize(path))),
    ])
}

fn replay(index: usize, case: &Value) {
    let actual = match text(case, "kind") {
        "scan" => scan_expected(case),
        "facts" => facts_expected(case),
        "posix" => posix_expected(case),
        _ => mention_expected(case),
    };
    let (want, got) = (json::compact(&canonical(field(case, "expected"))), json::compact(&canonical(&actual)));
    assert_eq!(want, got, "case {index}: {}", json::compact(&Value::Obj({
        let mut shown = Object::new();
        for key in ["path", "text", "tree", "sources", "name", "base", "other"] {
            if let Some(value) = case.as_object().and_then(|object| object.get(key)) {
                shown.set(key, value.clone());
            }
        }
        shown
    })));
}

#[test]
fn scan_corpus() {
    let corpus = match std::env::var("CRITIQUE_SCAN_CORPUS") {
        Ok(path) => std::fs::read_to_string(&path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/scan-corpus.json").to_string(),
    };
    let cases = match json::parse(&corpus) {
        Ok(Value::Arr(items)) => items,
        other => panic!("the corpus is not a JSON array: {other:?}"),
    };
    assert!(!cases.is_empty(), "the corpus holds no case");
    for (index, case) in cases.iter().enumerate() {
        replay(index, case);
    }
}
