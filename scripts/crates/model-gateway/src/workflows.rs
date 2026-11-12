//! A workflow's life: the name every delegation of one piece of work carries,
//! and the moment it started and ended. Caps and grants count against it, so a
//! workflow that nobody ends must not stay alive for ever.

use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::timestamps::{self, WORKFLOW_TTL_MS};

/// The version the workflows file is written as.
pub const WORKFLOWS_VERSION: f64 = 1.0;

/// One workflow, as the file holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowRecord {
    pub name: String,
    /// When it was first named, as an ISO timestamp.
    pub started_at: String,
    /// When someone ended it, as an ISO timestamp; absent while it runs.
    pub ended_at: Option<String>,
    /// Where the end sits in the record's key order: a record read from the
    /// file writes it first, one `workflow end` made writes it last.
    pub ended_last: bool,
}

impl WorkflowRecord {
    /// The record as the file writes it: `{ended_at, name, started_at}` when it
    /// has an end, else `{name, started_at}`.
    pub fn to_json(&self) -> Value {
        let mut object = Object::new();
        if self.ended_last {
            object.set("name", Value::str(&self.name));
            object.set("started_at", Value::str(&self.started_at));
            if let Some(ended_at) = &self.ended_at {
                object.set("ended_at", Value::str(ended_at));
            }
        } else {
            if let Some(ended_at) = &self.ended_at {
                object.set("ended_at", Value::str(ended_at));
            }
            object.set("name", Value::str(&self.name));
            object.set("started_at", Value::str(&self.started_at));
        }
        Value::Obj(object)
    }
}

/// What a change to the workflows leaves behind: the workflows to write, and the one it touched.
pub struct Change {
    pub workflows: Vec<WorkflowRecord>,
    pub workflow: WorkflowRecord,
}

/// The workflow name `value` names, refused when it is missing or is not a name
/// a workflow may carry. A name reaches the shell command the gateway suggests
/// and the paths a run keeps, so it is held to letters, digits and `._:/-`.
pub fn require_name(value: Option<&Value>, missing: &str) -> Result<String, GatewayError> {
    let Some(Value::Str(name)) = value else {
        return Err(GatewayError::invalid_input(missing));
    };
    if name.is_empty() {
        return Err(GatewayError::invalid_input(missing));
    }
    if !is_name(name) {
        return Err(GatewayError::invalid_input(format!(
            "workflow {} is not a workflow name: a letter or digit, then letters, digits and . _ : / - only, at most 80 characters",
            json::compact(&Value::str(name))
        )));
    }
    Ok(name.clone())
}

/// `^[A-Za-z0-9][A-Za-z0-9._:/-]{0,79}$`.
fn is_name(name: &str) -> bool {
    // The caller has refused the empty name, so the first character is there.
    let chars: Vec<char> = name.chars().collect();
    chars.len() <= 80
        && chars[0].is_ascii_alphanumeric()
        && chars[1..].iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'))
}

/// The workflows a file holds. A file that is not JSON, of another version, or
/// whose entries are not workflows is one error naming what is wrong, rather
/// than a surprise when a run joins a workflow that is not there.
pub fn parse_file(text: &str) -> Result<Vec<WorkflowRecord>, String> {
    let raw = json::parse(text).map_err(|message| format!("not JSON: {message}"))?;
    let Some(object) = raw.as_object() else {
        return Err("the workflows file must be a JSON object".to_string());
    };
    if object.get("version") != Some(&Value::Num(WORKFLOWS_VERSION)) {
        return Err(format!("version must be {}", json::number_text(WORKFLOWS_VERSION)));
    }
    let Some(Value::Arr(items)) = object.get("workflows") else {
        return Err("workflows must be a list".to_string());
    };
    items.iter().enumerate().map(|(index, value)| parse_record(value, index)).collect()
}

fn parse_record(raw: &Value, index: usize) -> Result<WorkflowRecord, String> {
    let where_ = format!("workflows[{index}]");
    let Some(object) = raw.as_object() else {
        return Err(format!("{where_} must be an object"));
    };
    let name = require_string(object.get("name"), &format!("{where_}.name"))?;
    let started_at = require_timestamp(object.get("started_at"), &format!("{where_}.started_at"))?;
    let Some(ended) = object.get("ended_at") else {
        return Ok(WorkflowRecord { name, started_at, ended_at: None, ended_last: false });
    };
    let ended_at = require_timestamp(Some(ended), &format!("{where_}.ended_at"))?;
    Ok(WorkflowRecord { name, started_at, ended_at: Some(ended_at), ended_last: false })
}

fn require_string(value: Option<&Value>, where_: &str) -> Result<String, String> {
    match value {
        Some(Value::Str(text)) if !text.is_empty() => Ok(text.clone()),
        _ => Err(format!("{where_} must be a non-empty string")),
    }
}

fn require_timestamp(value: Option<&Value>, where_: &str) -> Result<String, String> {
    match value {
        Some(Value::Str(text)) if timestamps::parse(text).is_some() => Ok(text.clone()),
        _ => Err(format!("{where_} must be an ISO timestamp")),
    }
}

/// The moment a workflow's 24 hours run out.
fn expiry_of(record: &WorkflowRecord) -> String {
    timestamps::to_iso(timestamps::parse(&record.started_at).unwrap_or_default() + WORKFLOW_TTL_MS)
}

/// Whether a workflow is closed to runs: someone ended it, or its 24 hours ran out.
pub fn ended(record: &WorkflowRecord, now_ms: i64) -> bool {
    if record.ended_at.is_some() {
        return true;
    }
    now_ms - timestamps::parse(&record.started_at).unwrap_or_default() >= WORKFLOW_TTL_MS
}

/// One workflow as a caller reads it, with the 24-hour end worked out from the clock.
pub fn view(record: &WorkflowRecord, now_ms: i64) -> Value {
    let ended_at = match &record.ended_at {
        Some(ended_at) => Some(ended_at.clone()),
        None if ended(record, now_ms) => Some(expiry_of(record)),
        None => None,
    };
    let state = if ended_at.is_none() { "running" } else { "ended" };
    let mut object = Object::new();
    object.set("ended_at", ended_at.as_deref().map_or(Value::Null, Value::str));
    object.set("name", Value::str(&record.name));
    object.set("started_at", Value::str(&record.started_at));
    object.set("state", Value::str(state));
    Value::Obj(object)
}

/// The workflow `name` names, refused when it ended: its caps and grants lapsed
/// with it, so a run may not quietly revive it.
pub fn require_open(records: &[WorkflowRecord], name: &str, now_ms: i64) -> Result<Option<WorkflowRecord>, GatewayError> {
    let Some(existing) = records.iter().find(|record| record.name == name) else {
        return Ok(None);
    };
    if !ended(existing, now_ms) {
        return Ok(Some(existing.clone()));
    }
    let ended_at = existing.ended_at.clone().unwrap_or_else(|| expiry_of(existing));
    Err(GatewayError::new(
        "workflow-ended",
        format!("workflow {name} ended at {ended_at}: a run may not join it, so start a new workflow"),
    ))
}

/// The workflows with `name` started at `now` if it is new, or as they are when
/// it is already running. A workflow that ended is refused.
pub fn start(records: &[WorkflowRecord], name: &str, now_ms: i64) -> Result<Change, GatewayError> {
    match require_open(records, name, now_ms)? {
        Some(existing) => Ok(Change { workflow: existing, workflows: records.to_vec() }),
        None => {
            let started = WorkflowRecord { name: name.to_string(), started_at: timestamps::to_iso(now_ms), ended_at: None, ended_last: false };
            let mut workflows = records.to_vec();
            workflows.push(started.clone());
            Ok(Change { workflow: started, workflows })
        }
    }
}

/// The workflows with `name` ended at `now`. One that already ended keeps the
/// moment it ended; one this machine never started is an error, so a typo does
/// not pass for a workflow that ran.
pub fn end(records: &[WorkflowRecord], name: &str, now_ms: i64) -> Result<Change, GatewayError> {
    let Some(existing) = records.iter().find(|record| record.name == name) else {
        return Err(GatewayError::new("unknown-workflow", format!("no workflow named {name} has started on this machine")));
    };
    if ended(existing, now_ms) {
        return Ok(Change { workflow: existing.clone(), workflows: records.to_vec() });
    }
    let ended_record = WorkflowRecord {
        name: existing.name.clone(),
        started_at: existing.started_at.clone(),
        ended_at: Some(timestamps::to_iso(now_ms)),
        ended_last: true,
    };
    let workflows = records
        .iter()
        .map(|record| if record.name == name { ended_record.clone() } else { record.clone() })
        .collect();
    Ok(Change { workflow: ended_record, workflows })
}
