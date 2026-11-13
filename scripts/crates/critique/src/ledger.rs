//! Reads a ledger back, field by field: the runs, findings, skipped reviews and
//! flow events critique's own ledger holds. A blank line is skipped, and so is a
//! line of a type this version does not know, so a later ticket can add one.

use crate::error::ShapeError;
use crate::fields::Fields;
use crate::stage::CRITIQUE_STAGES;
use model_gateway::json;
use model_gateway::text::js_trim;

/// The statuses a critic run can end in.
pub const CRITIC_STATUSES: [&str; 4] = ["ok", "failed", "violated-readonly", "budget-skip"];

/// The severities a finding can carry.
pub const SEVERITIES: [&str; 3] = ["critical", "major", "minor"];

/// The verdicts a finding can carry.
pub const VERDICTS: [&str; 4] = ["accepted", "rejected", "needs-human", "hint"];

/// The events the flow state records.
pub const FLOW_EVENTS: [&str; 5] = ["grilled", "no-critique", "landed", "answered", "draft"];

/// The status of the run line a low-risk triage writes for the ticket review it skipped.
pub const SKIPPED_LOW_RISK: &str = "skipped-low-risk";

/// What every ledger line says about the run it belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct LedgerRunContext {
    pub recorded_at: String,
    pub repository: String,
    pub run_id: String,
    /// Missing on lines recorded before the ledger named the spec.
    pub spec: Option<String>,
    /// The flow whose Vibe cap the run counted against; missing on lines recorded before flows.
    pub flow: Option<String>,
    /// 1 on lines recorded before rounds.
    pub round: f64,
    pub stage: String,
}

/// The hashes a critic's prompt was built from.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptHashes {
    pub contract: String,
    pub preamble: String,
    pub role: String,
    pub schema: String,
}

/// One critic's run, as stats reads it back.
#[derive(Clone, Debug, PartialEq)]
pub struct LedgerCriticRun {
    pub context: LedgerRunContext,
    pub critic_id: String,
    pub client: String,
    pub model: String,
    pub role: String,
    pub lenses: Vec<String>,
    pub prompt_hashes: PromptHashes,
    pub lens_hashes: Vec<(String, String)>,
    pub cost_usd: Option<f64>,
    pub status: String,
    pub failure: Option<String>,
    pub finding_count: f64,
}

/// One adjudicated finding, as stats reads it back.
#[derive(Clone, Debug, PartialEq)]
pub struct LedgerFinding {
    pub context: LedgerRunContext,
    pub finding_id: String,
    pub severity: String,
    pub raised_by: Vec<String>,
    pub verdict: String,
    pub pre_verdict: Option<String>,
    /// False on lines recorded before the ledger kept answers.
    pub answered: bool,
}

/// A ticket review a low-risk triage skipped, as the flow state reads it back.
#[derive(Clone, Debug, PartialEq)]
pub struct LedgerSkippedReview {
    pub run_id: String,
    pub recorded_at: String,
    pub repository: String,
    pub flow: String,
    /// The ticket's reference.
    pub ticket: String,
}

/// What the owner told the flow state that no run records.
#[derive(Clone, Debug, PartialEq)]
pub enum FlowEvent {
    Grilled,
    NoCritique(bool),
    Landed(String),
    Answered { run_id: String, finding_id: String },
    Draft(String),
}

/// Where and when a flow event was recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct LedgerFlowEvent {
    pub recorded_at: String,
    pub repository: String,
    pub flow: String,
    pub event: FlowEvent,
}

/// What a ledger holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ledger {
    pub runs: Vec<LedgerCriticRun>,
    pub findings: Vec<LedgerFinding>,
    pub skipped_reviews: Vec<LedgerSkippedReview>,
    pub flow_events: Vec<LedgerFlowEvent>,
}

/// Reads a ledger back, field by field. A blank line is skipped, and so is a
/// line of a type this version does not know, so a later ticket can add one.
/// Any other line without the shape record writes is a shape error naming its
/// line number.
pub fn parse_ledger(text: &str) -> Result<Ledger, ShapeError> {
    let mut runs: Vec<LedgerCriticRun> = Vec::new();
    let mut findings: Vec<LedgerFinding> = Vec::new();
    let mut skipped: Vec<LedgerSkippedReview> = Vec::new();
    let mut events: Vec<LedgerFlowEvent> = Vec::new();
    for (index, line) in text.split('\n').enumerate() {
        if js_trim(line).is_empty() {
            continue;
        }
        let number = index + 1;
        let value = json::parse(line).map_err(|_| ShapeError(format!("line {number} is not JSON")))?;
        let fields = Fields::of(&value, &format!("line {number}"))?;
        let r#type = fields.string("type")?;
        // A low-risk triage writes a run line for the review it skipped, which no critic ran.
        if r#type == "run" && fields.optional_string("status")?.as_deref() == Some(SKIPPED_LOW_RISK) {
            skipped.push(read_skipped_review(&fields)?);
        } else if r#type == "run" {
            runs.push(read_critic_run_line(&fields)?);
        }
        if r#type == "finding" {
            findings.push(read_finding_line(&fields)?);
        }
        if r#type == "flow" {
            events.push(read_flow_event(&fields)?);
        }
    }
    Ok(Ledger { runs, findings, skipped_reviews: skipped, flow_events: events })
}

fn read_skipped_review(fields: &Fields) -> Result<LedgerSkippedReview, ShapeError> {
    Ok(LedgerSkippedReview {
        flow: fields.string("flow")?,
        recorded_at: fields.string("recorded_at")?,
        repository: fields.string("repository")?,
        run_id: fields.string("run_id")?,
        ticket: fields.string("ticket")?,
    })
}

fn read_flow_event(fields: &Fields) -> Result<LedgerFlowEvent, ShapeError> {
    let flow = fields.string("flow")?;
    let recorded_at = fields.string("recorded_at")?;
    let repository = fields.string("repository")?;
    let event = match fields.one_of("event", &FLOW_EVENTS)?.as_str() {
        "no-critique" => FlowEvent::NoCritique(fields.boolean("value")?),
        "landed" => FlowEvent::Landed(fields.string("ticket")?),
        "answered" => FlowEvent::Answered { finding_id: fields.string("finding_id")?, run_id: fields.string("run_id")? },
        "draft" => FlowEvent::Draft(fields.string("draft")?),
        _ => FlowEvent::Grilled,
    };
    Ok(LedgerFlowEvent { recorded_at, repository, flow, event })
}

fn read_context(fields: &Fields) -> Result<LedgerRunContext, ShapeError> {
    Ok(LedgerRunContext {
        recorded_at: fields.string("recorded_at")?,
        repository: fields.string("repository")?,
        run_id: fields.string("run_id")?,
        spec: fields.optional_string("spec")?,
        flow: fields.optional_string("flow")?,
        round: if fields.has("round") { fields.number("round")? } else { 1.0 },
        stage: fields.one_of("stage", &CRITIQUE_STAGES)?,
    })
}

fn read_critic_run_line(fields: &Fields) -> Result<LedgerCriticRun, ShapeError> {
    let hashes = fields.object("prompt_hashes")?;
    let context = read_context(fields)?;
    let client = fields.string("client")?;
    let cost_usd = fields.nullable_number("cost_usd")?;
    let critic_id = fields.string("critic_id")?;
    let failure = fields.nullable_string("failure")?;
    let finding_count = if fields.has("finding_count") { fields.number("finding_count")? } else { 0.0 };
    let lens_hashes = fields.string_map("lens_hashes")?;
    let lenses = fields.strings("lenses")?;
    let model = fields.string("model")?;
    let prompt_hashes = PromptHashes {
        contract: hashes.string("contract")?,
        preamble: hashes.string("preamble")?,
        role: hashes.string("role")?,
        schema: hashes.string("schema")?,
    };
    let role = fields.string("role")?;
    let status = fields.one_of("status", &CRITIC_STATUSES)?;
    Ok(LedgerCriticRun { context, critic_id, client, model, role, lenses, prompt_hashes, lens_hashes, cost_usd, status, failure, finding_count })
}

fn read_finding_line(fields: &Fields) -> Result<LedgerFinding, ShapeError> {
    let context = read_context(fields)?;
    let answered = fields.has("answered") && fields.boolean("answered")?;
    let finding_id = fields.string("finding_id")?;
    let pre_verdict = fields.nullable_string("pre_verdict")?;
    let severity = fields.one_of("severity", &SEVERITIES)?;
    let raised_by = fields.strings("raised_by")?;
    let verdict = fields.one_of("verdict", &VERDICTS)?;
    Ok(LedgerFinding { context, finding_id, severity, raised_by, verdict, pre_verdict, answered })
}
