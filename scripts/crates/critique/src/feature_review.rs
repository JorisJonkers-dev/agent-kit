//! A feature review's gate, recorded for the head it reviewed so a script can read it without any model call. The node
//! runner's `domain/feature-review.ts`.

use crate::error::ShapeError;
use crate::fields::Fields;
use crate::findings::SEVERITIES;
use model_gateway::json::{self, Value};

/// One accepted feature finding and the follow-up ticket it became.
#[derive(Clone, Debug, PartialEq)]
pub struct FollowUp {
    pub finding_id: String,
    pub severity: String,
    /// The ticket as its tracker numbers it: `#301`, or the `NN` of a local ticket file.
    pub ticket: String,
    /// Where it lives: `host/owner/repo#<number>` for an issue, its repository path for a file.
    pub reference: String,
    /// Whether the ticket was open when the run was recorded.
    pub open: bool,
}

/// A critical follow-up, from this run or an earlier one of the same spec, still open: what holds the gate shut.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenCriticalFollowUp {
    pub finding_id: String,
    /// The run that accepted the finding.
    pub run_id: String,
    pub ticket: String,
    pub reference: String,
}

/// A feature review's gate, for the head it reviewed.
#[derive(Clone, Debug, PartialEq)]
pub struct GateRecord {
    pub run_id: String,
    pub recorded_at: String,
    pub repository: String,
    /// The spec's reference, as the manifest's artifact names it.
    pub spec: String,
    pub base_commit: String,
    /// The commit the gate holds for; a new head needs a new run.
    pub head_commit: String,
    /// `pass` or `fail`.
    pub gate: String,
    pub critical_open: f64,
    /// The report's first line: `GATE: PASS` or `GATE: FAIL (<n> critical)`.
    pub line: String,
    /// Every accepted finding of this run and its follow-up ticket.
    pub follow_ups: Vec<FollowUp>,
    pub open_critical: Vec<OpenCriticalFollowUp>,
}

/// Reads one gate record, as `gate.json` or a line of the gate index holds it; the message is the node runner's.
pub fn parse_gate_record(text: &str) -> Result<GateRecord, String> {
    let value = json::parse(text)?;
    let file = Fields::of(&value, "").map_err(|error| error.0)?;
    read_gate_record(&file).map_err(|error| error.0)
}

fn read_gate_record(file: &Fields<'_>) -> Result<GateRecord, ShapeError> {
    let base_commit = file.string("base_commit")?;
    let critical_open = file.number("critical_open")?;
    let follow_ups = file.list("follow_ups", |follow_up| {
        Ok(FollowUp {
            finding_id: follow_up.string("finding_id")?,
            open: follow_up.boolean("open")?,
            reference: follow_up.string("reference")?,
            severity: follow_up.one_of("severity", &SEVERITIES)?,
            ticket: follow_up.string("ticket")?,
        })
    })?;
    let gate = file.one_of("gate", &["pass", "fail"])?;
    let head_commit = file.string("head_commit")?;
    let line = file.string("line")?;
    let open_critical = file.list("open_critical", |follow_up| {
        Ok(OpenCriticalFollowUp {
            finding_id: follow_up.string("finding_id")?,
            reference: follow_up.string("reference")?,
            run_id: follow_up.string("run_id")?,
            ticket: follow_up.string("ticket")?,
        })
    })?;
    let recorded_at = file.string("recorded_at")?;
    let repository = file.string("repository")?;
    let run_id = file.string("run_id")?;
    let spec = file.string("spec")?;
    file.literal_str("stage", "feature-review")?;
    file.literal("version", 1.0)?;
    Ok(GateRecord { run_id, recorded_at, repository, spec, base_commit, head_commit, gate, critical_open, line, follow_ups, open_critical })
}

/// The report's first line: `GATE: PASS`, or `GATE: FAIL (<n> critical)`.
pub fn gate_line(critical_open: usize) -> String {
    if critical_open == 0 { "GATE: PASS".to_string() } else { format!("GATE: FAIL ({critical_open} critical)") }
}

/// The critical follow-ups that hold the gate: this run's, then every earlier run's for the same spec, each counted once
/// by ticket and only while its ticket is open.
pub fn open_critical_follow_ups(run_id: &str, current: &[FollowUp], earlier: &[GateRecord], is_open: impl Fn(&str) -> bool) -> Vec<OpenCriticalFollowUp> {
    let critical = current
        .iter()
        .map(|follow_up| (follow_up, run_id))
        .chain(earlier.iter().flat_map(|record| record.follow_ups.iter().map(|follow_up| (follow_up, record.run_id.as_str()))))
        .filter(|(follow_up, _)| follow_up.severity == "critical");
    let mut seen: Vec<&str> = Vec::new();
    let mut open = Vec::new();
    for (follow_up, from) in critical {
        if seen.contains(&follow_up.reference.as_str()) {
            continue;
        }
        seen.push(&follow_up.reference);
        if is_open(&follow_up.reference) {
            open.push(OpenCriticalFollowUp {
                finding_id: follow_up.finding_id.clone(),
                reference: follow_up.reference.clone(),
                run_id: from.to_string(),
                ticket: follow_up.ticket.clone(),
            });
        }
    }
    open
}

impl GateRecord {
    /// The record as `gate.json` and the gate index hold it: `JSON.stringify` order.
    pub fn to_json(&self) -> Value {
        let follow_ups = self
            .follow_ups
            .iter()
            .map(|follow_up| {
                json::object(vec![
                    ("finding_id", Value::str(&follow_up.finding_id)),
                    ("open", Value::Bool(follow_up.open)),
                    ("reference", Value::str(&follow_up.reference)),
                    ("severity", Value::str(&follow_up.severity)),
                    ("ticket", Value::str(&follow_up.ticket)),
                ])
            })
            .collect();
        let open = self
            .open_critical
            .iter()
            .map(|follow_up| {
                json::object(vec![
                    ("finding_id", Value::str(&follow_up.finding_id)),
                    ("reference", Value::str(&follow_up.reference)),
                    ("run_id", Value::str(&follow_up.run_id)),
                    ("ticket", Value::str(&follow_up.ticket)),
                ])
            })
            .collect();
        json::object(vec![
            ("base_commit", Value::str(&self.base_commit)),
            ("critical_open", Value::Num(self.critical_open)),
            ("follow_ups", Value::Arr(follow_ups)),
            ("gate", Value::str(&self.gate)),
            ("head_commit", Value::str(&self.head_commit)),
            ("line", Value::str(&self.line)),
            ("open_critical", Value::Arr(open)),
            ("recorded_at", Value::str(&self.recorded_at)),
            ("repository", Value::str(&self.repository)),
            ("run_id", Value::str(&self.run_id)),
            ("spec", Value::str(&self.spec)),
            ("stage", Value::str("feature-review")),
            ("version", Value::Num(1.0)),
        ])
    }
}
