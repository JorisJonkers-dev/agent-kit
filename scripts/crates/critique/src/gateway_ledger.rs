//! Reads the gateway's ledger for critique's records. A line that is not one of
//! them — a delegation's record, a line a later version wrote, a write cut short
//! — is skipped, so the gateway's own history never hides critique's.

use crate::budget::{LoggedInvocation, KINDS};
use crate::error::ShapeError;
use crate::fields::Fields;
use crate::ledger::{LedgerCriticRun, LedgerRunContext, PromptHashes, CRITIC_STATUSES};
use crate::stage::CRITIQUE_STAGES;
use model_gateway::json;
use model_gateway::text::js_trim;

/// What the gateway's ledger holds for critique: the runs it recorded, and the
/// launches they were recorded for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GatewayLedgerLines {
    pub runs: Vec<LedgerCriticRun>,
    pub invocations: Vec<LoggedInvocation>,
}

/// Reads the gateway's ledger for critique's records. A record without prompt
/// hashes is a run that reviewed nothing, such as the triage model's question:
/// it counts as spend, never as a critic's cell.
pub fn gateway_ledger_lines(text: &str) -> GatewayLedgerLines {
    let mut runs: Vec<LedgerCriticRun> = Vec::new();
    let mut invocations: Vec<LoggedInvocation> = Vec::new();
    for line in text.split('\n') {
        if let Some((run, invocation)) = critique_record(line) {
            invocations.push(invocation);
            if let Some(run) = run {
                runs.push(run);
            }
        }
    }
    GatewayLedgerLines { runs, invocations }
}

/// One line's record, or nothing when the line is blank or not one of critique's.
fn critique_record(line: &str) -> Option<(Option<LedgerCriticRun>, LoggedInvocation)> {
    if js_trim(line).is_empty() {
        return None;
    }
    // A line a later version wrote, or a write cut short: the rest of the ledger still counts.
    read_record(line).ok().flatten()
}

fn read_record(line: &str) -> Result<Option<(Option<LedgerCriticRun>, LoggedInvocation)>, ShapeError> {
    let value = json::parse(line).map_err(ShapeError)?;
    let fields = Fields::of(&value, "")?;
    if !fields.has("type") || fields.optional_string("type")?.as_deref() != Some("invocation") {
        return Ok(None);
    }
    let flow = fields.string("workflow")?;
    let recorded_at = fields.string("recorded_at")?;
    let repository = fields.string("repository")?;
    let round = fields.number("round")?;
    let run_id = fields.string("run_id")?;
    let spec = if fields.has("spec") { Some(fields.string("spec")?) } else { None };
    let stage = fields.one_of("stage", &CRITIQUE_STAGES)?;
    let invocation = LoggedInvocation {
        charged_usd: fields.number("charged_usd")?,
        critic_id: fields.string("critic_id")?,
        flow: flow.clone(),
        kind: fields.one_of("kind", &KINDS)?,
        recorded_at: recorded_at.clone(),
        repository: repository.clone(),
        run_id: Some(run_id.clone()),
    };
    if !fields.has("prompt_hashes") {
        return Ok(Some((None, invocation)));
    }
    let hashes = fields.object("prompt_hashes")?;
    let run = LedgerCriticRun {
        context: LedgerRunContext {
            recorded_at,
            repository,
            run_id,
            spec,
            flow: Some(flow),
            round,
            stage,
        },
        client: fields.string("critic_client")?,
        cost_usd: fields.nullable_number("critic_cost_usd")?,
        critic_id: fields.string("critic_id")?,
        failure: fields.optional_string("failure")?,
        // A run that leaves these undefined is written without them, so a record this code wrote reads back.
        finding_count: if fields.has("finding_count") { fields.number("finding_count")? } else { 0.0 },
        lens_hashes: if fields.has("lens_hashes") { fields.string_map("lens_hashes")? } else { Vec::new() },
        lenses: if fields.has("lenses") { fields.strings("lenses")? } else { Vec::new() },
        model: fields.string("critic_model")?,
        prompt_hashes: PromptHashes {
            contract: hashes.string("contract")?,
            preamble: hashes.string("preamble")?,
            role: hashes.string("role")?,
            schema: hashes.string("schema")?,
        },
        role: fields.optional_string("role")?.unwrap_or_default(),
        status: fields.one_of("status", &CRITIC_STATUSES)?,
    };
    Ok(Some((Some(run), invocation)))
}
