//! The source graph, repo map and context slices over generated modules,
//! compared with what the node runner makes of the same input (tests/gen-code-corpus.mjs). `CRITIQUE_CODE_CORPUS`
//! names a bigger corpus to replay.

use crate::context_slices::{changed_lines, context_slices, ChangedFile, ContextSlice};
use crate::ranking::{RankedFile, RankedSymbol};
use crate::repo_map::{build_repo_map, render_code_nav_map, render_repo_map, RenderedRepoMap, SourceGraph};
use model_gateway::json::{self, Value};

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

fn optional_text(value: &Value, key: &str) -> Option<String> {
    field(value, key).and_then(Value::as_str).map(str::to_string)
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

fn count(value: &Value, key: &str) -> usize {
    number(value, key) as usize
}

fn texts(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

fn slice_value(slice: &ContextSlice) -> Value {
    let mut pairs = vec![("first", Value::count(slice.first)), ("last", Value::count(slice.last)), ("path", Value::str(&slice.path)), ("reason", Value::str(slice.reason))];
    if let Some(symbol) = &slice.symbol {
        pairs.push(("symbol", Value::str(symbol)));
    }
    if let Some(of) = &slice.of {
        pairs.push(("of", Value::str(of)));
    }
    json::object(pairs)
}

fn rendered_value(map: &RenderedRepoMap) -> Value {
    json::object(vec![("dropped", Value::count(map.dropped)), ("listed", Value::count(map.listed)), ("source", Value::str(map.source)), ("text", Value::str(&map.text))])
}

fn pairs_of(value: &Value) -> Vec<(String, String)> {
    list(value, "texts").iter().map(|file| (text(file, "path"), text(file, "text"))).collect()
}

fn replay_graph(input: &Value) -> Value {
    let pairs = pairs_of(input);
    let graph = SourceGraph::build(&pairs);
    let changed: Vec<ChangedFile> = list(input, "changed")
        .iter()
        .map(|file| ChangedFile { path: text(file, "path"), text: text(file, "text"), before: optional_text(file, "before") })
        .collect();
    let ranking = field(input, "ranking").expect("ranking");
    let files: Vec<RankedFile> = list(ranking, "files").iter().map(|file| RankedFile { path: text(file, "path"), refs: number(file, "refs") }).collect();
    let symbols: Vec<RankedSymbol> = list(ranking, "symbols")
        .iter()
        .map(|symbol| RankedSymbol { path: text(symbol, "path"), line: number(symbol, "line"), kind: text(symbol, "kind"), name: text(symbol, "name"), refs: number(symbol, "refs") })
        .collect();
    let cap = number(input, "capBytes") as i64;
    let map = build_repo_map(&graph);
    let map_value = map.iter().map(|entry| {
        let symbols = entry.symbols.iter().map(|symbol| {
            json::object(vec![
                ("kind", Value::str(&symbol.kind)),
                ("line", Value::count(symbol.line)),
                ("name", Value::str(&symbol.name)),
                ("references", Value::count(symbol.references)),
                ("signature", Value::str(&symbol.signature)),
            ])
        });
        json::object(vec![("path", Value::str(&entry.path)), ("references", Value::count(entry.references)), ("symbols", Value::Arr(symbols.collect()))])
    });
    json::object(vec![
        ("slices", Value::Arr(context_slices(&graph, &changed).iter().map(slice_value).collect())),
        ("map", Value::Arr(map_value.collect())),
        ("scan", rendered_value(&render_repo_map(&map, "c0ffee", cap))),
        ("nav", rendered_value(&render_code_nav_map(&files, &symbols, "c0ffee", cap))),
        ("importers", Value::Arr(pairs.iter().map(|(path, _)| texts(&graph.importers(path))).collect())),
        (
            "lines",
            Value::Arr(changed.iter().map(|file| Value::Arr(changed_lines(file.before.as_deref(), &file.text).into_iter().map(Value::count).collect())).collect()),
        ),
    ])
}

#[test]
fn code_corpus() {
    let source = match std::env::var("CRITIQUE_CODE_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/code.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 100);
    for (index, case) in cases.iter().enumerate() {
        let input = field(case, "input").expect("input");
        let actual = match text(case, "kind").as_str() {
            "lines" => Value::Arr(changed_lines(optional_text(input, "before").as_deref(), &text(input, "after")).into_iter().map(Value::count).collect()),
            _ => replay_graph(input),
        };
        let expected = field(case, "expected").expect("expected");
        if let (Ok(dir), true) = (std::env::var("CRITIQUE_CORPUS_DUMP"), json::compact(&canonical(&actual)) != json::compact(&canonical(expected))) {
            std::fs::write(format!("{dir}/actual.json"), json::pretty(&canonical(&actual))).expect("dump");
            std::fs::write(format!("{dir}/expected.json"), json::pretty(&canonical(expected))).expect("dump");
        }
        assert_eq!(json::compact(&canonical(&actual)), json::compact(&canonical(expected)), "case {index} differs\n  case: {}", json::compact(case));
    }
}
