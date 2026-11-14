//! What a client hands back: one question put to a model, the answer it gave, and whether a client can run at all.
//! The node runner's `application/ports.ts` and `domain/routing.ts`.

use crate::critic_tools::{CriticToolset, ToolCall};
use crate::error::CritiqueError;
use crate::findings::CriticFinding;
use crate::roster::RosterCritic;
use crate::routing::CriticClient;
use crate::runtime::Runtime;

/// One question put to a model in a working directory of its own: a yes/no question such as triage asks, never a
/// review, so its answer is plain text rather than findings.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelQuestion {
    /// The model's working directory, and all it may read.
    pub work_dir: String,
    /// The question, written outside the working directory.
    pub prompt_path: String,
    /// Where the client writes what it keeps of the run.
    pub output_dir: String,
    pub max_price_usd: f64,
    pub time_limit_s: f64,
}

/// The model's final message, or why there is none.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelAnswer {
    /// The model answered: its final message as plain text, and what the run cost.
    Answered { text: String, cost_usd: Option<f64> },
    /// There is no answer: why, and what the run cost.
    Failed { failure: String, detail: String, cost_usd: Option<f64> },
}

/// What one run used, as its client reported it; cached input is prompt the provider served from its cache.
#[derive(Clone, Debug, PartialEq)]
pub struct CriticTokens {
    pub cached: f64,
    pub input: f64,
    pub output: f64,
}

/// One question's run: what it printed, the answer it gave, and what it cost.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelAnswerReport {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub answer: ModelAnswer,
    /// What the run counts against the flow's Vibe cap, as for a critic.
    pub charged_usd: f64,
    /// What the run used, when its export counts any.
    pub tokens: Option<CriticTokens>,
    /// What Vibe itself reported, as for a critic.
    pub vibe_cost_usd: Option<f64>,
}

/// Whether a client can run here, and why not when it cannot.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientReadiness {
    pub ready: bool,
    pub detail: Option<String>,
}

/// What a critic client makes of one critic's run: a review, or the reason it counts as none.
#[derive(Clone, Debug, PartialEq)]
pub enum CriticOutcome {
    Ok { findings: Vec<CriticFinding>, no_findings_reason: Option<String>, cost_usd: Option<f64> },
    Failed { failure: String, detail: String, cost_usd: Option<f64> },
}

/// Where one critic of a run works.
#[derive(Clone, Debug, PartialEq)]
pub struct CriticLaunch<'a> {
    pub critic: &'a RosterCritic,
    /// The critic's own copy of the packet: its working directory, and all it may read.
    pub packet_dir: String,
    /// The assembled prompt, written outside the packet.
    pub prompt_path: String,
    /// The critic's own directory in the run, for whatever its client writes there.
    pub critic_dir: String,
    /// The finding schema, for a client that can enforce it.
    pub schema_path: String,
    /// The tools the critic has, which its prompt names.
    pub toolset: CriticToolset,
}

/// A critic's run as its client saw it: the raw output, and what that output amounts to.
#[derive(Clone, Debug, PartialEq)]
pub struct CriticReport {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// The file the client read cost and effective config from.
    pub export_path: String,
    pub outcome: CriticOutcome,
    /// What the run counts against the flow's Vibe cap.
    pub charged_usd: f64,
    /// What Vibe itself reported, kept beside the cost the runner computed; absent for any other client.
    pub vibe_cost_usd: Option<Option<f64>>,
    /// What the run used, as its client reported it; absent when the client reports none.
    pub tokens: Option<CriticTokens>,
    /// Why a searching critic's searches failed, as its journal shows them; none when it searched without a failure or did not search.
    pub search_failure: Option<String>,
    /// Every tool call the critic's journal records; none for a client that keeps no journal.
    pub tool_calls: Vec<ToolCall>,
}

/// Asks the client of `config`'s kind whether it can run.
pub fn client_readiness(runtime: &Runtime, config: &CriticClient) -> Result<ClientReadiness, CritiqueError> {
    match config {
        CriticClient::Vibe(vibe) => crate::vibe_client::vibe_readiness(runtime, vibe),
        CriticClient::Agy(agy) => crate::agy_client::agy_readiness(runtime, agy),
        CriticClient::Codex(codex) => crate::work_client::codex_sign_in(runtime, codex),
        CriticClient::Claude(claude) => crate::work_client::claude_sign_in(runtime, claude),
        CriticClient::Pi(pi) => crate::pi_client::pi_readiness(runtime, pi),
    }
}

/// Runs the launch's critic through the client of its kind.
pub fn review_with(runtime: &Runtime, launch: &CriticLaunch<'_>) -> Result<CriticReport, CritiqueError> {
    match &launch.critic.client_config {
        CriticClient::Vibe(vibe) => crate::vibe_client::vibe_review(runtime, launch, vibe),
        CriticClient::Agy(agy) => crate::agy_client::agy_review(runtime, launch, agy),
        CriticClient::Codex(codex) => crate::work_client::codex_review(runtime, launch, codex),
        CriticClient::Claude(claude) => crate::work_client::claude_review(runtime, launch, claude),
        CriticClient::Pi(pi) => crate::pi_client::pi_review(runtime, launch, pi),
    }
}

/// Asks the client of `config`'s kind whether its CLI is installed and signed in.
pub fn client_sign_in(runtime: &Runtime, config: &CriticClient) -> Result<ClientReadiness, CritiqueError> {
    match config {
        CriticClient::Vibe(vibe) => crate::vibe_client::vibe_sign_in(runtime, vibe),
        // agy has no per-critic file, so being signed in is all a critic needs.
        CriticClient::Agy(agy) => crate::agy_client::agy_readiness(runtime, agy),
        CriticClient::Codex(codex) => crate::work_client::codex_sign_in(runtime, codex),
        CriticClient::Claude(claude) => crate::work_client::claude_sign_in(runtime, claude),
        CriticClient::Pi(pi) => crate::pi_client::pi_sign_in(runtime, pi),
    }
}

/// The CLI a critic client runs through.
pub fn client_binary(config: &CriticClient) -> &str {
    match config {
        CriticClient::Vibe(client) => &client.binary,
        CriticClient::Agy(client) => &client.binary,
        CriticClient::Codex(client) => &client.binary,
        CriticClient::Claude(client) => &client.binary,
        CriticClient::Pi(client) => &client.binary,
    }
}

/// How long `<binary> --version` may take.
const VERSION_TIMEOUT_MS: u64 = 10_000;

/// The first line `<binary> --version` prints, or none when the CLI is missing or fails. Free: no model is called.
pub fn client_version(runtime: &Runtime, config: &CriticClient) -> Option<String> {
    let result = runtime.gateway.process.exec(client_binary(config), &["--version"], Some(VERSION_TIMEOUT_MS)).ok()?;
    if result.timed_out || result.exit_code != 0 {
        return None;
    }
    let line = result.stdout.split('\n').next().map_or("", model_gateway::text::js_trim);
    (!line.is_empty()).then(|| line.to_string())
}
