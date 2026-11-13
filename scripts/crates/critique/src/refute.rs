//! The refute round's file, which `refute` writes beside the run and `record` reads. The node runner's
//! `domain/refute.ts`, the half of it record needs.

use crate::critic::{read_critic_run, CriticRun};
use crate::error::ShapeError;
use crate::fields::Fields;
use crate::findings::{read_finding_body, FindingBody, CHECK_HINT_CODES, HINT_CODES, MECHANICAL_REJECTIONS, SEVERITIES};
use crate::stage::CRITIQUE_STAGES;
use model_gateway::json;

/// What a refuter may answer: an empty findings list is no objection, and one finding whose check names the refuter
/// role's `refute` or `downgrade` rule is the objection.
pub const REFUTE_ANSWERS: [&str; 3] = ["no-objection", "refute", "downgrade"];

/// What the round did to one accepted critical finding: the refuter's answer when it holds, `stands` when the refuter
/// objected without evidence the packet supports or gave no usable answer, and `skipped` when no client could refute it.
pub const REFUTE_OUTCOMES: [&str; 5] = ["no-objection", "refute", "downgrade", "stands", "skipped"];

/// Why a finding stands or was skipped: the round's own three, then the evidence check's codes as for any finding.
fn refute_reasons() -> Vec<&'static str> {
    ["no-eligible-client", "no-review", "invalid-answer"].iter().chain(MECHANICAL_REJECTIONS.iter()).chain(HINT_CODES.iter()).chain(CHECK_HINT_CODES.iter()).copied().collect()
}

/// The refute round's result for one accepted critical finding.
#[derive(Clone, Debug, PartialEq)]
pub struct RefuteRecord {
    pub finding_id: String,
    pub outcome: String,
    /// What the refuter answered, when it gave one answer the runner could read.
    pub answer: Option<String>,
    pub reason_code: Option<String>,
    /// Free text for the report; never written to the ledger.
    pub detail: Option<String>,
    /// On a downgrade that holds: the severity the finding now has.
    pub severity: Option<String>,
    /// The refuter's objection as it wrote it, for the report.
    pub reply: Option<FindingBody>,
    /// The id the refuter gave its objection, which `refute.json` keeps beside the fields the reader takes.
    pub reply_id: Option<String>,
    /// The refuter's run; absent when the round was skipped.
    pub refuter: Option<CriticRun>,
}

/// `refute.json`: the results, and why the gateway's ledger would not take a refuter's run.
#[derive(Clone, Debug, PartialEq)]
pub struct RefuteFile {
    pub results: Vec<RefuteRecord>,
    pub ledger_warnings: Option<Vec<String>>,
}

/// Reads `refute.json`; a syntax error and a wrong shape both yield the message the node runner's `decoded` prints
/// after the file's path.
pub fn parse_refute_file(text: &str) -> Result<RefuteFile, String> {
    let value = json::parse(text)?;
    let file = Fields::of(&value, "").map_err(|error| error.0)?;
    read_refute_file(&file).map_err(|error| error.0)
}

fn read_refute_file(file: &Fields<'_>) -> Result<RefuteFile, ShapeError> {
    let results = file.list("results", read_refute_record)?;
    let ledger_warnings = if file.has("ledger_warnings") { Some(file.strings("ledger_warnings")?) } else { None };
    file.string("refute_path")?;
    file.string("run_id")?;
    file.one_of("stage", &CRITIQUE_STAGES)?;
    file.literal_str("status", "checked")?;
    Ok(RefuteFile { results, ledger_warnings })
}

fn read_refute_record(fields: &Fields<'_>) -> Result<RefuteRecord, ShapeError> {
    let answer = if fields.has("answer") { Some(fields.one_of("answer", &REFUTE_ANSWERS)?) } else { None };
    let reason_code = if fields.has("reason_code") { Some(fields.one_of("reason_code", &refute_reasons())?) } else { None };
    let detail = fields.optional_string("detail")?;
    let severity = if fields.has("severity") { Some(fields.one_of("severity", &SEVERITIES)?) } else { None };
    let finding_id = fields.string("finding_id")?;
    let outcome = fields.one_of("outcome", &REFUTE_OUTCOMES)?;
    let reply = if fields.has("reply") { Some(read_finding_body(&fields.object("reply")?)?) } else { None };
    let refuter = if fields.has("refuter") { Some(read_critic_run(&fields.object("refuter")?)?) } else { None };
    Ok(RefuteRecord { finding_id, outcome, answer, reason_code, detail, severity, reply, reply_id: None, refuter })
}
