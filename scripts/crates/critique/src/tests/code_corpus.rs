//! The source graph, repo map, context slices, windowed diff and code sample over generated modules, diffs and limits,
//! compared with what the node runner makes of the same input (tests/gen-code-corpus.mjs). `CRITIQUE_CODE_CORPUS`
//! names a bigger corpus to replay.

use crate::code_sample::{sample_code_tree, CodeSampleRequest};
use crate::context_slices::{changed_lines, context_slices, ChangedFile, ContextSlice};
use crate::diff_filter::LeftOutFile;
use crate::diff_window::{diff_drop_line, window_diff};
use crate::ranking::{RankedFile, RankedSymbol};
use crate::repo_map::{build_repo_map, render_code_nav_map, render_repo_map, RenderedRepoMap, SourceGraph};
use crate::repository_packet::{default_packet_limits, TrackedFile};
use crate::tracked_tree::Excluded;
use crate::tree_sample::LineRange;
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

fn reason(name: &str) -> &'static str {
    match name {
        "callee" => "callee",
        "caller" => "caller",
        _ => "changed",
    }
}

fn slice_of(value: &Value) -> ContextSlice {
    ContextSlice {
        path: text(value, "path"),
        first: count(value, "first"),
        last: count(value, "last"),
        reason: reason(&text(value, "reason")),
        symbol: optional_text(value, "symbol"),
        of: optional_text(value, "of"),
    }
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

fn rendered_of(value: &Value) -> RenderedRepoMap {
    RenderedRepoMap {
        text: text(value, "text"),
        listed: count(value, "listed"),
        dropped: count(value, "dropped"),
        source: if text(value, "source") == "code-nav" { "code-nav" } else { "scan" },
    }
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

fn side_value(start: f64, count: f64) -> Value {
    json::object(vec![("count", Value::Num(count)), ("start", Value::Num(start))])
}

fn replay_diff(input: &Value) -> Value {
    let windowed = window_diff(&text(input, "diff"), number(input, "budget") as i64);
    let dropped = windowed.dropped.iter().map(|drop| {
        let hunks = drop.hunks.iter().map(|hunk| json::object(vec![("new", side_value(hunk.new.start, hunk.new.count)), ("old", side_value(hunk.old.start, hunk.old.count)), ("text", Value::str(&hunk.text))]));
        json::object(vec![("hunks", Value::Arr(hunks.collect())), ("path", Value::str(&drop.path))])
    });
    json::object(vec![
        ("lines", texts(&windowed.dropped.iter().map(diff_drop_line).collect::<Vec<_>>())),
        ("windowed", json::object(vec![("diff", Value::Str(windowed.diff.clone())), ("dropped", Value::Arr(dropped.collect()))])),
    ])
}

/// A held range as the node runner writes it: a range its merging rebuilt has its two numbers, and one it took as
/// offered, a context slice, keeps the slice's fields.
fn range_value(range: &LineRange, request: &CodeSampleRequest) -> Value {
    let mut pairs = vec![("first".to_string(), Value::count(range.first)), ("last".to_string(), Value::count(range.last))];
    for (key, value) in range.origin.map(|at| request.slices[at].extra_fields()).unwrap_or_default() {
        pairs.push((key.to_string(), Value::Str(value)));
    }
    let mut object = json::Object::new();
    for (key, value) in pairs {
        object.set(&key, value);
    }
    Value::Obj(object)
}

fn replay_code(input: &Value) -> Value {
    let given = field(input, "limits").expect("limits");
    let mut limits = default_packet_limits();
    limits.cap_bytes = number(given, "cap_bytes");
    limits.facts_cap_bytes = number(given, "facts_cap_bytes");
    limits.manifest_cap_bytes = number(given, "manifest_cap_bytes");
    limits.repo_map_cap_bytes = number(given, "repo_map_cap_bytes");
    limits.window_lines = number(given, "window_lines");
    let tracked: Vec<TrackedFile> = list(input, "tracked").iter().map(|file| TrackedFile { path: text(file, "path"), text: optional_text(file, "text") }).collect();
    let excluded = field(input, "excluded").expect("excluded");
    let left_out: Vec<LeftOutFile> = list(input, "leftOut").iter().map(|file| LeftOutFile { path: text(file, "path"), reason: text(file, "reason"), changed_lines: number(file, "changed_lines") }).collect();
    let (changed, importers) = (strings(input, "changed"), strings(input, "importers"));
    let slices: Vec<ContextSlice> = list(input, "slices").iter().map(slice_of).collect();
    let ranked = field(input, "ranked").map(|_| strings(input, "ranked"));
    let repo_map = rendered_of(field(input, "repoMap").expect("repoMap"));
    let diff = text(input, "diff");
    let named = text(input, "namedText");
    let note = optional_text(input, "rankingNote");
    let request = CodeSampleRequest {
        commit: "c0ffee",
        limits: &limits,
        tracked: &tracked,
        excluded: Excluded { generated: count(excluded, "generated"), lock: count(excluded, "lock"), vendored: count(excluded, "vendored") },
        left_out: &left_out,
        diff: &diff,
        document_bytes: number(input, "documentBytes") as i64,
        repo_map: &repo_map,
        changed: &changed,
        importers: &importers,
        slices: &slices,
        named_text: &named,
        ranked: ranked.as_deref(),
        ranking_note: note.as_deref(),
    };
    match sample_code_tree(&request) {
        Err(refusal) => refusal,
        Ok(sample) => {
            let tree = sample.tree.iter().map(|file| {
                let mut pairs = vec![("path", Value::str(&file.path)), ("text", Value::str(&file.text))];
                if let Some(window) = &file.window {
                    let mut held = vec![("last", Value::count(window.last)), ("lines", Value::count(window.lines))];
                    if let Some(ranges) = &window.ranges {
                        let ranges = ranges.iter().map(|range| range_value(range, &request));
                        held.push(("ranges", Value::Arr(ranges.collect())));
                    }
                    pairs.push(("window", json::object(held)));
                }
                json::object(pairs)
            });
            json::object(vec![
                ("diff", Value::Str(sample.diff)),
                ("dropped", Value::count(sample.dropped)),
                ("index", Value::Str(sample.index)),
                ("slices", Value::Arr(sample.slices.iter().map(slice_value).collect())),
                ("status", Value::str("sampled")),
                ("tree", Value::Arr(tree.collect())),
            ])
        }
    }
}

#[test]
fn code_corpus() {
    let source = match std::env::var("CRITIQUE_CODE_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/code.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 150);
    for (index, case) in cases.iter().enumerate() {
        let input = field(case, "input").expect("input");
        let actual = match text(case, "kind").as_str() {
            "graph" => replay_graph(input),
            "diff" => replay_diff(input),
            "lines" => Value::Arr(changed_lines(optional_text(input, "before").as_deref(), &text(input, "after")).into_iter().map(Value::count).collect()),
            _ => replay_code(input),
        };
        let expected = field(case, "expected").expect("expected");
        if let (Ok(dir), true) = (std::env::var("CRITIQUE_CORPUS_DUMP"), json::compact(&canonical(&actual)) != json::compact(&canonical(expected))) {
            std::fs::write(format!("{dir}/actual.json"), json::pretty(&canonical(&actual))).expect("dump");
            std::fs::write(format!("{dir}/expected.json"), json::pretty(&canonical(expected))).expect("dump");
        }
        assert_eq!(json::compact(&canonical(&actual)), json::compact(&canonical(expected)), "case {index} differs\n  case: {}", json::compact(case));
    }
}
