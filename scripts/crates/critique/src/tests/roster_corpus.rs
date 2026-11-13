//! The depth choice and the roster resolution over generated routing files, requests and readiness, compared with what
//! the node runner makes of them (tests/gen-roster-corpus.mjs), as the JSON the manifest freezes, so the order of keys
//! counts. `CRITIQUE_ROSTER_CORPUS` names a bigger corpus to replay.

use crate::depth::{choose_depth, DepthEvidence, DepthSignalsConfig};
use crate::error::CritiqueError;
use crate::prepared_json::{critic_value, depth_choice_value, dropped_value, identity_value, limits_value};
use crate::roster_resolve::{
    candidate_clients, resolve_roster, resolve_work_roster, work_candidate_clients, work_grant_client, Availability, Roster, RosterRequest, WorkGrant, WorkRosterRequest,
};
use crate::routing::parse_routing;
use model_gateway::json::{self, Value};
use std::collections::HashMap;

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

fn maybe_count(value: &Value, key: &str) -> Option<usize> {
    field(value, key).map(|_| number(value, key) as usize)
}

fn outcome(result: Result<Value, CritiqueError>) -> Value {
    match result {
        Ok(ok) => json::object(vec![("ok", ok)]),
        Err(error) => json::object(vec![("error", json::object(vec![("code", Value::str(&error.code)), ("message", Value::str(&error.message))]))]),
    }
}

fn names(candidates: Vec<(String, &crate::routing::CriticClient)>) -> Value {
    Value::Arr(candidates.iter().map(|(name, _)| Value::str(name)).collect())
}

fn roster_value(roster: &Roster) -> Value {
    json::object(vec![
        ("critics", Value::Arr(roster.critics.iter().map(critic_value).collect())),
        ("depth", Value::str(&roster.depth)),
        ("distinct_families", Value::Bool(roster.distinct_families)),
        ("dropped", Value::Arr(roster.dropped.iter().map(dropped_value).collect())),
    ])
}

fn replay_depth(input: &Value) -> Value {
    let config = field(input, "config").expect("config");
    let thresholds = match field(config, "thresholds") {
        Some(Value::Obj(object)) => object.entries().into_iter().map(|(name, value)| (name.to_string(), if let Value::Num(number) = value { *number } else { panic!("threshold") })).collect(),
        _ => Vec::new(),
    };
    let config = DepthSignalsConfig { thresholds, risk_terms: strings(config, "risk_terms"), labels: strings(config, "labels") };
    let given = field(input, "evidence").expect("evidence");
    let labels = strings(given, "labels");
    let change = field(given, "change").map(|change| (number(change, "changed_lines"), number(change, "risk_path_hits")));
    let source = text(given, "text");
    let evidence = DepthEvidence {
        text: &source,
        labels: &labels,
        change,
        tickets: maybe_count(given, "tickets"),
        adrs: maybe_count(given, "adrs"),
        source_lines: maybe_count(given, "sourceLines"),
        context_maps: maybe_count(given, "contextMaps"),
        plan_steps: maybe_count(given, "planSteps"),
        stored_data_steps: maybe_count(given, "storedDataSteps"),
    };
    let explicit = optional_text(input, "explicit");
    let (depth, choice) = choose_depth(&text(input, "stage"), &config, &evidence, explicit.as_deref());
    Value::Str(json::compact(&json::object(vec![("choice", depth_choice_value(&choice)), ("depth", Value::Str(depth))])))
}

fn replay_roster(input: &Value) -> Value {
    let parsed = parse_routing(&json::parse(&text(input, "routing")).expect("routing json"));
    let routing = match parsed {
        Ok(routing) => routing,
        Err(error) => return json::object(vec![("error", json::object(vec![("code", Value::str("unexpected")), ("message", Value::str(&error.0))]))]),
    };
    let given = field(input, "request").expect("request");
    let allowed = strings(given, "allowedFamilies");
    let stage = text(given, "stage");
    let orchestrator = text(given, "orchestratorFamily");
    let request = RosterRequest { stage: &stage, depth: &text(given, "depth"), allowed_families: &allowed, orchestrator_family: &orchestrator, adr: field(given, "adr").is_some() };
    let mut readiness: HashMap<String, Availability> = HashMap::new();
    if let Some(Value::Obj(object)) = field(input, "readiness") {
        for (name, entry) in object.entries() {
            let ready = field(entry, "ready") == Some(&Value::Bool(true));
            readiness.insert(name.to_string(), Availability { ready, detail: optional_text(entry, "detail").unwrap_or_default(), reason: optional_text(entry, "reason").map(|_| "quota") });
        }
    }
    let grant_input = field(input, "grant").expect("grant");
    let grant = if field(grant_input, "live") == Some(&Value::Bool(true)) { WorkGrant::Live { workflow: text(grant_input, "workflow") } } else { WorkGrant::Missing { detail: text(grant_input, "detail") } };
    let work = WorkRosterRequest { stage: &stage, orchestrator_family: &orchestrator, grant: &grant };
    let roster = resolve_roster(&routing, &request, &readiness);
    let work_roster = resolve_work_roster(&routing, &work, &readiness);
    let ok = json::object(vec![
        ("candidates", outcome(candidate_clients(&routing, &request).map(names))),
        ("roster", outcome(roster.clone().map(|roster| Value::Str(json::compact(&roster_value(&roster)))))),
        ("identities", outcome(roster.map(|roster| Value::Arr(roster.critics.iter().map(identity_value).collect())))),
        ("workCandidates", outcome(work_candidate_clients(&routing, &work).map(names))),
        ("workGrant", outcome(work_grant_client(&routing, &stage).map(Value::Str))),
        ("workRoster", outcome(work_roster.clone().map(|roster| Value::Str(json::compact(&roster_value(&roster)))))),
        ("workIdentities", outcome(work_roster.map(|roster| Value::Arr(roster.critics.iter().map(identity_value).collect())))),
        ("limits", Value::Str(json::compact(&limits_value(&routing.limits)))),
    ]);
    json::object(vec![("ok", ok)])
}

#[test]
fn roster_corpus() {
    let source = match std::env::var("CRITIQUE_ROSTER_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/roster.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 200);
    for (index, case) in cases.iter().enumerate() {
        let input = field(case, "input").expect("input");
        let expected = field(case, "expected").expect("expected");
        let actual = if text(case, "kind") == "depth" { replay_depth(input) } else { replay_roster(input) };
        assert_eq!(json::compact(&actual), json::compact(expected), "case {index} differs\n  case: {}", json::compact(case));
    }
}
