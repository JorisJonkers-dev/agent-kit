//! One critic's launch: its own copy of the packet under the tripwire, its tools, its prompt and the client of its kind.
//! The node runner's `application/critic-run.ts` and the tool choice of `application/critic-tools.ts`.

use crate::clients::{review_with, CriticLaunch, CriticOutcome};
use crate::critic_tools::{tool_names, CriticToolset, READ_ONLY_TOOLSET};
use crate::error::CritiqueError;
use crate::evidence::PacketText;
use crate::findings::CriticFinding;
use crate::locations::setup_results_path;
use crate::manifest::RunManifest;
use crate::paths::join;
use crate::prompt::CriticPrompt;
use crate::roster::RosterCritic;
use crate::routing::CriticClient;
use crate::packet_search::SearchLimits;
use crate::runtime::Runtime;
use crate::search_client::{prove_search_reach, server_launch, session, stop, SearchSession};
use crate::vibe_search::search_blocked;
use std::sync::{Mutex, PoisonError};
use crate::session_start::setup_results;
use crate::tripwire::ReadOnlyTripwire;
use crate::critic::CriticRun;

/// The finding schema every critic answers in, under the shipped assets.
pub const FINDING_SCHEMA: &str = "schemas/finding.schema.json";

/// What one critic launch gave: its findings (none unless its review was usable) and its run.
#[derive(Clone, Debug, PartialEq)]
pub struct CriticLaunchResult {
    pub findings: Vec<CriticFinding>,
    pub run: CriticRun,
}

/// The packet as prepare froze it, read before any critic runs, so no critic can plant a quote.
pub fn read_frozen_packet(runtime: &Runtime, manifest: &RunManifest) -> Result<Vec<PacketText>, CritiqueError> {
    manifest
        .packet_files
        .iter()
        .map(|file| {
            let text = runtime.files.read_text(&join(&[&manifest.packet_dir, &file.path])).map_err(CritiqueError::from_fs)?;
            Ok(PacketText { kind: file.kind.clone(), path: file.path.clone(), text, window: file.window.clone() })
        })
        .collect()
}

/// The tools one critic gets, and why it reads alone when it could have searched.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolChoice {
    pub toolset: CriticToolset,
    pub search_failure: Option<String>,
}

/// Chooses each critic's tools for one command (ADR-0006). An agy critic uses agy's own read-only tools. A Vibe critic
/// searches through the runner's search server only when its installed agent file lets it, the server proved, once per
/// command, that it cannot see past a packet, and the server then started on the critic's own copy; otherwise it reads
/// alone, and the run says why. All of this is settled before the critic is launched, so no critic spends on a search
/// it cannot have.
pub struct CriticTools {
    limits: SearchLimits,
    /// Where the reach proof writes its packet, outside every critic's copy.
    proof_dir: String,
    /// The proof's answer, kept once made: none when proven, else why not.
    proof: Mutex<Option<Option<String>>>,
}

impl CriticTools {
    pub fn new(limits: SearchLimits, proof_dir: String) -> CriticTools {
        CriticTools { limits, proof_dir, proof: Mutex::new(None) }
    }

    /// agy's search tools are its own, so only agy's read-only proof can show they stay inside the packet, and that proof
    /// is #178's live check. Until setup records it passing for this client, an agy critic reads alone.
    fn agy(runtime: &Runtime, critic: &RosterCritic) -> Result<ToolChoice, CritiqueError> {
        let path = setup_results_path(&runtime.env)?;
        let results = if runtime.files.exists(&path).unwrap_or(false) {
            setup_results(&runtime.files.read_text(&path).map_err(CritiqueError::from_fs)?).map_err(|error| CritiqueError::unexpected(error.0))?
        } else {
            Vec::new()
        };
        let proven = results.iter().any(|(check, result)| *check == format!("read-only:{}", critic.client) && result == "pass");
        if proven {
            return Ok(ToolChoice { toolset: CriticToolset::Agy { search: true }, search_failure: None });
        }
        Ok(ToolChoice {
            toolset: CriticToolset::Agy { search: false },
            search_failure: Some(format!(
                "agy-search-unproven: {}'s read-only proof, which must show its search cannot reach past the packet, waits for agent-kit#178",
                critic.client
            )),
        })
    }

    /// The reach proof, made once: a proof that could not be made (a symlink the file system refuses, say) is a proof
    /// that failed, and is not kept, so the next critic tries it again.
    fn proven(&self, runtime: &Runtime) -> Option<String> {
        let mut kept = self.proof.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(answer) = kept.as_ref() {
            return answer.clone();
        }
        match prove_search_reach(runtime, &self.proof_dir, &self.limits) {
            Ok(answer) => {
                *kept = Some(answer.clone());
                answer
            }
            Err(message) => Some(message),
        }
    }

    pub fn choose(&self, runtime: &Runtime, critic: &RosterCritic, packet_dir: &str) -> Result<ToolChoice, CritiqueError> {
        let config = match &critic.client_config {
            CriticClient::Agy(_) => return CriticTools::agy(runtime, critic),
            // Pi searches with its own read-only tools, checked against the packet after the run.
            CriticClient::Pi(_) => return Ok(ToolChoice { toolset: CriticToolset::Pi, search_failure: None }),
            CriticClient::Vibe(config) => config,
            // Codex and the work Claude agent read the packet alone: the runner's search server is a Vibe client's.
            _ => return Ok(ToolChoice { toolset: READ_ONLY_TOOLSET, search_failure: None }),
        };
        let Some(own) = &runtime.search else {
            return Ok(ToolChoice { toolset: READ_ONLY_TOOLSET, search_failure: None });
        };
        let alone = |failure: String| Ok(ToolChoice { toolset: READ_ONLY_TOOLSET, search_failure: Some(failure) });
        if let Some(blocked) = search_blocked(runtime, &config.agent)? {
            return alone(blocked);
        }
        if let Some(unproven) = self.proven(runtime) {
            return alone(format!("search is not proven to stay inside the packet: {unproven}"));
        }
        match session(runtime, own, packet_dir, &self.limits, &[]) {
            SearchSession::Failed { detail } => alone(detail),
            SearchSession::Started { tools, .. } => Ok(ToolChoice {
                toolset: CriticToolset::VibeSearch { search: tools, limits: self.limits.clone(), server: server_launch(own, packet_dir, &self.limits) },
                search_failure: None,
            }),
        }
    }
}

/// How one critic is launched.
pub struct Launch<'a> {
    pub tripwire: &'a ReadOnlyTripwire,
    pub critic: &'a RosterCritic,
    pub critic_dir: &'a str,
    /// The critic's prompt, for the tools it has.
    pub prompt: &'a dyn Fn(&CriticToolset) -> CriticPrompt,
    pub retry_of: Option<String>,
    pub tools: &'a CriticTools,
}

/// Runs one critic in its own copy of the packet, under the tripwire, through the client of its kind, and keeps its raw
/// output in the critic's directory. Its tools are chosen once its copy exists, and its prompt, built for those tools, is
/// written before it starts.
pub fn run_critic(runtime: &Runtime, launch: &Launch<'_>) -> Result<CriticLaunchResult, CritiqueError> {
    let (critic, critic_dir) = (launch.critic, launch.critic_dir);
    let stdout_path = join(&[critic_dir, "stdout.json"]);
    let packet_dir = join(&[critic_dir, "packet"]);
    let armed = launch.tripwire.arm(runtime, &packet_dir)?;
    let choice = launch.tools.choose(runtime, critic, &packet_dir)?;
    let prompt = (launch.prompt)(&choice.toolset);
    let prompt_path = join(&[critic_dir, "prompt.md"]);
    runtime.files.write_text(&prompt_path, &prompt.text).map_err(CritiqueError::from_fs)?;
    let started = runtime.clock.monotonic_ms();
    let report = review_with(
        runtime,
        &CriticLaunch {
            critic,
            packet_dir: packet_dir.clone(),
            prompt_path: prompt_path.clone(),
            critic_dir: critic_dir.to_string(),
            schema_path: join(&[&runtime.assets_root, FINDING_SCHEMA]),
            toolset: choice.toolset.clone(),
        },
    );
    // The server stops with the critic, whatever came of the launch.
    if matches!(choice.toolset, CriticToolset::VibeSearch { .. }) {
        stop(runtime, &packet_dir);
    }
    let report = report?;
    let duration_ms = runtime.clock.monotonic_ms() - started;
    let violations = launch.tripwire.check(runtime, &armed)?;
    runtime.files.write_text(&stdout_path, &report.stdout).map_err(CritiqueError::from_fs)?;
    runtime.files.write_text(&join(&[critic_dir, "stderr.txt"]), &report.stderr).map_err(CritiqueError::from_fs)?;
    let (findings, cost_usd) = match &report.outcome {
        CriticOutcome::Ok { findings, cost_usd, .. } if violations.is_empty() => (findings.clone(), *cost_usd),
        CriticOutcome::Ok { cost_usd, .. } | CriticOutcome::Failed { cost_usd, .. } => (Vec::new(), *cost_usd),
    };
    let (status, failure, detail, no_findings_reason) = status_of(&report.outcome, &violations);
    let run = CriticRun {
        critic_id: critic.critic_id.clone(),
        client: critic.client.clone(),
        family: critic.client_config.family().to_string(),
        model: critic.client_config.model().to_string(),
        role: critic.role.clone(),
        lenses: critic.lenses.clone(),
        retry_of: launch.retry_of.clone(),
        status,
        failure,
        detail,
        no_findings_reason,
        exit_code: Some(f64::from(report.exit_code)),
        cost_usd,
        vibe_cost_usd: report.vibe_cost_usd,
        tokens: report.tokens.clone(),
        charged_usd: report.charged_usd,
        duration_ms,
        finding_count: findings.len() as f64,
        prompt_hashes: prompt.prompt_hashes.clone(),
        lens_hashes: prompt.lens_hashes.clone(),
        prompt_path,
        stdout_path: Some(stdout_path),
        export_path: Some(report.export_path.clone()),
        tools: Some(tool_names(&choice.toolset)),
        search_failure: choice.search_failure.clone().or_else(|| report.search_failure.clone()),
    };
    Ok(CriticLaunchResult { findings, run })
}

/// A critic that changed what it may only read gave no review, whatever its output says, so the read-only violation
/// outranks every other outcome.
fn status_of(outcome: &CriticOutcome, violations: &[String]) -> (String, Option<String>, Option<String>, Option<String>) {
    if !violations.is_empty() {
        return ("violated-readonly".to_string(), None, Some(format!("read-only violated: {}", violations.join("; "))), None);
    }
    match outcome {
        CriticOutcome::Failed { failure, detail, .. } => ("failed".to_string(), Some(failure.clone()), Some(detail.clone()), None),
        CriticOutcome::Ok { no_findings_reason, .. } => ("ok".to_string(), None, None, no_findings_reason.clone()),
    }
}
