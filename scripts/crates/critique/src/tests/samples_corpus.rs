//! The tree sampler, the drop summary and the repository sample over generated trees, limits and pieces, compared with
//! what the node runner makes of the same input (tests/gen-samples-corpus.mjs). `CRITIQUE_SAMPLES_CORPUS` names a
//! bigger corpus to replay.

use crate::drop_summary::{drop_summary_bound, summarise_drops, Drop};
use crate::repository_packet::{default_packet_limits, TrackedFile};
use crate::repository_sample::{sample_repository, FactFile, RepositoryFacts, SampleRequest};
use crate::tracked_tree::Excluded;
use crate::tree_sample::{admit_pieces, candidate_of, format_ranges, held_text, sampled_file, sent_status, Candidate, LineRange, Piece};
use model_gateway::json::{self, Value};
use std::collections::HashMap;

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

fn count(value: &Value, key: &str) -> usize {
    number(value, key) as usize
}

fn texts(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

fn range_value(range: &LineRange) -> Value {
    json::object(vec![("first", Value::count(range.first)), ("last", Value::count(range.last))])
}

fn ranges_value(ranges: &[LineRange]) -> Value {
    Value::Arr(ranges.iter().map(range_value).collect())
}

fn ranges(value: &Value, key: &str) -> Vec<LineRange> {
    list(value, key).iter().map(|range| LineRange { first: count(range, "first"), last: count(range, "last") }).collect()
}

fn tier(name: &str) -> &'static str {
    if name == "named" {
        "named"
    } else {
        "rest"
    }
}

fn replay_sample(input: &Value) -> Value {
    let mut limits = default_packet_limits();
    let given = field(input, "limits").expect("limits");
    limits.cap_bytes = number(given, "cap_bytes");
    limits.facts_cap_bytes = number(given, "facts_cap_bytes");
    limits.manifest_cap_bytes = number(given, "manifest_cap_bytes");
    limits.window_lines = number(given, "window_lines");
    let tracked: Vec<TrackedFile> = list(input, "tracked").iter().map(|file| TrackedFile { path: text(file, "path"), text: field(file, "text").and_then(Value::as_str).map(str::to_string) }).collect();
    let excluded = field(input, "excluded").expect("excluded");
    let facts = field(input, "facts").expect("facts");
    let repository_facts = RepositoryFacts {
        files: list(facts, "files").iter().map(|file| FactFile { name: text(file, "name"), holds: text(file, "holds"), text: text(file, "text") }).collect(),
        hotspots: field(facts, "hotspots").map(|_| strings(facts, "hotspots")),
        cycles: field(facts, "cycles").map(|cycles| match cycles {
            Value::Arr(items) => items.iter().map(|cycle| match cycle {
                Value::Arr(paths) => paths.iter().map(|path| path.as_str().expect("a path").to_string()).collect(),
                _ => panic!("cycle"),
            }).collect(),
            _ => panic!("cycles"),
        }),
        churn_ranks: field(facts, "churn").map(|churn| {
            let Value::Obj(object) = churn else { panic!("churn") };
            object.entries().into_iter().map(|(path, rank)| (path.to_string(), if let Value::Num(rank) = rank { *rank } else { panic!("rank") })).collect::<HashMap<_, _>>()
        }),
    };
    let documents = strings(input, "documents");
    let rest = field(input, "rest").map(|_| strings(input, "rest"));
    let note = field(input, "rankingNote").and_then(Value::as_str);
    let request = SampleRequest {
        commit: "c0ffee",
        limits: &limits,
        tracked: &tracked,
        excluded: Excluded { generated: count(excluded, "generated"), lock: count(excluded, "lock"), vendored: count(excluded, "vendored") },
        documents: &documents,
        document_bytes: number(input, "documentBytes") as i64,
        artifact_text: &text(input, "artifactText"),
        facts: &repository_facts,
        rest: rest.as_deref(),
        ranking_note: note,
    };
    match sample_repository(&request) {
        Err(refusal) => refusal,
        Ok(sample) => {
            let tree = sample.tree.iter().map(|file| {
                let mut pairs = vec![("path", Value::str(&file.path)), ("text", Value::str(&file.text))];
                if let Some(window) = &file.window {
                    let mut held = vec![("last", Value::count(window.last)), ("lines", Value::count(window.lines))];
                    if let Some(ranges) = &window.ranges {
                        held.push(("ranges", ranges_value(ranges)));
                    }
                    pairs.push(("window", json::object(held)));
                }
                json::object(pairs)
            });
            let facts = sample.facts.iter().map(|file| json::object(vec![("holds", Value::str(&file.holds)), ("name", Value::str(&file.name)), ("text", Value::str(&file.text))]));
            json::object(vec![
                ("dropped", Value::count(sample.dropped)),
                ("facts", Value::Arr(facts.collect())),
                ("index", Value::Str(sample.index)),
                ("manifest", Value::Str(sample.manifest)),
                ("status", Value::str("sampled")),
                ("tree", Value::Arr(tree.collect())),
            ])
        }
    }
}

fn replay_drops(input: &Value) -> Value {
    let drops: Vec<Drop> = list(input, "drops").iter().map(|drop| Drop { path: text(drop, "path"), reason: text(drop, "reason"), bytes: number(drop, "bytes") as i64 }).collect();
    json::object(vec![("bound", Value::Num(drop_summary_bound(&drops) as f64)), ("lines", texts(&summarise_drops(&drops)))])
}

fn replay_pieces(input: &Value) -> Value {
    let candidates: Vec<Candidate> = list(input, "candidates").iter().map(|c| candidate_of(&text(c, "path"), tier(&text(c, "tier")), &text(c, "text"))).collect();
    let pieces: Vec<Piece> = list(input, "pieces")
        .iter()
        .map(|piece| Piece {
            candidate: count(piece, "candidate"),
            ranges: ranges(piece, "ranges"),
            window: field(piece, "window") == Some(&Value::Bool(true)),
            note: field(piece, "note").map(|_| {
                let note = strings(piece, "note");
                (note[0].clone(), note[1].clone())
            }),
        })
        .collect();
    let mode = field(input, "describe").and_then(Value::as_str).map(str::to_string);
    let all = |candidate: &Candidate, held: &[LineRange]| Some(format!("- {}: {}", candidate.path, format_ranges(held)));
    let named = |candidate: &Candidate, held: &[LineRange]| if candidate.tier == "named" { all(candidate, held) } else { None };
    let admission = match mode.as_deref() {
        Some("all") => admit_pieces(&candidates, &pieces, number(input, "budget") as i64, Some(&all)),
        Some(_) => admit_pieces(&candidates, &pieces, number(input, "budget") as i64, Some(&named)),
        None => admit_pieces(&candidates, &pieces, number(input, "budget") as i64, None),
    };
    let held_of = |index: usize| admission.held[index].clone();
    let files = candidates.iter().enumerate().map(|(index, candidate)| match sampled_file(candidate, &held_of(index)) {
        None => Value::Null,
        Some(file) => {
            let mut pairs = vec![("path", Value::str(&file.path)), ("text", Value::str(&file.text))];
            if let Some(window) = &file.window {
                let mut held = vec![("last", Value::count(window.last)), ("lines", Value::count(window.lines))];
                if let Some(ranges) = &window.ranges {
                    held.push(("ranges", ranges_value(ranges)));
                }
                pairs.push(("window", json::object(held)));
            }
            json::object(pairs)
        }
    });
    json::object(vec![
        ("admitted", Value::Arr(admission.admitted.iter().map(|admitted| Value::Bool(*admitted)).collect())),
        ("files", Value::Arr(files.collect())),
        ("held", Value::Arr((0..candidates.len()).map(|index| ranges_value(&held_of(index))).collect())),
        ("status", Value::Arr(candidates.iter().enumerate().map(|(index, candidate)| Value::Str(sent_status(candidate, &held_of(index)))).collect())),
        ("texts", Value::Arr(candidates.iter().enumerate().map(|(index, candidate)| Value::Str(held_text(candidate, &held_of(index)))).collect())),
    ])
}

#[test]
fn samples_corpus() {
    let source = match std::env::var("CRITIQUE_SAMPLES_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/samples.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 200);
    for (index, case) in cases.iter().enumerate() {
        let input = field(case, "input").expect("input");
        let actual = match text(case, "kind").as_str() {
            "sample" => replay_sample(input),
            "drops" => replay_drops(input),
            _ => replay_pieces(input),
        };
        let expected = field(case, "expected").expect("expected");
        assert_eq!(json::compact(&canonical(&actual)), json::compact(&canonical(expected)), "case {index} differs\n  case: {}", json::compact(case));
    }
}
