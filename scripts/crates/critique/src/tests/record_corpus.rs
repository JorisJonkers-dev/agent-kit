//! A corpus of generated runs (manifest, fan-out result, verdicts, refute round and gate), adjudicated, reported and
//! turned into ledger lines, compared with what the node runner's record did with them.

use crate::adjudication::{adjudicate_findings, build_run, checked_verdicts, check_refutes, unasked_answer};
use crate::decode::{decoded, read_input};
use crate::error::CritiqueError;
use crate::fan_out_result::parse_fan_out_result;
use crate::feature_review::parse_gate_record;
use crate::fields::Fields;
use crate::manifest::parse_manifest;
use crate::refute::parse_refute_file;
use crate::report::{render_report, render_summary};
use crate::run_ledger::ledger_lines;
use crate::verdicts::{candidate_questions, count_verdicts, read_verdict, second_round_due};
use model_gateway::json::{self, Value};

/// What a case's node run computed, as the corpus keeps it.
#[derive(Debug, PartialEq)]
struct Outcome {
    asked: Option<Vec<String>>,
    counts: [usize; 4],
    deferred: Option<Vec<String>>,
    ledger: Vec<String>,
    report: String,
    second_round_due: bool,
    summary: String,
}

fn member<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).unwrap_or_else(|| panic!("case has no {key}"))
}

fn text(value: &Value, key: &str) -> Option<String> {
    member(value, key).as_str().map(str::to_string)
}

/// What the command does with the case's files, up to the point where it would write.
fn adjudicate(case: &Value) -> Result<Outcome, CritiqueError> {
    let input = json::object(vec![("verdicts", member(case, "verdicts").clone())]);
    let list = read_input("record", Fields::of(&input, "").and_then(|fields| fields.list("verdicts", read_verdict)))?;
    let manifest = decoded("invalid-run", "/r/manifest.json", parse_manifest(&text(case, "manifest").expect("manifest")))?;
    let merged = decoded("invalid-run", "/r/findings.json", parse_fan_out_result(&text(case, "findings").expect("findings")))?;
    let verdicts = checked_verdicts(&manifest, &merged, &list)?;
    let refuted = match text(case, "refute") {
        Some(refute) => {
            let file = decoded("invalid-run", "/r/refute.json", parse_refute_file(&refute))?;
            check_refutes("/r/refute.json", &merged.findings, &file)?;
            Some(file)
        }
        None => None,
    };
    let results = refuted.as_ref().map(|file| file.results.clone());
    let findings = adjudicate_findings(&verdicts, &merged.findings, results.as_deref().unwrap_or(&[]))?;
    let gate = text(case, "gate").map(|gate| parse_gate_record(&gate).expect("a valid gate"));
    let run = build_run(&manifest, &merged, findings, refuted.as_ref(), gate);
    if let Some(error) = unasked_answer(&run) {
        return Err(error);
    }
    let candidates = candidate_questions(&run.stage, &run.findings);
    let ids = |positions: &[usize]| positions.iter().map(|position| run.findings[*position].finding.id.clone()).collect::<Vec<_>>();
    Ok(Outcome {
        asked: candidates.as_ref().map(|c| ids(&c.asked)),
        counts: count_verdicts(&run.findings),
        deferred: candidates.as_ref().map(|c| ids(&c.deferred)),
        ledger: ledger_lines(&run, "2026-11-03T12:00:00.000Z"),
        report: render_report(&run),
        second_round_due: second_round_due(&run.stage, run.round, &run.findings),
        summary: render_summary(&run, "/r/report.md"),
    })
}

fn strings(value: &Value) -> Vec<String> {
    match value {
        Value::Arr(items) => items.iter().map(|item| item.as_str().expect("a string").to_string()).collect(),
        _ => panic!("not a list"),
    }
}

fn optional_strings(value: &Value) -> Option<Vec<String>> {
    (*value != Value::Null).then(|| strings(value))
}

fn expected(case: &Value) -> Result<Outcome, CritiqueError> {
    let expected = member(case, "expected");
    if let Some(error) = expected.as_object().and_then(|object| object.get("error")) {
        return Err(CritiqueError::new(&text(error, "code").expect("code"), text(error, "message").expect("message")));
    }
    let ok = member(expected, "ok");
    let count = |key: &str| match member(member(ok, "counts"), key) {
        Value::Num(number) => *number as usize,
        _ => panic!("count is not a number"),
    };
    Ok(Outcome {
        asked: optional_strings(member(ok, "asked")),
        counts: [count("accepted"), count("hint"), count("needs-human"), count("rejected")],
        deferred: optional_strings(member(ok, "deferred")),
        ledger: strings(member(ok, "ledger")),
        report: text(ok, "report").expect("report"),
        second_round_due: *member(ok, "second_round_due") == Value::Bool(true),
        summary: text(ok, "summary").expect("summary"),
    })
}

/// Replays the corpus; `CRITIQUE_RECORD_CORPUS` names a bigger one.
#[test]
fn record_corpus() {
    let file = match std::env::var("CRITIQUE_RECORD_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/record-corpus.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&file) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 100, "the corpus is too small to mean anything");
    for (index, case) in cases.iter().enumerate() {
        assert_eq!(adjudicate(case), expected(case), "case {index}");
    }
}
