//! `findings.json`, which fan-out writes into the run and record reads back. The node runner's `parseFanOutResult` and
//! the skipped-check reader of `domain/evidence.ts`.

use crate::critic::{read_critic_run, CriticRun};
use crate::error::ShapeError;
use crate::fields::Fields;
use crate::findings::{read_run_finding, RunFinding};
use crate::stage::CRITIQUE_STAGES;
use model_gateway::json;

/// One check the critics could not run because the packet holds no code under `tree/`.
#[derive(Clone, Debug, PartialEq)]
pub struct SkippedCheck {
    pub check: String,
    /// `category`, `role <name>` or `lens <name>`.
    pub from: String,
    pub about: Option<String>,
}

/// What fan-out found: every critic's run and the merged findings.
#[derive(Clone, Debug, PartialEq)]
pub struct FanOutResult {
    pub critics: Vec<CriticRun>,
    /// True when no critic of the stage produced a usable review.
    pub degraded: bool,
    pub findings: Vec<RunFinding>,
    /// Why the gateway's ledger would not take a run, one message per failed append.
    pub ledger_warnings: Option<Vec<String>>,
    pub skipped_checks: Option<Vec<SkippedCheck>>,
}

fn read_skipped_check(fields: &Fields<'_>) -> Result<SkippedCheck, ShapeError> {
    let about = fields.optional_string("about")?;
    Ok(SkippedCheck { check: fields.string("check")?, from: fields.string("from")?, about })
}

/// Reads `findings.json`; a syntax error and a wrong shape both yield the message the node runner's `decoded` prints
/// after the file's path.
pub fn parse_fan_out_result(text: &str) -> Result<FanOutResult, String> {
    let value = json::parse(text)?;
    let file = Fields::of(&value, "").map_err(|error| error.0)?;
    read_fan_out_result(&file).map_err(|error| error.0)
}

fn read_fan_out_result(file: &Fields<'_>) -> Result<FanOutResult, ShapeError> {
    let critics = file.list("critics", read_critic_run)?;
    let degraded = file.boolean("degraded")?;
    let findings = file.list("findings", read_run_finding)?;
    file.string("findings_path")?;
    let ledger_warnings = if file.has("ledger_warnings") { Some(file.strings("ledger_warnings")?) } else { None };
    file.string("run_id")?;
    let skipped_checks = if file.has("skipped_checks") { Some(file.list("skipped_checks", read_skipped_check)?) } else { None };
    file.one_of("stage", &CRITIQUE_STAGES)?;
    file.literal_str("status", "reviewed")?;
    Ok(FanOutResult { critics, degraded, findings, ledger_warnings, skipped_checks })
}
