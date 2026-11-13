//! `fan-out`: runs every critic of a prepared run, each in its own copy of the packet and blind to the others, then merges
//! duplicate findings and gives each the evidence check's verdict. A critic that failed is retried once through the first
//! of its fallbacks that may run and that no critic that reviewed already has. The node runner's
//! `application/fan-out.ts`.
//!
//! The critics of a wave run one after another, in roster order, where the node runner starts them side by side; what a
//! run records is the same, but its ledger lines follow the roster rather than the order the critics finished in.

use crate::assets::read_asset;
use crate::budget_command::quota_reached;
use crate::checks::{check_named, check_runnable, declared_checks, with_declared_severity, DeclaredCheck};
use crate::critic::CriticRun;
use crate::critic_run::{read_frozen_packet, run_critic, Launch, FINDING_SCHEMA};
use crate::decode::{read_input, read_json_file};
use crate::error::{CritiqueError, ShapeError};
use crate::evidence::{check_evidence, index_packet, skipped_checks, PacketIndex};
use crate::fields::Fields;
use crate::findings::{CriticFinding, FindingBody, PreVerdict, RunFinding};
use crate::flow_budget::{FlowBudget, InvocationContext, LedgerClient, LedgerRun};
use crate::locations::{findings_path, manifest_path};
use crate::manifest::{listed_files, parse_manifest, spec_reference, RunManifest};
use crate::merge::{merge_findings, CriticFindings, Merged};
use crate::paths::join;
use crate::plan::plan_limit;
use crate::prompt::{assemble_prompt, PromptSources};
use crate::refute_round::{body_json, run_json};
use crate::roster::RosterCritic;
use crate::routing::CriticClient;
use crate::runtime::Runtime;
use crate::tripwire::ReadOnlyTripwire;
use model_gateway::json::{self, Object, Value};
use std::collections::HashMap;

/// What one critic's launch left: its findings, its run, the checks its prompt declared, and why the gateway's ledger
/// would not take the run, where it would not.
struct Attempt {
    findings: Vec<CriticFinding>,
    run: CriticRun,
    checks: Vec<DeclaredCheck>,
    ledger_warning: Option<String>,
}

/// What one wave of critics shares: the run, its tripwire and its budget.
struct Wave<'a> {
    manifest: &'a RunManifest,
    tripwire: ReadOnlyTripwire,
}

fn read(raw: &Value) -> Result<String, ShapeError> {
    Fields::of(raw, "")?.string("run_dir")
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let run_dir = read_input("fan-out", read(raw))?;
    let manifest = read_json_file(runtime, &manifest_path(&run_dir), "invalid-run", parse_manifest)?;
    // A work client sees nedap code only: a run of any other repository that names one is refused, whatever its manifest says.
    if manifest.work.is_none() {
        if let Some(stray) = manifest.critics.iter().find(|critic| matches!(critic.client_config, CriticClient::Codex(_) | CriticClient::Claude(_))) {
            return Err(CritiqueError::new(
                "invalid-run",
                format!("critic {} runs on the work client {}, which only a nedap repository's run may use", stray.critic_id, stray.client),
            ));
        }
    }
    let tripwire = ReadOnlyTripwire::load(runtime, &manifest)?;
    let packet = index_packet(&read_frozen_packet(runtime, &manifest)?, manifest.ticket_check.as_ref().map_or(&[][..], |check| &check.ticket_ids));
    let wave = Wave { manifest: &manifest, tripwire };
    let mut outcomes = Vec::new();
    for critic in &manifest.critics {
        outcomes.push(attempt(runtime, &wave, critic, None)?);
    }
    let retries = retries(runtime, &wave, &outcomes)?;
    outcomes.extend(retries);
    let merged = merge_findings(
        &outcomes.iter().map(|outcome| CriticFindings { critic_id: outcome.run.critic_id.clone(), findings: outcome.findings.clone() }).collect::<Vec<_>>(),
    );
    let raisers: HashMap<&str, (&str, &[DeclaredCheck])> =
        outcomes.iter().map(|outcome| (outcome.run.critic_id.as_str(), (outcome.run.role.as_str(), outcome.checks.as_slice()))).collect();
    let findings: Vec<Merged> = merged.into_iter().map(|finding| with_pre_verdict(finding, &manifest.stage, &packet, &raisers)).collect();
    let ledger_warnings: Vec<&String> = outcomes.iter().filter_map(|outcome| outcome.ledger_warning.as_ref()).collect();
    let skipped = skipped_checks(&manifest.stage, packet.tree, &check_sources(runtime, &manifest)?);
    let path = findings_path(&run_dir);
    let mut result = Object::new();
    result.set("critics", Value::Arr(outcomes.iter().map(|outcome| run_json(&outcome.run)).collect()));
    result.set("degraded", Value::Bool(outcomes.iter().all(|outcome| outcome.run.status != "ok")));
    result.set("findings", Value::Arr(findings.iter().map(finding_json).collect()));
    result.set("findings_path", Value::str(&path));
    if !ledger_warnings.is_empty() {
        result.set("ledger_warnings", Value::Arr(ledger_warnings.iter().map(|warning| Value::str(warning)).collect()));
    }
    result.set("run_id", Value::str(&manifest.run_id));
    if !skipped.is_empty() {
        let entry = |check: &crate::fan_out_result::SkippedCheck| {
            let mut object = Object::new();
            if let Some(about) = &check.about {
                object.set("about", Value::str(about));
            }
            object.set("check", Value::str(&check.check));
            object.set("from", Value::str(&check.from));
            Value::Obj(object)
        };
        result.set("skipped_checks", Value::Arr(skipped.iter().map(entry).collect()));
    }
    result.set("stage", Value::str(&manifest.stage));
    result.set("status", Value::str("reviewed"));
    let result = Value::Obj(result);
    runtime.files.write_text(&path, &format!("{}\n", json::pretty(&result))).map_err(CritiqueError::from_fs)?;
    Ok(result)
}

/// A finding is judged against the prompt of the critic that raised it first, whose check a merged finding keeps: that
/// check's declared severity, if it declares one, replaces the critic's. The stage's evidence check speaks first; only a
/// finding it leaves to the orchestrator is checked for its check id.
fn with_pre_verdict(mut merged: Merged, stage: &str, packet: &PacketIndex, raisers: &HashMap<&str, (&str, &[DeclaredCheck])>) -> Merged {
    let raiser = merged.finding.raised_by.first().and_then(|id| raisers.get(id.as_str()));
    let declared = raiser.map_or(&[][..], |(_, checks)| *checks);
    let rated = with_declared_severity(&merged.finding, declared);
    let check = rated.body.check.as_deref();
    let verdict: Option<PreVerdict> = check_evidence(&rated.body, stage, packet, raiser.map(|(role, _)| *role))
        .or_else(|| check_named(check, declared))
        .or_else(|| check_runnable(check, declared, packet.tree));
    merged.finding = RunFinding { pre_verdict: verdict, ..rated };
    merged
}

/// A merged finding as `findings.json` holds it: the first critic's fields, who raised it, its number, then what fan-out
/// added, in the order the node runner builds the object.
pub(crate) fn finding_json(merged: &Merged) -> Value {
    let finding = &merged.finding;
    let body: &FindingBody = &finding.body;
    let Value::Obj(mut object) = Value::Obj(body_json(&FindingBody { suggestion: None, ..body.clone() })) else { unreachable!("a body is an object") };
    let early = body.suggestion.as_ref().filter(|_| !merged.late_suggestion);
    if let Some(suggestion) = early {
        object.set("suggestion", Value::str(suggestion));
    }
    object.set("critic_finding_id", Value::str(&finding.critic_finding_id));
    object.set("raised_by", Value::Arr(finding.raised_by.iter().map(|id| Value::str(id)).collect()));
    if let Some(suggestion) = body.suggestion.as_ref().filter(|_| merged.late_suggestion) {
        object.set("suggestion", Value::str(suggestion));
    }
    object.set("id", Value::str(&finding.id));
    if let Some(declared) = &finding.declared_severity {
        object.set("declared_severity", Value::str(declared));
    }
    if let Some(pre) = &finding.pre_verdict {
        let mut verdict = Object::new();
        verdict.set("reason", Value::str(&pre.reason));
        verdict.set("reason_code", Value::str(&pre.reason_code));
        verdict.set("verdict", Value::str(&pre.verdict));
        object.set("pre_verdict", Value::Obj(verdict));
    }
    Value::Obj(object)
}

/// Once the first wave has ended, retries every failed critic that has a fallback left, with today's agy count and
/// Vibe's plan read again, since the first wave (and any other session on this machine) has added to them.
fn retries(runtime: &Runtime, wave: &Wave<'_>, first: &[Attempt]) -> Result<Vec<Attempt>, CritiqueError> {
    let manifest = wave.manifest;
    let agy = quota_reached(FlowBudget::new(runtime).agy_invocations_today()?, &manifest.limits.antigravity_quota, &manifest.stage).is_some();
    let vibe = plan_limit(runtime, "vibe")?.unable_to_start.is_some();
    let mut attempts = Vec::new();
    let runs: Vec<&CriticRun> = first.iter().map(|attempt| &attempt.run).collect();
    for (critic, retry_of) in fallback_retries(manifest, &runs, (agy, vibe)) {
        attempts.push(attempt(runtime, wave, &critic, Some(retry_of))?);
    }
    Ok(attempts)
}

/// One retry per failed critic, through the first of its fallbacks whose family no critic that reviewed, and no earlier
/// retry, already has (when the stage requires distinct families). A critic that violated read-only gets none: a client
/// that wrote once may write again. A retry runs with the failed critic's role, lenses, cap and time limit, under its own
/// critic id, so it has its own packet copy and tripwire, and it has no fallbacks of its own. `blocked` says whether agy
/// is past its quota and whether Vibe is at its plan limit.
pub(crate) fn fallback_retries(manifest: &RunManifest, first: &[&CriticRun], blocked: (bool, bool)) -> Vec<(RosterCritic, String)> {
    let mut taken: Vec<String> = first.iter().filter(|run| run.status == "ok").map(|run| run.family.clone()).collect();
    let mut retries = Vec::new();
    for (critic, outcome) in manifest.critics.iter().zip(first) {
        if outcome.status != "failed" {
            continue;
        }
        let fallback = critic.fallbacks.iter().find(|candidate| {
            let held = match candidate.client_config {
                CriticClient::Agy(_) => blocked.0,
                CriticClient::Vibe(_) => blocked.1,
                _ => false,
            };
            !held && (!manifest.distinct_families || !taken.iter().any(|family| family == candidate.client_config.family()))
        });
        let Some(fallback) = fallback else { continue };
        taken.push(fallback.client_config.family().to_string());
        retries.push((
            RosterCritic {
                client: fallback.client.clone(),
                client_config: fallback.client_config.clone(),
                critic_id: format!("{}-retry-{}", critic.critic_id, fallback.client),
                fallbacks: Vec::new(),
                ..critic.clone()
            },
            critic.critic_id.clone(),
        ));
    }
    retries
}

/// Launches one critic in its own copy of the packet, and records the launch in both ledgers.
fn attempt(runtime: &Runtime, wave: &Wave<'_>, critic: &RosterCritic, retry_of: Option<String>) -> Result<Attempt, CritiqueError> {
    let manifest = wave.manifest;
    let critic_dir = join(&[&manifest.run_dir, "critics", &critic.critic_id]);
    let role = read_asset(runtime, &format!("roles/{}.md", critic.role))?;
    let mut lenses = Vec::new();
    for name in &critic.lenses {
        lenses.push((name.clone(), read_asset(runtime, &format!("lenses/{name}.md"))?));
    }
    let sources = PromptSources {
        contract: read_asset(runtime, "prompts/output-contract.md")?,
        lenses,
        preamble: read_asset(runtime, "prompts/preamble.md")?,
        role,
        schema: read_asset(runtime, FINDING_SCHEMA)?,
    };
    let texts: Vec<&str> = std::iter::once(sources.role.as_str()).chain(sources.lenses.iter().map(|(_, text)| text.as_str())).collect();
    let checks = declared_checks(&texts);
    let prompt = |toolset: &_| {
        let slices = match &manifest.code { Some(code) => code.slices.as_deref().unwrap_or(&[]), None => &[] };
        assemble_prompt(&sources, &listed_files(manifest), &manifest.stage, slices, toolset)
    };
    let launched = run_critic(runtime, &Launch { tripwire: &wave.tripwire, critic, critic_dir: &critic_dir, prompt: &prompt, retry_of })?;
    let context = InvocationContext {
        flow: manifest.flow.clone(),
        repository: manifest.repository.clone(),
        round: manifest.round,
        run_id: manifest.run_id.clone(),
        stage: manifest.stage.clone(),
    };
    let ledger_warning = FlowBudget::new(runtime).log_invocation(&context, &LedgerRun::of(&launched.run), &LedgerClient::of(&critic.client_config), Some(&spec_reference(manifest)))?;
    Ok(Attempt { findings: launched.findings, run: launched.run, checks, ledger_warning })
}

/// The checks each role and lens file of the run's critics declares, by file, in roster order: the files whose checks the
/// critics' prompts declare, so the skipped list and the needs-tree hint agree.
fn check_sources(runtime: &Runtime, manifest: &RunManifest) -> Result<Vec<(String, Vec<DeclaredCheck>)>, CritiqueError> {
    let mut files: Vec<String> = Vec::new();
    for critic in &manifest.critics {
        for file in std::iter::once(format!("roles/{}", critic.role)).chain(critic.lenses.iter().map(|name| format!("lenses/{name}"))) {
            if !files.contains(&file) {
                files.push(file);
            }
        }
    }
    files
        .iter()
        .map(|file| {
            let text = read_asset(runtime, &format!("{file}.md"))?;
            let from = match file.strip_prefix("roles/") {
                Some(role) => format!("role {role}"),
                None => format!("lens {}", file.strip_prefix("lenses/").unwrap_or(file)),
            };
            Ok((from, declared_checks(&[&text])))
        })
        .collect()
}
