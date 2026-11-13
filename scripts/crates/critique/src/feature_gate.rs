//! Computes a feature review's gate when its verdicts are recorded. Each accepted finding must name a follow-up that is
//! a ticket of the spec. The gate fails while any critical follow-up, this run's or an earlier run's of the same spec,
//! is still open on the tracker, so a re-run on a new head keeps failing until those tickets are closed, whether or not
//! its critics raise the finding again. The node runner's `application/feature-gate.ts`.

use crate::decode::decoded;
use crate::error::CritiqueError;
use crate::feature_review::{gate_line, open_critical_follow_ups, parse_gate_record, FollowUp, GateRecord};
use crate::manifest::{spec_reference, RunManifest};
use crate::paths::join;
use crate::repository::Resolved;
use crate::runtime::Runtime;
use crate::tracker::parse_issue_reference;
use crate::tracker_resolver::{resolve_tracker, TrackedTicket, TrackerResolution, TicketsRead};
use crate::verdicts::AdjudicatedFinding;
use model_gateway::json::{self, number_text, Value};
use model_gateway::text::dirname;
use model_gateway::timestamps::to_iso;

/// The gate a feature-review run holds, in the run directory.
pub fn gate_path(run_dir: &str) -> String {
    join(&[run_dir, "gate.json"])
}

/// Every feature-review gate of the repository, one JSON line per recorded run in the critique directory of the git
/// common directory, oldest first: the latest line for a head is that head's gate.
pub fn gate_index_path(critique_dir: &str) -> String {
    join(&[critique_dir, "gates.jsonl"])
}

/// The gate of a run whose verdicts are adjudicated.
pub fn compute_gate(runtime: &Runtime, manifest: &RunManifest, findings: &[AdjudicatedFinding]) -> Result<GateRecord, CritiqueError> {
    let Some(code) = &manifest.code else {
        return Err(CritiqueError::new("invalid-run", format!("run {} is a feature review without the commits it reviewed", manifest.run_id)));
    };
    let spec = spec_reference(manifest);
    let tickets = spec_tickets(runtime, manifest)?;
    let mut follow_ups = Vec::new();
    for adjudicated in findings.iter().filter(|adjudicated| adjudicated.verdict.verdict == "accepted") {
        let given = adjudicated.verdict.follow_up.as_deref().unwrap_or("");
        let Some(ticket) = ticket_named(&tickets, given, &manifest.repository) else {
            return Err(CritiqueError::new(
                "invalid-input",
                format!("{}'s follow-up {given} is not a ticket of {spec}; create it as one of the spec's tickets first", adjudicated.finding.id),
            ));
        };
        follow_ups.push(FollowUp {
            finding_id: adjudicated.finding.id.clone(),
            open: ticket.open,
            reference: ticket.ticket.reference.clone(),
            severity: adjudicated.finding.body.severity.clone(),
            ticket: ticket.ticket.id.clone(),
        });
    }
    let earlier: Vec<GateRecord> = gate_index(runtime, manifest)?.into_iter().filter(|record| record.repository == manifest.repository && record.spec == spec).collect();
    // A follow-up that is no longer a ticket of the spec cannot be shown closed, so it still holds the gate.
    let is_open = |reference: &str| tickets.iter().find(|ticket| ticket.ticket.reference == reference).map_or(true, |ticket| ticket.open);
    let open = open_critical_follow_ups(&manifest.run_id, &follow_ups, &earlier, is_open);
    Ok(GateRecord {
        base_commit: code.base_commit.clone(),
        critical_open: open.len() as f64,
        gate: if open.is_empty() { "pass" } else { "fail" }.to_string(),
        head_commit: code.head_commit.clone(),
        line: gate_line(open.len()),
        follow_ups,
        open_critical: open,
        recorded_at: to_iso(runtime.clock.now_ms()),
        repository: manifest.repository.clone(),
        run_id: manifest.run_id.clone(),
        spec,
    })
}

/// Writes the gate into the run and appends it to the repository's gate index.
pub fn save_gate(runtime: &Runtime, manifest: &RunManifest, gate: &GateRecord) -> Result<(), CritiqueError> {
    let value = gate.to_json();
    runtime.files.write_text(&gate_path(&manifest.run_dir), &format!("{}\n", json::pretty(&value))).map_err(CritiqueError::from_fs)?;
    let index = gate_index_path(dirname(&manifest.run_dir));
    runtime.files.mkdir_all(dirname(&index)).map_err(CritiqueError::from_fs)?;
    runtime.files.append(&index, &format!("{}\n", json::compact(&value))).map_err(CritiqueError::from_fs)
}

/// The spec's tickets as the tracker holds them now; a tracker that cannot be read leaves the gate unknown.
fn spec_tickets(runtime: &Runtime, manifest: &RunManifest) -> Result<Vec<TrackedTicket>, CritiqueError> {
    let repository = Resolved { root: manifest.repo_root.clone(), common_dir: dirname(dirname(&manifest.run_dir)).to_string(), key: manifest.repository.clone() };
    let refused = |value: &Value| {
        let message = value.as_object().and_then(|object| object.get("message")).and_then(Value::as_str).unwrap_or_default();
        CritiqueError::new("tracker-unreachable", format!("the gate needs the spec's tickets: {message}"))
    };
    match resolve_tracker(runtime, &repository)? {
        TrackerResolution::Refused(value) => Err(refused(&value)),
        TrackerResolution::Resolved(tracker) => match tracker.tickets(runtime, &manifest.artifact.reference)? {
            TicketsRead::Refused(value) => Err(refused(&value)),
            TicketsRead::Read(tickets) => Ok(tickets),
        },
    }
}

/// Every gate recorded for the repository, oldest first.
fn gate_index(runtime: &Runtime, manifest: &RunManifest) -> Result<Vec<GateRecord>, CritiqueError> {
    let path = gate_index_path(dirname(&manifest.run_dir));
    if !runtime.files.exists(&path).unwrap_or(false) {
        return Ok(Vec::new());
    }
    let text = runtime.files.read_text(&path).map_err(CritiqueError::from_fs)?;
    let lines = text.split('\n').filter(|line| !model_gateway::text::js_trim(line).is_empty());
    lines.enumerate().map(|(n, line)| decoded("invalid-run", &format!("{path} line {}", n + 1), parse_gate_record(line))).collect()
}

/// The ticket a follow-up names: by its id (`#301`, `03`), its reference, or any form of issue reference.
fn ticket_named<'a>(tickets: &'a [TrackedTicket], given: &str, repository_key: &str) -> Option<&'a TrackedTicket> {
    let as_reference = parse_issue_reference(given, repository_key).map(|issue| format!("{}#{}", issue.repository, number_text(issue.number)));
    tickets.iter().find(|ticket| ticket.ticket.id == given || ticket.ticket.reference == given || Some(&ticket.ticket.reference) == as_reference.as_ref())
}
