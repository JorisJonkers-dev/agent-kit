//! What a client hands back: one question put to a model, the answer it gave, and whether a client can run at all.
//! The node runner's `application/ports.ts` and `domain/routing.ts`.

use crate::error::CritiqueError;
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

/// Asks the client of `config`'s kind whether it can run. Only Vibe has an adapter so far: Antigravity's, Codex's and
/// Claude's arrive with the commands that need them, so a machine that wires one of those has no work critic, and
/// prepare drops it as unavailable.
pub fn client_readiness(runtime: &Runtime, config: &CriticClient) -> Result<ClientReadiness, CritiqueError> {
    match config {
        CriticClient::Vibe(vibe) => crate::vibe_client::vibe_readiness(runtime, vibe),
        CriticClient::Agy(_) => Ok(unwired("agy")),
        CriticClient::Codex(_) => Ok(unwired("codex")),
        CriticClient::Claude(_) => Ok(unwired("claude")),
    }
}

/// Why a work critic cannot run on a machine that wires no client for its kind.
fn unwired(kind: &str) -> ClientReadiness {
    ClientReadiness { ready: false, detail: Some(format!("no {kind} critic client is wired on this machine")) }
}
