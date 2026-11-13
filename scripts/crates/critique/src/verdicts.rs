//! The orchestrator's verdict on every finding, and what a verdict may say at each stage. The node runner's
//! `domain/verdicts.ts`, and the verdict reader of `application/record.ts`.

use crate::error::ShapeError;
use crate::fields::Fields;
use crate::findings::{RunFinding, SEVERITIES};
use crate::manifest::PacketFile;
use crate::stage_table::{candidate_questions as question_cap, mixed_candidates, second_round};

pub const VERDICTS: [&str; 4] = ["accepted", "rejected", "needs-human", "hint"];

/// More than this many accepted critical findings call for a second round.
pub const SECOND_ROUND_AFTER_CRITICAL: usize = 3;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Verdict {
    pub finding_id: String,
    pub verdict: String,
    /// Free text for the report; never written to the ledger.
    pub reason: Option<String>,
    /// A short slug the ledger may keep, such as `fabricated-evidence`.
    pub reason_code: Option<String>,
    /// On a needs-human verdict: the owner has answered the question.
    pub answered: Option<bool>,
    /// On an accepted feature-review verdict: the follow-up ticket the finding became.
    pub follow_up: Option<String>,
    /// On a feature-review verdict accepted by a cross-ticket trace: the packet's ticket files the trace crossed.
    pub trace_tickets: Option<Vec<String>>,
    /// On a ticket-review verdict accepted by `test-fails`: the repository path the test was kept at.
    pub regression_test: Option<String>,
    /// On an accepted ticket-review verdict: the finding is fixed in the change about to be committed.
    pub fixed: Option<bool>,
    /// The orchestrator's override of the severity the finding's check fixes; it needs `severity_reason`.
    pub severity: Option<String>,
    pub severity_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdjudicatedFinding {
    pub finding: RunFinding,
    pub verdict: Verdict,
}

fn is_reason_code(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=40).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..].iter().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// One verdict of a `record` input.
pub fn read_verdict(fields: &Fields<'_>) -> Result<Verdict, ShapeError> {
    let verdict = fields.one_of("verdict", &VERDICTS)?;
    let reason = if verdict == "rejected" || fields.has("reason") { Some(fields.string("reason")?) } else { None };
    let reason_code = fields.optional_matching("reason_code", is_reason_code, "a short lowercase slug")?;
    let finding_id = fields.string("finding_id")?;
    let answered = if fields.has("answered") { Some(fields.boolean("answered")?) } else { None };
    let follow_up = fields.optional_string("follow_up")?;
    let trace_tickets = if fields.has("trace_tickets") { Some(fields.strings("trace_tickets")?) } else { None };
    let regression_test = fields.optional_string("regression_test")?;
    let fixed = if fields.has("fixed") { Some(fields.boolean("fixed")?) } else { None };
    let severity = if fields.has("severity") { Some(fields.one_of("severity", &SEVERITIES)?) } else { None };
    let severity_reason = fields.optional_matching("severity_reason", is_reason_code, "a short lowercase slug")?;
    Ok(Verdict { finding_id, verdict, reason, reason_code, answered, follow_up, trace_tickets, regression_test, fixed, severity, severity_reason })
}

/// How many findings carry each kind of verdict, in the order `accepted`, `hint`, `needs-human`, `rejected`.
pub fn count_verdicts(findings: &[AdjudicatedFinding]) -> [usize; 4] {
    let mut counts = [0; 4];
    for adjudicated in findings {
        match adjudicated.verdict.verdict.as_str() {
            "accepted" => counts[0] += 1,
            "hint" => counts[1] += 1,
            "needs-human" => counts[2] += 1,
            _ => counts[3] += 1,
        }
    }
    counts
}

/// More than three accepted critical findings call for a second round, unless this was it.
pub fn second_round_due(stage: &str, round: f64, findings: &[AdjudicatedFinding]) -> bool {
    let critical = findings.iter().filter(|f| f.finding.body.severity == "critical" && f.verdict.verdict == "accepted").count();
    round == 1.0 && second_round(stage) && critical > SECOND_ROUND_AFTER_CRITICAL
}

/// Why a verdict does not fit its stage, if it does not. A question is answered on a needs-human verdict; at the
/// decisions stage every accepted finding is a question, so the answer goes on the accepted verdict.
pub fn stage_verdict_error(stage: &str, verdict: &Verdict, category: Option<&str>) -> Option<String> {
    let id = &verdict.finding_id;
    let candidate = question_cap(stage).is_some() && (!mixed_candidates(stage) || category == Some("undocumented-decision"));
    if candidate && verdict.verdict == "accepted" {
        return Some(format!("{id}: a recovered decision is never accepted: only the owner's answer writes a record, so give it needs-human, or rejected with the reason it fails"));
    }
    if stage == "decisions" {
        if verdict.verdict == "needs-human" {
            return Some(format!("{id}: the decisions stage asks every accepted gap as a question; give it accepted, not needs-human"));
        }
        if verdict.answered.is_some() && verdict.verdict != "accepted" {
            return Some(format!("{id}: only an accepted verdict can be answered at the decisions stage"));
        }
        return None;
    }
    (verdict.answered.is_some() && verdict.verdict != "needs-human").then(|| format!("{id}: only a needs-human verdict can be answered"))
}

/// The candidates a recovery run asks the owner about, and the ones it defers, as positions in the findings.
#[derive(Clone, Debug, PartialEq)]
pub struct CandidateQuestions {
    pub asked: Vec<usize>,
    pub deferred: Vec<usize>,
}

/// At a stage whose findings are candidates for the owner to confirm: the needs-human candidates ranked by severity
/// (critical first, then in finding order), the first of them asked and the rest deferred. `None` at every other
/// stage.
pub fn candidate_questions(stage: &str, findings: &[AdjudicatedFinding]) -> Option<CandidateQuestions> {
    let cap = question_cap(stage)?;
    let rank = |position: usize| SEVERITIES.iter().position(|level| *level == findings[position].finding.body.severity).unwrap_or(SEVERITIES.len());
    let mut ranked: Vec<usize> = (0..findings.len()).filter(|position| findings[*position].verdict.verdict == "needs-human").collect();
    ranked.sort_by_key(|position| (rank(*position), *position));
    let deferred = ranked.split_off(cap.min(ranked.len()));
    Some(CandidateQuestions { asked: ranked, deferred })
}

/// Why a verdict's severity override does not fit its finding, if it does not. Only a severity the finding's check
/// fixes can be overridden, to a different level and with a reason.
pub fn severity_override_problem(finding: &RunFinding, verdict: &Verdict) -> Option<String> {
    let id = &verdict.finding_id;
    let Some(severity) = &verdict.severity else {
        return verdict.severity_reason.as_ref().map(|_| format!("{id}: a severity_reason belongs to a severity override"));
    };
    let Some(declared) = &finding.declared_severity else {
        return Some(format!("{id}: only a finding whose check fixes its severity can be overridden"));
    };
    if severity == declared {
        return Some(format!("{id}: the override repeats the declared severity {severity}"));
    }
    verdict.severity_reason.is_none().then(|| format!("{id}: a severity override needs a severity_reason"))
}

/// Critical questions the owner has not answered yet; while any is open, tickets must not be cut from the spec.
pub fn open_critical_questions(findings: &[AdjudicatedFinding]) -> Vec<&AdjudicatedFinding> {
    findings
        .iter()
        .filter(|f| f.finding.body.severity == "critical" && f.verdict.verdict == "needs-human" && f.verdict.answered != Some(true))
        .collect()
}

/// The two ways a feature finding can be accepted: a failing test the orchestrator ran, or a trace it walked across
/// code from two or more tickets.
pub const FEATURE_ACCEPT_REASONS: [&str; 2] = ["test-fails", "cross-ticket-trace"];

/// The two ways a ticket finding can be accepted: a failing test, or a trace walked line by line.
pub const TICKET_ACCEPT_REASONS: [&str; 2] = ["test-fails", "trace-walked"];

/// What is wrong with a verdict given at this stage, if anything: the fields only a feature review or a ticket review
/// takes, and what the stage needs to accept a finding.
pub fn verdict_problem(stage: &str, verdict: &Verdict, packet_files: &[PacketFile]) -> Option<String> {
    feature_verdict_problem(stage, verdict, packet_files).or_else(|| ticket_verdict_problem(stage, verdict))
}

fn feature_verdict_problem(stage: &str, verdict: &Verdict, packet_files: &[PacketFile]) -> Option<String> {
    let id = &verdict.finding_id;
    let extras = verdict.follow_up.is_some() || verdict.trace_tickets.is_some();
    if stage != "feature-review" {
        return extras.then(|| format!("{id}: follow_up and trace_tickets belong to a feature-review verdict"));
    }
    if verdict.verdict != "accepted" {
        return extras.then(|| format!("{id}: only an accepted feature finding has a follow_up or trace_tickets"));
    }
    let reason = verdict.reason_code.as_deref();
    if !matches!(reason, Some("test-fails" | "cross-ticket-trace")) {
        return Some(format!("{id}: a feature finding is accepted only with reason_code {}", FEATURE_ACCEPT_REASONS.join(" or ")));
    }
    if reason == Some("cross-ticket-trace") {
        let tickets: Vec<&str> = packet_files.iter().filter(|file| file.kind == "ticket").map(|file| file.path.as_str()).collect();
        let mut crossed: Vec<&str> = verdict.trace_tickets.iter().flatten().map(String::as_str).collect();
        crossed.sort_unstable();
        crossed.dedup();
        if crossed.len() < 2 || crossed.iter().any(|path| !tickets.contains(path)) {
            return Some(format!("{id}: a cross-ticket-trace names at least two tickets of the packet in trace_tickets"));
        }
    } else if verdict.trace_tickets.is_some() {
        return Some(format!("{id}: trace_tickets belong to a cross-ticket-trace"));
    }
    verdict.follow_up.is_none().then(|| format!("{id}: an accepted feature finding names its follow-up ticket in follow_up"))
}

fn ticket_verdict_problem(stage: &str, verdict: &Verdict) -> Option<String> {
    let id = &verdict.finding_id;
    let extras = verdict.regression_test.is_some() || verdict.fixed.is_some();
    if stage != "ticket-review" {
        return extras.then(|| format!("{id}: regression_test and fixed belong to a ticket-review verdict"));
    }
    if verdict.verdict != "accepted" {
        return extras.then(|| format!("{id}: only an accepted ticket finding has a regression_test or is fixed"));
    }
    let reason = verdict.reason_code.as_deref();
    if !matches!(reason, Some("test-fails" | "trace-walked")) {
        return Some(format!("{id}: a ticket finding is accepted only with reason_code {}", TICKET_ACCEPT_REASONS.join(" or ")));
    }
    if reason == Some("test-fails") && verdict.regression_test.is_none() {
        return Some(format!("{id}: a finding accepted by test-fails names the test kept in the ticket's change in regression_test"));
    }
    if reason != Some("test-fails") && verdict.regression_test.is_some() {
        return Some(format!("{id}: regression_test belongs to a finding accepted by test-fails"));
    }
    None
}

/// A ticket review in which no critic reported: it vouches for no change, so its commit stays blocked.
pub fn degraded_ticket_review(stage: &str, degraded: bool) -> bool {
    stage == "ticket-review" && degraded
}

/// Accepted critical findings of a ticket review not yet fixed; while any is, the change must not be committed.
pub fn unfixed_critical<'a>(stage: &str, findings: &'a [AdjudicatedFinding]) -> Vec<&'a AdjudicatedFinding> {
    if stage != "ticket-review" {
        return Vec::new();
    }
    findings.iter().filter(|f| f.finding.body.severity == "critical" && f.verdict.verdict == "accepted" && f.verdict.fixed != Some(true)).collect()
}
