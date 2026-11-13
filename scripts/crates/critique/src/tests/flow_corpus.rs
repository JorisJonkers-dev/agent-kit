//! The flow state over generated sources, compared with what the node runner's `domain/flow.ts` computes: the state,
//! its next step, the state read back from (sometimes damaged) text, and the state file's name.

use crate::flow::{flow_state, flow_state_file_name, next_step, parse_flow_state, state_json, FlowFinding, FlowRun, FlowSources, SourceGate, SourceReview, SourceSkip, SourceTicket};
use crate::ledger::FlowEvent;
use model_gateway::json::{self, Value};

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).unwrap_or(&Value::Null)
}

fn text(value: &Value, key: &str) -> String {
    field(value, key).as_str().unwrap_or_default().to_string()
}

fn nullable(value: &Value, key: &str) -> Option<String> {
    field(value, key).as_str().map(str::to_string)
}

fn flag(value: &Value, key: &str) -> bool {
    *field(value, key) == Value::Bool(true)
}

fn number(value: &Value, key: &str) -> f64 {
    match field(value, key) {
        Value::Num(number) => *number,
        _ => 0.0,
    }
}

fn list(value: &Value, key: &str) -> Vec<Value> {
    match field(value, key) {
        Value::Arr(items) => items.clone(),
        _ => Vec::new(),
    }
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    list(value, key).iter().map(|item| item.as_str().unwrap_or_default().to_string()).collect()
}

fn event(value: &Value) -> FlowEvent {
    match text(value, "event").as_str() {
        "no-critique" => FlowEvent::NoCritique(flag(value, "value")),
        "landed" => FlowEvent::Landed(text(value, "ticket")),
        "answered" => FlowEvent::Answered { finding_id: text(value, "finding_id"), run_id: text(value, "run_id") },
        "draft" => FlowEvent::Draft(text(value, "draft")),
        _ => FlowEvent::Grilled,
    }
}

fn sources(value: &Value) -> FlowSources {
    let plan = field(value, "plan");
    FlowSources {
        draft: nullable(value, "draft"),
        events: list(value, "events").iter().map(event).collect(),
        findings: list(value, "findings")
            .iter()
            .map(|f| FlowFinding { answered: flag(f, "answered"), finding_id: text(f, "finding_id"), run_id: text(f, "run_id"), severity: text(f, "severity"), stage: text(f, "stage"), verdict: text(f, "verdict") })
            .collect(),
        flow: text(value, "flow"),
        gates: list(value, "gates").iter().map(|g| SourceGate { gate: text(g, "gate"), head_commit: text(g, "head_commit"), line: text(g, "line"), run_id: text(g, "run_id") }).collect(),
        head_commit: text(value, "head_commit"),
        now: text(value, "now"),
        plan_exists: flag(plan, "exists"),
        plan_path: nullable(plan, "path"),
        repository: text(value, "repository"),
        reviews: list(value, "reviews")
            .iter()
            .map(|r| SourceReview { degraded: flag(r, "degraded"), recorded_at: text(r, "recorded_at"), review: text(r, "review"), run_id: text(r, "run_id"), ticket: text(r, "ticket") })
            .collect(),
        runs: list(value, "runs")
            .iter()
            .map(|r| FlowRun { recorded: flag(r, "recorded"), round: number(r, "round"), run_id: text(r, "run_id"), stage: text(r, "stage"), ticket_check: nullable(r, "ticket_check") })
            .collect(),
        skipped_reviews: list(value, "skipped_reviews").iter().map(|s| SourceSkip { recorded_at: text(s, "recorded_at"), run_id: text(s, "run_id"), ticket: text(s, "ticket") }).collect(),
        spec: nullable(value, "spec"),
        tickets: list(value, "tickets")
            .iter()
            .map(|t| SourceTicket { blocked_by: strings(t, "blockedBy"), id: text(t, "id"), open: flag(t, "open"), reference: text(t, "reference"), title: text(t, "title") })
            .collect(),
        vibe_spend_usd: number(value, "vibe_spend_usd"),
    }
}

#[test]
fn flow_corpus() {
    let source = match std::env::var("CRITIQUE_FLOW_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/flow-state.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 100);
    for (index, case) in cases.iter().enumerate() {
        let expected = field(case, "expected");
        let state = flow_state(&sources(field(case, "sources")));
        assert_eq!(json::compact(&state_json(&state)), json::compact(field(expected, "state")), "case {index}: state differs");
        assert_eq!(json::compact(&next_step(&state)), json::compact(field(expected, "next")), "case {index}: next step differs");
        let parsed = match parse_flow_state(&text(case, "text")) {
            Ok(read) => json::object(vec![("ok", state_json(&read)), ("next", next_step(&read))]),
            Err(message) => json::object(vec![("error", Value::Str(message))]),
        };
        assert_eq!(json::compact(&parsed), json::compact(field(expected, "parsed")), "case {index}: read-back differs");
        assert_eq!(flow_state_file_name(&text(case, "flow")), text(expected, "file"), "case {index}: file name differs");
    }
}
