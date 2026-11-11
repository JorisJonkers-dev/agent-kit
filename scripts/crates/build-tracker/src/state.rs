//! The build tracker's state file: the one source the page is rendered from,
//! read from its parsed JSON (version 1). Keys this version does not name are
//! kept in the raw JSON, so a later version can add fields.

use crate::json::{compact, Object, Value};
use crate::spec::{parse_issue_reference, RepositoryName};

pub const STATE_VERSION: f64 = 1.0;

/// A failure the command line reports: its code and what went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

impl Failure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Failure {
        Failure { code, message: message.into() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Todo,
    Running,
    Review,
    Merged,
    Owner,
}

pub const STATUSES: [Status; 5] = [Status::Todo, Status::Running, Status::Review, Status::Merged, Status::Owner];

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Running => "running",
            Status::Review => "review",
            Status::Merged => "merged",
            Status::Owner => "owner",
        }
    }

    pub fn parse(text: &str) -> Option<Status> {
        STATUSES.into_iter().find(|status| status.name() == text)
    }

    pub fn names() -> String {
        STATUSES.map(Status::name).join(", ")
    }
}

/// A blocker outside the spec: `owner/repo#number`, or a local ticket's name as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalBlocker {
    pub reference: String,
    /// Its issue's state (`open` or `closed`), when the issue tracker reported one.
    pub state: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub number: u64,
    pub title: String,
    pub blocked_by: Vec<u64>,
    pub external: Vec<ExternalBlocker>,
    /// A local ticket's file under the repository root.
    pub file: Option<String>,
    pub status: Status,
    pub note: String,
    pub live: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Milestone {
    pub name: String,
    pub closes: u64,
}

/// The spec as the state names it: an issue, or a local spec's path under its repository root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpecReference {
    Github { repository: RepositoryName, number: u64 },
    Local { path: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildTrackerState {
    pub spec: SpecReference,
    pub title: String,
    pub subtitle: String,
    pub milestones: Vec<Milestone>,
    /// Every ticket, in issue-number order.
    pub tickets: Vec<Ticket>,
}

fn invalid(reason: impl Into<String>) -> Failure {
    Failure::new("unreadable-state", reason)
}

fn require_string(value: &Value, field: &str) -> Result<String, Failure> {
    value.as_str().map(str::to_string).ok_or_else(|| invalid(format!("{field} must be a string")))
}

fn issue_number(value: &Value) -> Option<u64> {
    match value {
        Value::Num(number) if number.fract() == 0.0 && *number > 0.0 && *number < 9_007_199_254_740_992.0 => Some(*number as u64),
        _ => None,
    }
}

/// Whether a raw ticket entry of the state file is merged.
pub fn raw_is_merged(entry: &Value) -> bool {
    entry.as_object().and_then(|object| object.get("status")).and_then(Value::as_str) == Some("merged")
}

/// The JSON object a state file holds, or why it is not one.
pub fn state_object(value: Value) -> Result<Object, Failure> {
    match value {
        Value::Obj(object) => Ok(object),
        _ => Err(invalid("the state is not a JSON object")),
    }
}

/// Reads parsed JSON as a version-1 state, or says what is wrong with it.
pub fn parse_state(raw: &Object) -> Result<BuildTrackerState, Failure> {
    if raw.get("version") != Some(&Value::Num(STATE_VERSION)) {
        let seen = raw.get("version").map_or("missing".to_string(), compact);
        return Err(invalid(format!("version must be 1, not {seen}")));
    }
    let spec = parse_spec(raw.get("spec"))?;
    let title = require_string(raw.get("title").unwrap_or(&Value::Null), "title")?;
    let subtitle = match raw.get("subtitle") {
        None => String::new(),
        Some(value) => require_string(value, "subtitle")?,
    };
    let Some(Value::Obj(table)) = raw.get("tickets") else {
        return Err(invalid("tickets must be an object keyed by issue number"));
    };
    let mut tickets = Vec::new();
    for (key, value) in table.entries() {
        tickets.push(parse_ticket(key, value)?);
    }
    for ticket in &tickets {
        if let Some(stray) = ticket.blocked_by.iter().find(|blocker| !tickets.iter().any(|other| other.number == **blocker)) {
            return Err(invalid(format!("ticket {} is blocked by {stray}, which is not in the state", ticket.number)));
        }
    }
    let milestones = read_milestones(raw.get("milestones"), &tickets)?;
    tickets.sort_by_key(|ticket| ticket.number);
    Ok(BuildTrackerState { milestones, spec, subtitle, tickets, title })
}

fn parse_spec(raw: Option<&Value>) -> Result<SpecReference, Failure> {
    if let Some(path) = raw.and_then(Value::as_object).and_then(|object| object.get("path")).and_then(Value::as_str).filter(|path| !path.is_empty()) {
        return Ok(SpecReference::Local { path: path.to_string() });
    }
    let reference = raw.and_then(Value::as_str).and_then(parse_issue_reference);
    match reference {
        Some(issue) => Ok(SpecReference::Github { number: issue.number, repository: issue.repository }),
        None => Err(invalid("spec must be a reference like owner/repo#123, or { \"path\": \"<file>\" } for a local spec")),
    }
}

fn parse_ticket(key: &str, value: &Value) -> Result<Ticket, Failure> {
    let place = format!("ticket {key}");
    let number = match key.parse::<u64>() {
        Ok(number) if !key.starts_with(['0', '+']) => number,
        _ => return Err(invalid(format!("{place}: a ticket key must be its issue number"))),
    };
    let Some(entry) = value.as_object() else {
        return Err(invalid(format!("{place} is not an object")));
    };
    let blocked_by = match entry.get("blocked_by") {
        Some(Value::Arr(items)) => items.iter().map(issue_number).collect::<Option<Vec<u64>>>(),
        _ => None,
    };
    let Some(blocked_by) = blocked_by else {
        return Err(invalid(format!("{place}: blocked_by must be a list of issue numbers")));
    };
    let Some(status) = entry.get("status").and_then(Value::as_str).and_then(Status::parse) else {
        return Err(invalid(format!("{place}: status must be one of {}", Status::names())));
    };
    if entry.get("live").is_some_and(|live| !matches!(live, Value::Bool(_))) {
        return Err(invalid(format!("{place}: live must be true or false")));
    }
    let external = parse_external(entry.get("external"), &place)?;
    let optional = |name: &str| entry.get(name).map(|value| require_string(value, &format!("{place}: {name}"))).transpose();
    let file = optional("file")?;
    let note = optional("note")?.unwrap_or_default();
    let title = require_string(entry.get("title").unwrap_or(&Value::Null), &format!("{place}: title"))?;
    Ok(Ticket { blocked_by, external, file, live: entry.get("live") == Some(&Value::Bool(true)), note, number, status, title })
}

/// Each external blocker is `{ "ref": ..., "state"?: "open" | "closed" }`, or just its ref as a string.
fn parse_external(raw: Option<&Value>, place: &str) -> Result<Vec<ExternalBlocker>, Failure> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let reason = format!("{place}: external must be a list of blocker references");
    let Value::Arr(items) = raw else {
        return Err(invalid(reason));
    };
    let mut blockers = Vec::new();
    for item in items {
        let reference = match item {
            Value::Str(text) if !text.is_empty() => {
                blockers.push(ExternalBlocker { reference: text.clone(), state: None });
                continue;
            }
            Value::Obj(object) => object.get("ref").and_then(Value::as_str).filter(|text| !text.is_empty()),
            _ => None,
        };
        let (Some(reference), Value::Obj(object)) = (reference, item) else {
            return Err(invalid(reason));
        };
        let state = match object.get("state") {
            None => None,
            Some(Value::Str(state)) if state == "open" || state == "closed" => Some(state.clone()),
            Some(_) => return Err(invalid(format!("{place}: an external blocker's state must be open or closed"))),
        };
        blockers.push(ExternalBlocker { reference: reference.to_string(), state });
    }
    Ok(blockers)
}

fn read_milestones(raw: Option<&Value>, tickets: &[Ticket]) -> Result<Vec<Milestone>, Failure> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let Value::Arr(items) = raw else {
        return Err(invalid("milestones must be a list"));
    };
    let mut milestones = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let place = format!("milestone {}", index + 1);
        let Some(object) = item.as_object() else {
            return Err(invalid(format!("{place} is not an object")));
        };
        let name = require_string(object.get("name").unwrap_or(&Value::Null), &format!("{place}: name"))?;
        let closes = object.get("closes").and_then(issue_number).filter(|closes| tickets.iter().any(|ticket| ticket.number == *closes));
        let Some(closes) = closes else {
            return Err(invalid(format!("{place}: closes must name a ticket in the state")));
        };
        milestones.push(Milestone { closes, name });
    }
    Ok(milestones)
}

/// The `tickets` object of a raw state, which `parse_state` has checked is one.
pub fn raw_tickets(raw: &Object) -> Object {
    raw.get("tickets").and_then(Value::as_object).cloned().expect("a checked state has a tickets object")
}
