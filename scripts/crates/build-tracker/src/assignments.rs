//! `set`'s `<ticket>=<status>[:note]` arguments and `refresh`'s landed tickets.

use crate::json::{Object, Value};
use crate::state::{raw_tickets, BuildTrackerState, Failure, Status};
use crate::text::{parse_number, trim};

/// One `<ticket>=<status>[:note]` argument of `set`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub ticket: u64,
    pub status: Status,
    /// Absent keeps the ticket's note; an empty note removes it.
    pub note: Option<String>,
}

/// `^#?([1-9][0-9]*)=([^:]*)(?::(.*))?$`: the ticket, the status text and the note after the first colon.
fn split_assignment(text: &str) -> Option<(u64, &str, Option<&str>)> {
    let bare = text.strip_prefix('#').unwrap_or(text);
    let (ticket, rest) = bare.split_once('=')?;
    if ticket.starts_with('0') || ticket.is_empty() || !ticket.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (status, note) = match rest.split_once(':') {
        Some((status, note)) => (status, Some(note)),
        None => (rest, None),
    };
    Some((parse_number(ticket)?, status, note))
}

fn known(state: &BuildTrackerState, ticket: u64) -> bool {
    state.tickets.iter().any(|candidate| candidate.number == ticket)
}

/// Reads every assignment against the state, so one bad ticket or status refuses them all before anything is written.
pub fn parse_assignments(texts: &[String], state: &BuildTrackerState) -> Result<Vec<Assignment>, Failure> {
    let mut assignments = Vec::new();
    for text in texts {
        let Some((ticket, status, note)) = split_assignment(text) else {
            return Err(Failure::new("invalid-assignment", format!("{text} is not <ticket>=<status>[:note]")));
        };
        if !known(state, ticket) {
            return Err(Failure::new("unknown-ticket", format!("ticket {ticket} is not in the state")));
        }
        let Some(status_value) = Status::parse(status) else {
            return Err(Failure::new("unknown-status", format!("{status} is not a status; use one of {}", Status::names())));
        };
        assignments.push(Assignment { note: note.map(str::to_string), status: status_value, ticket });
    }
    Ok(assignments)
}

/// Each ticket named as landed (`12`, `#12`, or a local ticket's `01`) as a `merged` assignment that keeps its note.
/// Every name is read against the state first, so one bad ticket refuses them all.
pub fn landed_assignments(texts: &[String], state: &BuildTrackerState) -> Result<Vec<Assignment>, Failure> {
    let mut assignments = Vec::new();
    for text in texts {
        let trimmed = trim(text);
        let digits = trimmed.strip_prefix('#').unwrap_or(trimmed).trim_start_matches('0');
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Failure::new("invalid-ticket", format!("{text} is not a ticket number")));
        }
        let Some(ticket) = parse_number(digits).filter(|ticket| known(state, *ticket)) else {
            return Err(Failure::new("unknown-ticket", format!("ticket {digits} is not one of the spec's tickets")));
        };
        assignments.push(Assignment { note: None, status: Status::Merged, ticket });
    }
    Ok(assignments)
}

/// The raw state with each assignment applied in order, every other key kept as it was.
pub fn apply_assignments(raw: &Object, assignments: &[Assignment]) -> Object {
    let mut tickets = raw_tickets(raw);
    for assignment in assignments {
        let key = assignment.ticket.to_string();
        let mut entry = tickets.get(&key).and_then(Value::as_object).cloned().unwrap_or_default();
        entry.set("status", Value::str(assignment.status.name()));
        match assignment.note.as_deref() {
            Some("") => entry.remove("note"),
            Some(note) => entry.set("note", Value::str(note)),
            None => {}
        }
        tickets.set(&key, Value::Obj(entry));
    }
    let mut applied = raw.clone();
    applied.set("tickets", Value::Obj(tickets));
    applied
}
