//! The ticket checker over generated ticket files, ticket graphs and specs, compared with what the node runner's
//! `domain/tickets.ts` makes of them.

use crate::tickets::{check_json, check_tickets, local_ticket_open, local_tickets, size_report, summarise_check, LocalTicketFile, Ticket, TicketSize};
use model_gateway::json::{self, Value};

/// The keys of every object sorted, so the comparison is of values; the command's own key order is the goldens' job.
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

fn text(value: &Value, key: &str) -> String {
    value.as_object().and_then(|object| object.get(key)).and_then(Value::as_str).expect("a string field").to_string()
}

fn list(value: &Value, key: &str) -> Vec<Value> {
    match value.as_object().and_then(|object| object.get(key)) {
        Some(Value::Arr(items)) => items.clone(),
        _ => panic!("{key} is not a list"),
    }
}

fn ticket_json(ticket: &Ticket) -> Value {
    json::object(vec![
        ("blockedBy", Value::Arr(ticket.blocked_by.iter().map(|id| Value::str(id)).collect())),
        ("id", Value::str(&ticket.id)),
        ("reference", Value::str(&ticket.reference)),
        ("text", Value::str(&ticket.text)),
        ("title", Value::str(&ticket.title)),
    ])
}

fn replay(index: usize, case: &Value) {
    let object = case.as_object().expect("case object");
    let expected = object.get("expected").expect("expected");
    let tickets: Vec<Ticket> = match object.get("files") {
        Some(Value::Arr(files)) => {
            let files: Vec<LocalTicketFile> = files.iter().map(|file| LocalTicketFile { reference: text(file, "reference"), text: text(file, "text") }).collect();
            let tickets = local_tickets(&files);
            let actual = Value::Arr(tickets.iter().map(ticket_json).collect());
            let wanted = expected.as_object().and_then(|expected| expected.get("local")).expect("local");
            assert_eq!(json::compact(&canonical(&actual)), json::compact(&canonical(wanted)), "case {index}: local tickets differ");
            tickets
        }
        _ => list(case, "tickets")
            .iter()
            .map(|ticket| Ticket {
                blocked_by: list(ticket, "blockedBy").iter().map(|id| id.as_str().expect("an id").to_string()).collect(),
                id: text(ticket, "id"),
                reference: text(ticket, "reference"),
                text: text(ticket, "text"),
                title: text(ticket, "title"),
            })
            .collect(),
    };
    let check = check_tickets(&text(case, "spec"), &tickets);
    let size = object.get("size").expect("size");
    let number = |key: &str| match size.as_object().and_then(|size| size.get(key)) {
        Some(Value::Num(number)) => *number,
        _ => panic!("size has no {key}"),
    };
    let limits = TicketSize { max_criteria: number("max_criteria"), max_modules: number("max_modules") };
    let actual = json::object(vec![
        ("check", json::object(check_json(&check))),
        ("open", Value::Arr(tickets.iter().map(|ticket| Value::Bool(local_ticket_open(&ticket.text))).collect())),
        ("size", size_report(&check, &limits)),
        ("summary", summarise_check(&check)),
    ]);
    let mut wanted = expected.as_object().expect("expected object").clone();
    wanted.remove("local");
    assert_eq!(json::compact(&canonical(&actual)), json::compact(&canonical(&Value::Obj(wanted))), "case {index} differs\n  case: {}", json::compact(case));
}

/// Replays the corpus; `CRITIQUE_TICKETS_CORPUS` names a bigger one.
#[test]
fn tickets_corpus() {
    let source = match std::env::var("CRITIQUE_TICKETS_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/tickets-check.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 100);
    for (index, case) in cases.iter().enumerate() {
        replay(index, case);
    }
}
