//! A new state from a spec's tickets, and a state brought up to date with them.

use crate::json::{object, Object, Value};
use crate::spec::{spec_text, ResolvedSpec};
use crate::state::{raw_is_merged, raw_tickets, ExternalBlocker, Failure, Milestone, STATE_VERSION};
use crate::text::{is_space, parse_number, trim};

/// One ticket of a spec as its issue tracker holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecTicket {
    pub number: u64,
    pub title: String,
    pub closed: bool,
    /// Its blockers among the spec's tickets.
    pub blocked_by: Vec<u64>,
    /// Its blockers outside the spec.
    pub external: Vec<ExternalBlocker>,
    /// A local ticket's file, under the repository root.
    pub file: Option<String>,
}

/// A spec's title and tickets as its issue tracker holds them now, read once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueTrackerSnapshot {
    pub title: String,
    pub tickets: Vec<SpecTicket>,
}

/// `^(.*\S)\s*=\s*#?([1-9]\d*)$`: the name before the last `=`, and the ticket after it.
fn split_milestone(text: &str) -> Option<(&str, u64)> {
    let (head, tail) = text.rsplit_once('=')?;
    let name = head.trim_end_matches(is_space);
    let digits = tail.trim_start_matches(is_space);
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    let valid = !digits.starts_with('0') && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
    if name.is_empty() || !valid {
        return None;
    }
    Some((name, parse_number(digits)?))
}

/// Reads each `<name>=<ticket>` argument of `init` in order; every ticket must be one of the spec's.
pub fn parse_milestones(texts: &[String], snapshot: &IssueTrackerSnapshot) -> Result<Vec<Milestone>, Failure> {
    let mut milestones = Vec::new();
    for text in texts {
        let Some((name, closes)) = split_milestone(trim(text)) else {
            return Err(Failure::new("invalid-milestone", format!("{text} is not <name>=<ticket>")));
        };
        if !snapshot.tickets.iter().any(|ticket| ticket.number == closes) {
            return Err(Failure::new("unknown-ticket", format!("milestone {name} closes ticket {closes}, which is not one of the spec's tickets")));
        }
        milestones.push(Milestone { closes, name: name.to_string() });
    }
    Ok(milestones)
}

fn numbers(values: &[u64]) -> Value {
    Value::Arr(values.iter().map(|number| Value::Num(*number as f64)).collect())
}

/// The entry with the fields the issue tracker owns replaced; an empty external list or a missing file leaves no key.
fn issue_tracker_fields(entry: &Object, ticket: &SpecTicket) -> Object {
    // Title and blockers lead a new entry; the entry's own keys follow.
    let mut result = Object::new();
    result.set("title", Value::str(&ticket.title));
    result.set("blocked_by", Value::Arr(Vec::new()));
    for (key, value) in entry.entries() {
        result.set(key, value.clone());
    }
    result.set("title", Value::str(&ticket.title));
    result.set("blocked_by", numbers(&ticket.blocked_by));
    result.remove("external");
    result.remove("file");
    if !ticket.external.is_empty() {
        let blockers = ticket.external.iter().map(|blocker| {
            let mut fields = vec![("ref", Value::str(&blocker.reference))];
            fields.extend(blocker.state.iter().map(|state| ("state", Value::str(state))));
            object(fields)
        });
        result.set("external", Value::Arr(blockers.collect()));
    }
    if let Some(file) = &ticket.file {
        result.set("file", Value::str(file));
    }
    result
}

fn with_status(status: &str) -> Object {
    let mut entry = Object::new();
    entry.set("status", Value::str(status));
    entry
}

/// A new state for the spec: every open ticket `todo`, every closed one `merged`.
pub fn seed_state(spec: &ResolvedSpec, snapshot: &IssueTrackerSnapshot, milestones: &[Milestone]) -> Object {
    let mut tickets = Object::new();
    for ticket in &snapshot.tickets {
        let entry = issue_tracker_fields(&with_status(if ticket.closed { "merged" } else { "todo" }), ticket);
        tickets.set(&ticket.number.to_string(), Value::Obj(entry));
    }
    let mut raw = Object::new();
    if !milestones.is_empty() {
        let items = milestones.iter().map(|milestone| object(vec![("closes", Value::Num(milestone.closes as f64)), ("name", Value::str(&milestone.name))]));
        raw.set("milestones", Value::Arr(items.collect()));
    }
    raw.set(
        "spec",
        match spec {
            ResolvedSpec::Github { .. } => Value::Str(spec_text(spec)),
            ResolvedSpec::Local { path, .. } => object(vec![("path", Value::str(path))]),
        },
    );
    raw.set("tickets", Value::Obj(tickets));
    raw.set("title", Value::str(&snapshot.title));
    raw.set("version", Value::Num(STATE_VERSION));
    raw
}

pub struct SyncOutcome {
    pub raw: Object,
    /// Tickets the state did not have.
    pub added: Vec<u64>,
    /// Tickets whose issue closed since the state last saw them open.
    pub merged: Vec<u64>,
}

/// The state brought up to date with the issue tracker. New tickets are added; a known ticket gets its title,
/// blockers and file from the issue tracker, and turns `merged` once its issue is closed. Every status, note and
/// other key set on an open ticket stays, and so do the milestones, the title and tickets no longer listed.
pub fn sync_state(raw: &Object, snapshot: &IssueTrackerSnapshot) -> SyncOutcome {
    let mut tickets = raw_tickets(raw);
    let (mut added, mut merged) = (Vec::new(), Vec::new());
    for ticket in &snapshot.tickets {
        let key = ticket.number.to_string();
        let known = tickets.get(&key).and_then(Value::as_object).cloned();
        match &known {
            None => added.push(ticket.number),
            Some(known) if ticket.closed && !raw_is_merged(&Value::Obj(known.clone())) => merged.push(ticket.number),
            Some(_) => {}
        }
        let status = match (&known, ticket.closed) {
            (_, true) => Value::str("merged"),
            (Some(known), false) => known.get("status").cloned().unwrap_or_else(|| Value::str("todo")),
            (None, false) => Value::str("todo"),
        };
        let mut entry = known.unwrap_or_default();
        entry.set("status", status);
        tickets.set(&key, Value::Obj(issue_tracker_fields(&entry, ticket)));
    }
    let mut synced = raw.clone();
    synced.set("tickets", Value::Obj(tickets));
    SyncOutcome { added, merged, raw: synced }
}
