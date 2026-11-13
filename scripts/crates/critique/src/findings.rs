//! A finding as a critic reports it and as the orchestrator receives it, read against the output contract. The node
//! runner's `domain/findings.ts`.

use crate::cold_read::last_fenced_json;
use crate::error::ShapeError;
use crate::fields::Fields;
use model_gateway::json::{self, Value};
use model_gateway::text::js_trim;

pub const SEVERITIES: [&str; 3] = ["critical", "major", "minor"];

pub const CATEGORIES: [&str; 13] = [
    "unasked-question",
    "contradiction",
    "omission",
    "invention",
    "untestable",
    "failure-mode",
    "security",
    "structure",
    "bug",
    "integration",
    "drift",
    "stale",
    "undocumented-decision",
];

pub const EVIDENCE_KINDS: [&str; 4] = ["quote", "input", "failing-test", "trace"];

/// The codes of a mechanical rejection, which the orchestrator cannot overturn: `fabricated-evidence` and a recovered
/// decision's three.
pub const MECHANICAL_REJECTIONS: [&str; 4] = ["fabricated-evidence", "no-code-trace", "single-site", "invented-rationale"];

pub const HINT_CODES: [&str; 22] = [
    "no-evidence",
    "no-spec-quote",
    "no-decision-quote",
    "no-decision-or-scenario",
    "no-test-or-trace",
    "no-ticket-rule",
    "no-ticket-id",
    "no-scenario",
    "not-a-decision-gap",
    "no-record-quote",
    "one-file-contradiction",
    "needs-tree",
    "not-a-candidate",
    "no-layer-config",
    "no-trace",
    "verdict-word",
    "no-rule-or-consequence",
    "no-observable",
    "no-plan-step",
    "no-plan-rule",
    "no-step-quote",
    "no-plan-anchor",
];

/// Hints about the check a finding names, given after the stage's evidence rule found nothing to say.
pub const CHECK_HINT_CODES: [&str; 2] = ["no-check", "unknown-check"];

#[derive(Clone, Debug, PartialEq)]
pub struct Evidence {
    pub kind: String,
    pub value: String,
    pub file: Option<String>,
}

/// What a finding claims, and the evidence for it.
#[derive(Clone, Debug, PartialEq)]
pub struct FindingBody {
    pub severity: String,
    pub category: String,
    pub location: String,
    pub claim: String,
    pub evidence: Vec<Evidence>,
    pub suggestion: Option<String>,
    /// The check id of the lens bullet or role rule the finding comes from; a finding without one still reads, and
    /// fan-out gives it a hint.
    pub check: Option<String>,
}

/// One finding as a critic reported it.
#[derive(Clone, Debug, PartialEq)]
pub struct CriticFinding {
    pub id: String,
    pub body: FindingBody,
}

/// The verdict fan-out's evidence check gives a finding before the orchestrator sees it: `rejected` for good (a
/// mechanical rejection) or a `hint`, which the orchestrator may still accept or reject.
#[derive(Clone, Debug, PartialEq)]
pub struct PreVerdict {
    pub verdict: String,
    pub reason_code: String,
    pub reason: String,
}

/// A finding as the orchestrator receives it: the critic's fields plus who raised it.
#[derive(Clone, Debug, PartialEq)]
pub struct RunFinding {
    pub id: String,
    pub body: FindingBody,
    pub critic_finding_id: String,
    pub raised_by: Vec<String>,
    pub pre_verdict: Option<PreVerdict>,
    /// The severity the finding's check fixes, set by fan-out in place of the critic's.
    pub declared_severity: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FindingsParse {
    Ok { findings: Vec<CriticFinding>, no_findings_reason: Option<String> },
    /// `failure` is one of `no-findings-block`, `unparseable-findings`, `invalid-findings` and
    /// `missing-no-findings-reason`.
    Failed { failure: &'static str, detail: String },
}

/// Reads the last ```json block of a critic's final message and checks it against the output contract.
pub fn parse_findings(text: &str) -> FindingsParse {
    let Some(block) = last_fenced_json(text) else {
        return FindingsParse::Failed { failure: "no-findings-block", detail: "the reply has no ```json block".to_string() };
    };
    match json::parse(block) {
        Ok(value) => read_findings(&value),
        Err(message) => FindingsParse::Failed { failure: "unparseable-findings", detail: format!("SyntaxError: {message}") },
    }
}

/// Checks a findings object, already parsed from JSON, against the output contract.
pub fn read_findings(value: &Value) -> FindingsParse {
    let read = Fields::of(value, "the block").and_then(|fields| fields.list("findings", read_critic_finding));
    let findings = match read {
        Ok(findings) => findings,
        Err(error) => return FindingsParse::Failed { failure: "invalid-findings", detail: format!("JsonShapeError: {}", error.0) },
    };
    let reason = match value.as_object().and_then(|object| object.get("no_findings_reason")) {
        Some(Value::Str(text)) => js_trim(text).to_string(),
        _ => String::new(),
    };
    if findings.is_empty() && reason.is_empty() {
        return FindingsParse::Failed { failure: "missing-no-findings-reason", detail: "an empty findings list needs no_findings_reason".to_string() };
    }
    FindingsParse::Ok { findings, no_findings_reason: (!reason.is_empty()).then_some(reason) }
}

/// The fields of a finding, read against the output contract.
pub fn read_finding_body(fields: &Fields<'_>) -> Result<FindingBody, ShapeError> {
    let suggestion = fields.optional_text("suggestion")?;
    // An empty check is no check: it earns a hint, not a failed review.
    let check = fields.optional_text("check")?.map(|text| js_trim(&text).to_string()).filter(|text| !text.is_empty());
    Ok(FindingBody {
        category: fields.one_of("category", &CATEGORIES)?,
        check,
        claim: fields.string("claim")?,
        evidence: fields.list("evidence", read_evidence)?,
        location: fields.string("location")?,
        severity: fields.one_of("severity", &SEVERITIES)?,
        suggestion,
    })
}

pub fn read_run_finding(fields: &Fields<'_>) -> Result<RunFinding, ShapeError> {
    let body = read_finding_body(fields)?;
    let critic_finding_id = fields.string("critic_finding_id")?;
    let id = fields.string("id")?;
    let raised_by = fields.strings("raised_by")?;
    let declared_severity = if fields.has("declared_severity") { Some(fields.one_of("declared_severity", &SEVERITIES)?) } else { None };
    let pre_verdict = if fields.has("pre_verdict") { Some(read_pre_verdict(&fields.object("pre_verdict")?)?) } else { None };
    Ok(RunFinding { id, body, critic_finding_id, raised_by, pre_verdict, declared_severity })
}

fn read_pre_verdict(fields: &Fields<'_>) -> Result<PreVerdict, ShapeError> {
    let reason = fields.string("reason")?;
    let verdict = fields.one_of("verdict", &["rejected", "hint"])?;
    let reason_code = if verdict == "rejected" {
        fields.one_of("reason_code", &MECHANICAL_REJECTIONS)?
    } else {
        let hints: Vec<&str> = HINT_CODES.iter().chain(CHECK_HINT_CODES.iter()).copied().collect();
        fields.one_of("reason_code", &hints)?
    };
    Ok(PreVerdict { verdict, reason_code, reason })
}

fn read_critic_finding(fields: &Fields<'_>) -> Result<CriticFinding, ShapeError> {
    Ok(CriticFinding { body: read_finding_body(fields)?, id: fields.string("id")? })
}

fn read_evidence(fields: &Fields<'_>) -> Result<Evidence, ShapeError> {
    let file = fields.optional_string("file")?;
    Ok(Evidence { kind: fields.one_of("kind", &EVIDENCE_KINDS)?, value: fields.string("value")?, file })
}
