//! A run with the orchestrator's verdict on every finding: the verdicts weighed against fan-out's pre-verdicts and the
//! refute round's results, and what the report and the ledger read from it. The node runner's `domain/verdicts.ts`
//! (`AdjudicatedRun`, `familiesOf`, `isDissent`) and the adjudication half of `application/record.ts`.

use crate::critic::CriticRun;
use crate::error::CritiqueError;
use crate::fan_out_result::{FanOutResult, SkippedCheck};
use crate::feature_review::GateRecord;
use crate::findings::{RunFinding, MECHANICAL_REJECTIONS};
use crate::manifest::{spec_reference, DepthChoice, RunManifest, TicketCheckSummary};
use crate::refute::{RefuteFile, RefuteRecord};
use crate::roster::DroppedCritic;
use crate::verdicts::{candidate_questions, severity_override_problem, stage_verdict_error, verdict_problem, AdjudicatedFinding, Verdict};
use std::collections::{HashMap, HashSet};

/// A run as record sees it once every finding has its verdict.
#[derive(Clone, Debug, PartialEq)]
pub struct AdjudicatedRun {
    pub run_id: String,
    pub stage: String,
    pub depth: String,
    pub round: f64,
    pub previous_run_id: Option<String>,
    pub depth_choice: DepthChoice,
    pub repository: String,
    /// The spec the run belongs to, as the artifact's reference names it. Stats counts features by it.
    pub spec: String,
    /// The flow whose Vibe spend the run adds to.
    pub flow: String,
    /// True when no critic produced a usable review.
    pub degraded: bool,
    pub critics: Vec<CriticRun>,
    pub dropped: Vec<DroppedCritic>,
    pub findings: Vec<AdjudicatedFinding>,
    /// Only for the tickets stage: what the ticket checker found when the packet was frozen.
    pub ticket_check: Option<TicketCheckSummary>,
    /// A feature review's gate, for the head it reviewed.
    pub gate: Option<GateRecord>,
    /// The refute round's results, when the run had one; already applied to `findings`.
    pub refutes: Option<Vec<RefuteRecord>>,
    /// The model gateway's warning for each Vibe plan window at or above its warning ratio.
    pub plan_warnings: Option<Vec<String>>,
    /// Why the gateway's ledger would not take a run, one message per failed append.
    pub ledger_warnings: Option<Vec<String>>,
    /// What the critics could not check because the packet holds no code under `tree/`.
    pub skipped_checks: Option<Vec<SkippedCheck>>,
}

/// The families of the critics that raised a finding, each once, in the order the critics ran.
pub fn families_of(critics: &[CriticRun], finding: &RunFinding) -> Vec<String> {
    let mut families: Vec<String> = Vec::new();
    for critic in critics.iter().filter(|critic| finding.raised_by.contains(&critic.critic_id)) {
        if !families.contains(&critic.family) {
            families.push(critic.family.clone());
        }
    }
    families
}

/// The orchestrator's rejection of a finding with a strong signal: raised by two or more families, or critical. A
/// finding rejected for fabricated evidence is not dissent: its evidence does not exist, and no orchestrator ruled
/// against it.
pub fn is_dissent(critics: &[CriticRun], finding: &AdjudicatedFinding) -> bool {
    finding.verdict.verdict == "rejected"
        && !MECHANICAL_REJECTIONS.contains(&finding.verdict.reason_code.as_deref().unwrap_or(""))
        && (finding.finding.body.severity == "critical" || families_of(critics, &finding.finding).len() >= 2)
}

/// The verdicts by the finding they are for; one for a finding the run does not have, or a second for one, is input
/// the command refuses.
pub fn verdicts_by_finding(list: &[Verdict], known: &HashSet<String>) -> Result<HashMap<String, Verdict>, CritiqueError> {
    let mut verdicts = HashMap::new();
    for verdict in list {
        if !known.contains(&verdict.finding_id) {
            return Err(CritiqueError::new("invalid-input", format!("unknown finding {}", verdict.finding_id)));
        }
        if verdicts.contains_key(&verdict.finding_id) {
            return Err(CritiqueError::new("invalid-input", format!("more than one verdict for {}", verdict.finding_id)));
        }
        verdicts.insert(verdict.finding_id.clone(), verdict.clone());
    }
    Ok(verdicts)
}

/// Every finding with the orchestrator's verdict, then the refute round applied, so the second-round count sees only
/// the criticals still accepted.
pub fn adjudicate_findings(
    verdicts: &HashMap<String, Verdict>,
    findings: &[RunFinding],
    refutes: &[RefuteRecord],
) -> Result<Vec<AdjudicatedFinding>, CritiqueError> {
    let adjudicated = findings.iter().map(|finding| adjudicate(verdicts, finding)).collect::<Result<Vec<_>, _>>()?;
    Ok(apply_refutes(adjudicated, refutes))
}

/// The run's manifest, fan-out's result and the adjudicated findings, as one run for the report and the ledger.
pub fn build_run(
    manifest: &RunManifest,
    merged: &FanOutResult,
    findings: Vec<AdjudicatedFinding>,
    refuted: Option<&RefuteFile>,
    gate: Option<GateRecord>,
) -> AdjudicatedRun {
    let warnings: Vec<String> =
        merged.ledger_warnings.iter().flatten().chain(refuted.and_then(|file| file.ledger_warnings.as_ref()).into_iter().flatten()).cloned().collect();
    AdjudicatedRun {
        run_id: manifest.run_id.clone(),
        stage: manifest.stage.clone(),
        depth: manifest.depth.clone(),
        round: manifest.round,
        previous_run_id: manifest.previous_run_id.clone(),
        depth_choice: manifest.depth_choice.clone(),
        repository: manifest.repository.clone(),
        spec: spec_reference(manifest),
        flow: manifest.flow.clone(),
        degraded: merged.degraded,
        critics: merged.critics.clone(),
        dropped: manifest.dropped.clone(),
        findings,
        ticket_check: manifest.ticket_check.clone(),
        gate,
        refutes: refuted.map(|file| file.results.clone()),
        plan_warnings: manifest.plan_warnings.clone(),
        ledger_warnings: (!warnings.is_empty()).then_some(warnings),
        skipped_checks: merged.skipped_checks.clone(),
    }
}

/// Applies the refute round to the orchestrator's verdicts. A refutation that holds rejects the finding (`refuted`); a
/// downgrade that holds lowers its severity and keeps it accepted. Every other outcome leaves the verdict standing, and
/// a finding the orchestrator no longer accepts is left as it ruled.
fn apply_refutes(findings: Vec<AdjudicatedFinding>, refutes: &[RefuteRecord]) -> Vec<AdjudicatedFinding> {
    findings
        .into_iter()
        .map(|mut adjudicated| {
            let Some(result) = refutes.iter().find(|refute| refute.finding_id == adjudicated.finding.id) else { return adjudicated };
            if adjudicated.verdict.verdict != "accepted" {
                return adjudicated;
            }
            if let (true, Some(severity)) = (result.outcome == "downgrade", &result.severity) {
                adjudicated.finding.body.severity = severity.clone();
                return adjudicated;
            }
            if result.outcome != "refute" {
                return adjudicated;
            }
            let by = result.refuter.as_ref().map_or("the refuter", |refuter| refuter.critic_id.as_str());
            let claim = result.reply.as_ref().map_or("", |reply| reply.claim.as_str());
            adjudicated.verdict = Verdict {
                finding_id: adjudicated.finding.id.clone(),
                reason: Some(format!("Refuted by {by}: {claim}")),
                reason_code: Some("refuted".to_string()),
                verdict: "rejected".to_string(),
                severity: adjudicated.verdict.severity.clone(),
                severity_reason: adjudicated.verdict.severity_reason.clone(),
                ..Verdict::default()
            };
            adjudicated
        })
        .collect()
}

/// A finding with the orchestrator's verdict. A severity override replaces the severity the finding's check fixed, and
/// stays on the verdict even where a pre-verdict stands in for the rest of it.
fn adjudicate(verdicts: &HashMap<String, Verdict>, finding: &RunFinding) -> Result<AdjudicatedFinding, CritiqueError> {
    let given = verdicts.get(&finding.id);
    let mut verdict = verdict_for(verdicts, finding)?;
    let mut finding = finding.clone();
    if let Some(severity) = given.and_then(|given| given.severity.as_ref()) {
        finding.body.severity = severity.clone();
        verdict.severity = Some(severity.clone());
        if let Some(reason) = given.and_then(|given| given.severity_reason.as_ref()) {
            verdict.severity_reason = Some(reason.clone());
        }
    }
    Ok(AdjudicatedFinding { finding, verdict })
}

/// The orchestrator's verdict, weighed against fan-out's pre-verdict. A fabricated-evidence rejection stands whatever
/// the orchestrator says, and only a rejection may restate it. A hint stands unless the orchestrator accepts or rejects
/// the finding. A finding without a pre-verdict needs a verdict.
fn verdict_for(verdicts: &HashMap<String, Verdict>, finding: &RunFinding) -> Result<Verdict, CritiqueError> {
    let id = &finding.id;
    let given = verdicts.get(id);
    let Some(pre) = &finding.pre_verdict else {
        return given.cloned().ok_or_else(|| CritiqueError::new("invalid-input", format!("no verdict for {id}")));
    };
    let mechanical = Verdict {
        finding_id: id.clone(),
        reason: Some(pre.reason.clone()),
        reason_code: Some(pre.reason_code.clone()),
        verdict: pre.verdict.clone(),
        ..Verdict::default()
    };
    if pre.verdict == "rejected" {
        if given.is_some_and(|given| given.verdict != "rejected") {
            return Err(CritiqueError::new("invalid-input", format!("{id} was rejected as {}, which a verdict cannot overturn", pre.reason_code)));
        }
        return Ok(mechanical);
    }
    if given.is_some_and(|given| given.verdict == "needs-human") {
        return Err(CritiqueError::new("invalid-input", format!("{id} is a hint: its verdict may only be accepted, rejected or hint")));
    }
    Ok(given.cloned().unwrap_or(mechanical))
}

/// The verdicts of a `record` input, each checked against its stage and its finding, and by the finding they are for.
/// The first verdict that does not fit is the error.
pub fn checked_verdicts(manifest: &RunManifest, merged: &FanOutResult, list: &[Verdict]) -> Result<HashMap<String, Verdict>, CritiqueError> {
    for verdict in list {
        let category = merged.findings.iter().find(|finding| finding.id == verdict.finding_id).map(|finding| finding.body.category.as_str());
        let error = stage_verdict_error(&manifest.stage, verdict, category).or_else(|| verdict_problem(&manifest.stage, verdict, &manifest.packet_files));
        if let Some(error) = error {
            return Err(CritiqueError::new("invalid-input", error));
        }
    }
    let known: HashSet<String> = merged.findings.iter().map(|finding| finding.id.clone()).collect();
    let verdicts = verdicts_by_finding(list, &known)?;
    for finding in &merged.findings {
        if let Some(error) = verdicts.get(&finding.id).and_then(|given| severity_override_problem(finding, given)) {
            return Err(CritiqueError::new("invalid-input", error));
        }
    }
    Ok(verdicts)
}

/// The refute round's file, which must refute only findings of the run.
pub fn check_refutes(path: &str, findings: &[RunFinding], file: &RefuteFile) -> Result<(), CritiqueError> {
    match file.results.iter().find(|result| !findings.iter().any(|finding| finding.id == result.finding_id)) {
        Some(unknown) => Err(CritiqueError::new("invalid-run", format!("{path} refutes {}, which the run does not have", unknown.finding_id))),
        None => Ok(()),
    }
}

/// A question the run's cap left out of the questions it asks must not be answered: the owner was never asked it.
pub fn unasked_answer(run: &AdjudicatedRun) -> Option<CritiqueError> {
    let candidates = candidate_questions(&run.stage, &run.findings)?;
    let unasked = candidates.deferred.iter().map(|position| &run.findings[*position]).find(|finding| finding.verdict.answered == Some(true))?;
    Some(CritiqueError::new(
        "invalid-input",
        format!("{} is past the {} questions a run asks, so it was not asked and cannot be answered", unasked.finding.id, candidates.asked.len()),
    ))
}
