//! The merge of several critics' findings over generated findings, compared with what the node runner's
//! `domain/merge.ts` makes of them, key order included.

use crate::fan_out_command::finding_json;
use crate::findings::{CriticFinding, Evidence, FindingBody};
use crate::merge::{merge_findings, CriticFindings};
use model_gateway::json::{self, Value};

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object().and_then(|object| object.get(key))
}

fn text(value: &Value, key: &str) -> String {
    field(value, key).and_then(Value::as_str).unwrap_or_else(|| panic!("no {key}")).to_string()
}

fn items<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match field(value, key) {
        Some(Value::Arr(items)) => items,
        _ => panic!("no list {key}"),
    }
}

fn finding(value: &Value) -> CriticFinding {
    CriticFinding {
        id: text(value, "id"),
        body: FindingBody {
            severity: text(value, "severity"),
            category: text(value, "category"),
            location: text(value, "location"),
            claim: text(value, "claim"),
            evidence: items(value, "evidence")
                .iter()
                .map(|item| Evidence { kind: text(item, "kind"), value: text(item, "value"), file: field(item, "file").map(|_| text(item, "file")) })
                .collect(),
            suggestion: field(value, "suggestion").map(|_| text(value, "suggestion")),
            check: field(value, "check").map(|_| text(value, "check")),
        },
    }
}

/// Replays the corpus; `CRITIQUE_MERGE_FINDINGS_CORPUS` names a bigger one.
#[test]
fn merge_findings_corpus() {
    let source = match std::env::var("CRITIQUE_MERGE_FINDINGS_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/merge-findings-corpus.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 100);
    for (index, case) in cases.iter().enumerate() {
        let critics: Vec<CriticFindings> =
            items(case, "critics").iter().map(|critic| CriticFindings { critic_id: text(critic, "critic_id"), findings: items(critic, "findings").iter().map(finding).collect() }).collect();
        let merged = Value::Arr(merge_findings(&critics).iter().map(finding_json).collect());
        // The node runner's merged findings carry no `declared_severity` or `pre_verdict`, which fan-out adds after.
        assert_eq!(json::compact(&merged), text(case, "expected"), "case {index} differs\n  case: {}", json::compact(case));
    }
}
