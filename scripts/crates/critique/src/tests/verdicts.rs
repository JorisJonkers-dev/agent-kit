//! verdicts' own rules and the corpus the node runner's verdict rules, reader and finding reader were replayed over.

use super::findings::{parse_value, run_finding_result, strings};
use crate::error::ShapeError;
use crate::fields::Fields;
use crate::findings::{parse_findings, read_run_finding, RunFinding};
use crate::manifest::PacketFile;
use crate::verdicts::{
    candidate_questions, count_verdicts, degraded_ticket_review, open_critical_questions, read_verdict, second_round_due, severity_override_problem,
    stage_verdict_error, unfixed_critical, verdict_problem, AdjudicatedFinding, Verdict,
};
use model_gateway::json::{self, Value};

fn text(value: &Value, key: &str) -> Option<String> {
    value.as_object()?.get(key)?.as_str().map(str::to_string)
}

fn flag(value: &Value, key: &str) -> Option<bool> {
    match value.as_object()?.get(key)? {
        Value::Bool(flag) => Some(*flag),
        _ => None,
    }
}

/// A verdict as the corpus holds it, field by field, with none of the reader's checks.
fn verdict_of(value: &Value) -> Verdict {
    let tickets = match value.as_object().and_then(|object| object.get("trace_tickets")) {
        Some(Value::Arr(items)) => Some(items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect()),
        _ => None,
    };
    Verdict {
        finding_id: text(value, "finding_id").unwrap_or_default(),
        verdict: text(value, "verdict").unwrap_or_default(),
        reason: text(value, "reason"),
        reason_code: text(value, "reason_code"),
        answered: flag(value, "answered"),
        follow_up: text(value, "follow_up"),
        trace_tickets: tickets,
        regression_test: text(value, "regression_test"),
        fixed: flag(value, "fixed"),
        severity: text(value, "severity"),
        severity_reason: text(value, "severity_reason"),
    }
}

fn verdict_value(verdict: &Verdict) -> Value {
    let mut pairs = vec![("finding_id", Value::str(&verdict.finding_id)), ("verdict", Value::str(&verdict.verdict))];
    let mut put = |key: &'static str, value: Option<Value>| {
        if let Some(value) = value {
            pairs.push((key, value));
        }
    };
    put("reason", verdict.reason.as_deref().map(Value::str));
    put("reason_code", verdict.reason_code.as_deref().map(Value::str));
    put("answered", verdict.answered.map(Value::Bool));
    put("follow_up", verdict.follow_up.as_deref().map(Value::str));
    put("trace_tickets", verdict.trace_tickets.as_deref().map(strings));
    put("regression_test", verdict.regression_test.as_deref().map(Value::str));
    put("fixed", verdict.fixed.map(Value::Bool));
    put("severity", verdict.severity.as_deref().map(Value::str));
    put("severity_reason", verdict.severity_reason.as_deref().map(Value::str));
    json::object(pairs)
}

fn read_verdict_result(value: &Value) -> Value {
    match Fields::of(value, "").and_then(|fields| read_verdict(&fields)) {
        Ok(verdict) => json::object(vec![("ok", verdict_value(&verdict))]),
        Err(ShapeError(message)) => json::object(vec![("error", Value::str(&format!("JsonShapeError: {message}")))]),
    }
}

fn packet_files(value: &Value) -> Vec<PacketFile> {
    let Some(Value::Arr(items)) = value.as_object().and_then(|object| object.get("packet_files")) else { return Vec::new() };
    items
        .iter()
        .map(|item| PacketFile {
            path: text(item, "path").unwrap_or_default(),
            source: String::new(),
            kind: text(item, "kind").unwrap_or_default(),
            sha256: String::new(),
            window: None,
        })
        .collect()
}

fn adjudicated(value: &Value) -> Vec<AdjudicatedFinding> {
    let Some(Value::Arr(items)) = value.as_object().and_then(|object| object.get("findings")) else { return Vec::new() };
    items
        .iter()
        .map(|item| {
            let finding: RunFinding = read_run_finding(&Fields::of(item, "").expect("object")).expect("a valid finding");
            AdjudicatedFinding { finding, verdict: verdict_of(item.as_object().and_then(|object| object.get("verdict")).expect("a verdict")) }
        })
        .collect()
}

fn ids(findings: &[AdjudicatedFinding], positions: &[usize]) -> Value {
    Value::Arr(positions.iter().map(|position| Value::str(&findings[*position].finding.id)).collect())
}

fn id_list(found: Vec<&AdjudicatedFinding>) -> Value {
    Value::Arr(found.into_iter().map(|finding| Value::str(&finding.finding.id)).collect())
}

fn field<'a>(case: &'a Value, key: &str) -> &'a Value {
    case.as_object().and_then(|object| object.get(key)).unwrap_or_else(|| panic!("case has no {key}"))
}

fn option_text(found: Option<String>) -> Value {
    found.map_or(Value::Null, |message| Value::str(&message))
}

#[test]
fn the_readers_and_rules_answer_as_the_node_runner_did_over_the_corpus() {
    let corpus = json::parse(include_str!("../../tests/fixtures/verdicts.json")).expect("corpus");
    let Value::Arr(cases) = corpus else { panic!("a list") };
    assert!(cases.len() >= 400);
    for (number, case) in cases.iter().enumerate() {
        let kind = text(case, "kind").expect("kind");
        let expected = json::compact(&match kind.as_str() {
            "reply" => field(case, "expected").clone(),
            "finding" | "verdict" => field(case, "expected").clone(),
            _ => case.clone(),
        });
        let found = match kind.as_str() {
            "reply" => parse_value(&parse_findings(&text(case, "text").expect("text"))),
            "finding" => run_finding_result(field(case, "value")),
            "verdict" => read_verdict_result(field(case, "value")),
            "rules" => {
                let verdict = verdict_of(field(case, "verdict"));
                let stage = text(case, "stage").expect("stage");
                let finding = read_run_finding(&Fields::of(field(case, "finding"), "").expect("object")).expect("finding");
                let answers = vec![
                    ("stage_error", option_text(stage_verdict_error(&stage, &verdict, text(case, "category").as_deref()))),
                    ("problem", option_text(verdict_problem(&stage, &verdict, &packet_files(case)))),
                    ("override", option_text(severity_override_problem(&finding, &verdict))),
                ];
                let mut shown = case.as_object().expect("object").clone();
                for (key, value) in answers {
                    shown.set(key, value);
                }
                Value::Obj(shown)
            }
            _ => {
                let stage = text(case, "stage").expect("stage");
                let findings = adjudicated(case);
                let round = match field(case, "round") {
                    Value::Num(number) => *number,
                    _ => panic!("a round"),
                };
                let counts = count_verdicts(&findings);
                let questions = match candidate_questions(&stage, &findings) {
                    Some(found) => json::object(vec![("asked", ids(&findings, &found.asked)), ("deferred", ids(&findings, &found.deferred))]),
                    None => Value::Null,
                };
                let mut shown = case.as_object().expect("object").clone();
                shown.set(
                    "counts",
                    json::object(vec![
                        ("accepted", Value::count(counts[0])),
                        ("hint", Value::count(counts[1])),
                        ("needs-human", Value::count(counts[2])),
                        ("rejected", Value::count(counts[3])),
                    ]),
                );
                shown.set("second_round", Value::Bool(second_round_due(&stage, round, &findings)));
                shown.set("questions", questions);
                shown.set("open", id_list(open_critical_questions(&findings)));
                shown.set("unfixed", id_list(unfixed_critical(&stage, &findings)));
                Value::Obj(shown)
            }
        };
        assert_eq!(json::compact(&found), expected, "case {number} ({kind})");
    }
}

#[test]
fn a_degraded_ticket_review_is_the_only_degraded_run_that_blocks_a_commit() {
    assert!(degraded_ticket_review("ticket-review", true));
    assert!(!degraded_ticket_review("ticket-review", false));
    assert!(!degraded_ticket_review("spec", true));
}

fn verdict(kind: &str, reason_code: Option<&str>) -> Verdict {
    Verdict { finding_id: "f1".to_string(), verdict: kind.to_string(), reason_code: reason_code.map(str::to_string), ..Verdict::default() }
}

fn tickets() -> Vec<PacketFile> {
    ["t/1.md", "t/2.md"]
        .iter()
        .map(|path| PacketFile { path: path.to_string(), source: String::new(), kind: "ticket".to_string(), sha256: String::new(), window: None })
        .collect()
}

#[test]
fn a_feature_finding_is_accepted_only_by_a_failing_test_or_a_trace_across_tickets() {
    let problem = |stage: &str, verdict: &Verdict| verdict_problem(stage, verdict, &tickets());
    let traced = |tickets: &[&str]| Verdict {
        trace_tickets: Some(tickets.iter().map(|path| path.to_string()).collect()),
        follow_up: Some("#9".to_string()),
        ..verdict("accepted", Some("cross-ticket-trace"))
    };
    assert_eq!(problem("spec", &Verdict { follow_up: Some("#9".to_string()), ..verdict("accepted", None) }), Some("f1: follow_up and trace_tickets belong to a feature-review verdict".to_string()));
    assert_eq!(problem("spec", &verdict("accepted", None)), None);
    assert_eq!(problem("feature-review", &Verdict { follow_up: Some("#9".to_string()), ..verdict("rejected", None) }), Some("f1: only an accepted feature finding has a follow_up or trace_tickets".to_string()));
    assert_eq!(problem("feature-review", &verdict("hint", None)), None);
    assert_eq!(problem("feature-review", &verdict("accepted", Some("refuted"))), Some("f1: a feature finding is accepted only with reason_code test-fails or cross-ticket-trace".to_string()));
    assert_eq!(problem("feature-review", &verdict("accepted", None)), Some("f1: a feature finding is accepted only with reason_code test-fails or cross-ticket-trace".to_string()));
    let crossing = Some("f1: a cross-ticket-trace names at least two tickets of the packet in trace_tickets".to_string());
    assert_eq!(problem("feature-review", &verdict("accepted", Some("cross-ticket-trace"))), crossing);
    assert_eq!(problem("feature-review", &traced(&["t/1.md", "t/1.md"])), crossing);
    assert_eq!(problem("feature-review", &traced(&["t/1.md", "zzz"])), crossing);
    assert_eq!(problem("feature-review", &traced(&["t/1.md", "t/2.md"])), None);
    let unfollowed = Verdict { follow_up: None, ..traced(&["t/2.md", "t/1.md"]) };
    assert_eq!(problem("feature-review", &unfollowed), Some("f1: an accepted feature finding names its follow-up ticket in follow_up".to_string()));
    let tested = Verdict { follow_up: Some("#9".to_string()), ..verdict("accepted", Some("test-fails")) };
    assert_eq!(problem("feature-review", &tested), None);
    let stray = Verdict { trace_tickets: Some(vec!["t/1.md".to_string()]), ..verdict("accepted", Some("test-fails")) };
    assert_eq!(problem("feature-review", &stray), Some("f1: trace_tickets belong to a cross-ticket-trace".to_string()));
}

#[test]
fn a_ticket_finding_is_accepted_only_by_a_failing_test_or_a_walked_trace() {
    let problem = |verdict: &Verdict| verdict_problem("ticket-review", verdict, &[]);
    assert_eq!(verdict_problem("tickets", &Verdict { fixed: Some(true), ..verdict("accepted", None) }, &[]), Some("f1: regression_test and fixed belong to a ticket-review verdict".to_string()));
    assert_eq!(problem(&Verdict { fixed: Some(false), ..verdict("rejected", None) }), Some("f1: only an accepted ticket finding has a regression_test or is fixed".to_string()));
    assert_eq!(problem(&verdict("rejected", None)), None);
    assert_eq!(problem(&verdict("accepted", None)), Some("f1: a ticket finding is accepted only with reason_code test-fails or trace-walked".to_string()));
    assert_eq!(problem(&verdict("accepted", Some("test-fails"))), Some("f1: a finding accepted by test-fails names the test kept in the ticket's change in regression_test".to_string()));
    assert_eq!(problem(&Verdict { regression_test: Some("a.test.ts".to_string()), ..verdict("accepted", Some("test-fails")) }), None);
    assert_eq!(problem(&Verdict { regression_test: Some("a.test.ts".to_string()), ..verdict("accepted", Some("trace-walked")) }), Some("f1: regression_test belongs to a finding accepted by test-fails".to_string()));
    assert_eq!(problem(&verdict("accepted", Some("trace-walked"))), None);
}

#[test]
fn a_recovered_decision_is_never_accepted_and_the_decisions_stage_asks_every_accepted_gap() {
    let error = |stage: &str, verdict: &Verdict, category: Option<&str>| stage_verdict_error(stage, verdict, category);
    assert!(error("decision-recovery", &verdict("accepted", None), None).expect("refused").contains("never accepted"));
    assert_eq!(error("architecture-review", &verdict("accepted", None), Some("bug")), None);
    assert!(error("architecture-review", &verdict("accepted", None), Some("undocumented-decision")).is_some());
    assert!(error("decisions", &verdict("needs-human", None), None).expect("refused").contains("not needs-human"));
    assert_eq!(error("decisions", &Verdict { answered: Some(true), ..verdict("rejected", None) }, None), Some("f1: only an accepted verdict can be answered at the decisions stage".to_string()));
    assert_eq!(error("decisions", &Verdict { answered: Some(true), ..verdict("accepted", None) }, None), None);
    assert_eq!(error("spec", &Verdict { answered: Some(true), ..verdict("accepted", None) }, None), Some("f1: only a needs-human verdict can be answered".to_string()));
    assert_eq!(error("spec", &Verdict { answered: Some(true), ..verdict("needs-human", None) }, None), None);
}

#[test]
fn a_severity_override_needs_a_declared_severity_a_different_level_and_a_reason() {
    let finding = |declared: Option<&str>| {
        let value = json::parse(&format!(
            r#"{{"id":"f1","critic_finding_id":"c","raised_by":[],"severity":"major","category":"bug","location":"l","claim":"c","evidence":[]{}}}"#,
            declared.map_or(String::new(), |level| format!(r#","declared_severity":"{level}""#))
        ))
        .expect("json");
        read_run_finding(&Fields::of(&value, "").expect("object")).expect("finding")
    };
    let override_with = |severity: Option<&str>, reason: Option<&str>| Verdict {
        severity: severity.map(str::to_string),
        severity_reason: reason.map(str::to_string),
        ..verdict("accepted", None)
    };
    assert_eq!(severity_override_problem(&finding(None), &override_with(None, None)), None);
    assert_eq!(severity_override_problem(&finding(None), &override_with(None, Some("why"))), Some("f1: a severity_reason belongs to a severity override".to_string()));
    assert_eq!(severity_override_problem(&finding(None), &override_with(Some("minor"), Some("why"))), Some("f1: only a finding whose check fixes its severity can be overridden".to_string()));
    assert_eq!(severity_override_problem(&finding(Some("minor")), &override_with(Some("minor"), Some("why"))), Some("f1: the override repeats the declared severity minor".to_string()));
    assert_eq!(severity_override_problem(&finding(Some("minor")), &override_with(Some("major"), None)), Some("f1: a severity override needs a severity_reason".to_string()));
    assert_eq!(severity_override_problem(&finding(Some("minor")), &override_with(Some("major"), Some("why"))), None);
}
