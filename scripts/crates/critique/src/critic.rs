//! One critic's run as fan-out recorded it: who ran, how it went, and where its raw output is. The node runner's
//! `domain/critic.ts`.

use crate::clients::CriticTokens;
use crate::error::ShapeError;
use crate::family::CRITIC_FAMILIES;
use crate::fields::Fields;
use crate::flow_budget::PromptHashes;

/// The statuses a critic run can end in.
pub const CRITIC_STATUSES: [&str; 4] = ["ok", "failed", "violated-readonly", "budget-skip"];

/// Why a critic's run counts as no review, besides `exit-<code>`: the findings failures, then the run's own.
const RUN_FAILURES: [&str; 16] = [
    "no-findings-block",
    "unparseable-findings",
    "invalid-findings",
    "missing-no-findings-reason",
    "empty-output",
    "unparseable-output",
    "missing-export",
    "unsafe-config",
    "config-error",
    "infrastructure-failure",
    "stopped-early",
    "print-timeout",
    "denied-action",
    "missing-structured-output",
    "unsafe-tool",
    "timeout",
];

/// One critic's run as fan-out recorded it.
#[derive(Clone, Debug, PartialEq)]
pub struct CriticRun {
    pub critic_id: String,
    pub client: String,
    pub family: String,
    pub model: String,
    pub role: String,
    pub lenses: Vec<String>,
    /// The critic id of the failed critic this one is the fallback retry for.
    pub retry_of: Option<String>,
    pub status: String,
    pub failure: Option<String>,
    pub detail: Option<String>,
    pub no_findings_reason: Option<String>,
    /// Null for a critic that was never launched.
    pub exit_code: Option<f64>,
    /// What the critic cost; null when nothing is known.
    pub cost_usd: Option<f64>,
    /// What Vibe itself reported, kept for comparison: absent for any other client, null when it reported none.
    pub vibe_cost_usd: Option<Option<f64>>,
    pub tokens: Option<CriticTokens>,
    /// What the launch counts against the flow's Vibe cap.
    pub charged_usd: f64,
    pub duration_ms: f64,
    pub finding_count: f64,
    pub prompt_hashes: PromptHashes,
    pub lens_hashes: Vec<(String, String)>,
    pub prompt_path: String,
    pub stdout_path: Option<String>,
    pub export_path: Option<String>,
    /// The tools the critic had, as its prompt named them.
    pub tools: Option<Vec<String>>,
    /// Why the critic could not search, or why its searches failed during the run.
    pub search_failure: Option<String>,
}

/// A failure a critic run may carry: one of the named ones, or `exit-` and digits (`/^exit-\d+$/`).
fn is_critic_failure(value: &str) -> bool {
    RUN_FAILURES.contains(&value)
        || value.strip_prefix("exit-").is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

fn read_tokens(fields: &Fields<'_>) -> Result<CriticTokens, ShapeError> {
    Ok(CriticTokens { cached: fields.number("cached")?, input: fields.number("input")?, output: fields.number("output")? })
}

/// One critic run, in the order the node reader evaluates its fields, so the first wrong field is the one reported.
pub fn read_critic_run(fields: &Fields<'_>) -> Result<CriticRun, ShapeError> {
    let failure = fields.optional_matching("failure", is_critic_failure, "a critic failure")?;
    let detail = fields.optional_string("detail")?;
    let no_findings_reason = fields.optional_string("no_findings_reason")?;
    let retry_of = fields.optional_string("retry_of")?;
    let stdout_path = fields.optional_string("stdout_path")?;
    let export_path = fields.optional_string("export_path")?;
    let vibe_cost_usd = if fields.has("vibe_cost_usd") { Some(fields.nullable_number("vibe_cost_usd")?) } else { None };
    let tokens = if fields.has("tokens") { Some(read_tokens(&fields.object("tokens")?)?) } else { None };
    let search_failure = fields.optional_string("search_failure")?;
    let hashes = fields.object("prompt_hashes")?;
    let charged_usd = fields.number("charged_usd")?;
    let client = fields.string("client")?;
    let cost_usd = fields.nullable_number("cost_usd")?;
    let critic_id = fields.string("critic_id")?;
    let duration_ms = fields.number("duration_ms")?;
    let exit_code = fields.nullable_number("exit_code")?;
    let family = fields.one_of("family", CRITIC_FAMILIES)?;
    let finding_count = fields.number("finding_count")?;
    let lens_hashes = fields.string_map("lens_hashes")?;
    let lenses = fields.strings("lenses")?;
    let model = fields.string("model")?;
    let prompt_hashes = PromptHashes {
        contract: hashes.string("contract")?,
        preamble: hashes.string("preamble")?,
        role: hashes.string("role")?,
        schema: hashes.string("schema")?,
    };
    let prompt_path = fields.string("prompt_path")?;
    let role = fields.string("role")?;
    let status = fields.one_of("status", &CRITIC_STATUSES)?;
    let tools = if fields.has("tools") { Some(fields.strings("tools")?) } else { None };
    Ok(CriticRun {
        critic_id,
        client,
        family,
        model,
        role,
        lenses,
        retry_of,
        status,
        failure,
        detail,
        no_findings_reason,
        exit_code,
        cost_usd,
        vibe_cost_usd,
        tokens,
        charged_usd,
        duration_ms,
        finding_count,
        prompt_hashes,
        lens_hashes,
        prompt_path,
        stdout_path,
        export_path,
        tools,
        search_failure,
    })
}
