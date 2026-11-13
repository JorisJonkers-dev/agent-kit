//! The evidence check, line references, declared checks and skipped checks over generated packets and findings,
//! compared with what the node runner makes of them.

use crate::checks::{check_named, check_runnable, declared_checks, with_declared_severity, DeclaredCheck};
use crate::evidence::{check_evidence, index_packet, line_references, skipped_checks, PacketText};
use crate::findings::{Evidence, FindingBody, PreVerdict, RunFinding};
use crate::manifest::{LineRange, PacketWindow};
use model_gateway::json::{self, Value};

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object().and_then(|object| object.get(key))
}

fn text(value: &Value, key: &str) -> String {
    field(value, key).and_then(Value::as_str).unwrap_or_else(|| panic!("no string {key}")).to_string()
}

fn number(value: &Value) -> f64 {
    match value {
        Value::Num(number) => *number,
        _ => panic!("not a number"),
    }
}

fn items(value: &Value, key: &str) -> Vec<Value> {
    match field(value, key) {
        Some(Value::Arr(items)) => items.clone(),
        _ => panic!("no list {key}"),
    }
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    items(value, key).iter().map(|item| item.as_str().expect("a string").to_string()).collect()
}

fn packet(case: &Value) -> Vec<PacketText> {
    items(case, "packet")
        .iter()
        .map(|file| PacketText {
            path: text(file, "path"),
            kind: text(file, "kind"),
            text: text(file, "text"),
            window: field(file, "window").map(|window| PacketWindow {
                last: number(field(window, "last").expect("last")),
                lines: number(field(window, "lines").expect("lines")),
                ranges: field(window, "ranges").map(|_| {
                    items(window, "ranges").iter().map(|range| LineRange { first: number(field(range, "first").expect("first")), last: number(field(range, "last").expect("last")) }).collect()
                }),
            }),
        })
        .collect()
}

fn finding(value: &Value) -> FindingBody {
    FindingBody {
        severity: text(value, "severity"),
        category: text(value, "category"),
        location: text(value, "location"),
        claim: text(value, "claim"),
        evidence: items(value, "evidence").iter().map(|item| Evidence { kind: text(item, "kind"), value: text(item, "value"), file: None }).collect(),
        suggestion: field(value, "suggestion").map(|_| text(value, "suggestion")),
        check: field(value, "check").map(|_| text(value, "check")),
    }
}

fn verdict_json(verdict: &Option<PreVerdict>) -> Value {
    match verdict {
        None => Value::Null,
        Some(pre) => json::object(vec![("reason", Value::str(&pre.reason)), ("reason_code", Value::str(&pre.reason_code)), ("verdict", Value::str(&pre.verdict))]),
    }
}

fn declared_json(checks: &[DeclaredCheck]) -> Value {
    Value::Arr(
        checks
            .iter()
            .map(|check| {
                let mut pairs = vec![("id", Value::str(&check.id))];
                if let Some(severity) = &check.severity {
                    pairs.push(("severity", Value::str(severity)));
                }
                if check.needs_tree {
                    pairs.push(("needsTree", Value::Bool(true)));
                }
                json::object(pairs)
            })
            .collect(),
    )
}

fn skipped_json(skipped: &[crate::fan_out_result::SkippedCheck]) -> Value {
    Value::Arr(
        skipped
            .iter()
            .map(|entry| {
                let mut pairs = vec![("check", Value::str(&entry.check)), ("from", Value::str(&entry.from))];
                if let Some(about) = &entry.about {
                    pairs.push(("about", Value::str(about)));
                }
                json::object(pairs)
            })
            .collect(),
    )
}

fn canonical(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Arr(list) => Value::Arr(list.iter().map(sorted).collect()),
            Value::Obj(object) => {
                let mut entries = object.entries();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                let mut result = json::Object::new();
                for (key, item) in entries {
                    result.set(key, sorted(item));
                }
                Value::Obj(result)
            }
            other => other.clone(),
        }
    }
    json::compact(&sorted(value))
}

fn replay(index: usize, case: &Value) {
    let expected = field(case, "expected");
    let (actual, wanted) = match text(case, "kind").as_str() {
        "evidence" => {
            let tickets = strings(case, "tickets");
            let indexed = index_packet(&packet(case), &tickets);
            let role = field(case, "role").map(|_| text(case, "role"));
            let result = check_evidence(&finding(field(case, "finding").expect("finding")), &text(case, "stage"), &indexed, role.as_deref());
            (verdict_json(&result), expected.expect("expected").clone())
        }
        "refs" => {
            let found = line_references(&text(case, "text"));
            let list = found.iter().map(|r| json::object(vec![("first", Value::Num(r.first)), ("last", Value::Num(r.last)), ("path", Value::str(&r.path))])).collect();
            (Value::Arr(list), expected.expect("expected").clone())
        }
        _ => return replay_checks(index, case),
    };
    assert_eq!(canonical(&actual), canonical(&wanted), "case {index} differs\n  case: {}", json::compact(case));
}

fn replay_checks(index: usize, case: &Value) {
    let texts = strings(case, "texts");
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let declared = declared_checks(&refs);
    let same = |label: &str, actual: Value, wanted: &Value| assert_eq!(canonical(&actual), canonical(wanted), "case {index} {label} differs\n  case: {}", json::compact(case));
    same("declared", declared_json(&declared), field(case, "declared").expect("declared"));
    let probe = field(case, "probe").expect("probe");
    let check = field(probe, "check").map(|_| text(probe, "check"));
    same("named", verdict_json(&check_named(check.as_deref(), &declared)), field(case, "named").expect("named"));
    same("runnable", verdict_json(&check_runnable(check.as_deref(), &declared, false)), field(case, "runnable").expect("runnable"));
    same("runnable with a tree", verdict_json(&check_runnable(check.as_deref(), &declared, true)), field(case, "runnableTree").expect("runnableTree"));
    let body = FindingBody {
        severity: text(probe, "severity"),
        category: "bug".to_string(),
        location: "l".to_string(),
        claim: "c".to_string(),
        evidence: Vec::new(),
        suggestion: None,
        check,
    };
    let run = RunFinding { id: "f1".to_string(), body, critic_finding_id: "x".to_string(), raised_by: vec!["c1".to_string()], pre_verdict: None, declared_severity: None };
    let result = with_declared_severity(&run, &declared);
    let severity = field(case, "severity").expect("severity");
    assert_eq!(result.body.severity, text(severity, "severity"), "case {index} severity");
    assert_eq!(result.declared_severity, field(severity, "declared_severity").map(|_| text(severity, "declared_severity")), "case {index} declared severity");
    let sources = vec![("role a".to_string(), declared.clone()), ("lens b".to_string(), declared.iter().skip(1).cloned().collect())];
    for entry in items(case, "skipped") {
        let Value::Arr(pair) = entry else { panic!("a pair") };
        let stage = pair[0].as_str().expect("a stage");
        same("skipped", skipped_json(&skipped_checks(stage, false, &sources)), &pair[1]);
    }
    same("skipped with a tree", skipped_json(&skipped_checks("adr-review", true, &sources[..1])), field(case, "skippedTree").expect("skippedTree"));
}

/// Replays the corpus; `CRITIQUE_EVIDENCE_CORPUS` names a bigger one.
#[test]
fn evidence_corpus() {
    let source = match std::env::var("CRITIQUE_EVIDENCE_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/evidence-corpus.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 1000);
    for (index, case) in cases.iter().enumerate() {
        replay(index, case);
    }
}
