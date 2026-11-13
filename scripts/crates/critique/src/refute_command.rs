//! `refute`: the refute round (CONTEXT.md). Takes a run directory and the orchestrator's verdicts, after fan-out and
//! before record. Each accepted critical finding (critical as the verdict's severity override has it, else as fan-out
//! fixed it) goes to the first routed client, base before deep, whose family neither raised it nor is the orchestrator's,
//! run with the refuter role on the original packet plus the finding. Its answer counts only with evidence the packet
//! holds; otherwise the verdict stands. With no eligible client the finding is skipped. Results go to `refute.json`,
//! which record applies, and one line each to the ledger; a refuter's launch is logged like any critic's, so its spend
//! counts in the run's flow cap. A run is refuted once, and only before it is recorded. The node runner's
//! `application/refute.ts`.

use crate::assets::read_asset;
use crate::budget_command::quota_reached;
use crate::checks::declared_checks;
use crate::clients::client_readiness;
use crate::critic_run::{read_frozen_packet, run_critic, CriticTools, Launch, FINDING_SCHEMA};
use crate::decode::{read_input, read_json_file};
use crate::error::{CritiqueError, ShapeError};
use crate::evidence::{index_packet, PacketIndex};
use crate::fan_out_result::{parse_fan_out_result, FanOutResult};
use crate::fields::Fields;
use crate::findings::RunFinding;
use crate::flow_budget::{FlowBudget, InvocationContext, LedgerClient, LedgerRun};
use crate::locations::{findings_path, ledger_path, manifest_path, refute_path, report_path};
use crate::manifest::{listed_files, parse_manifest, spec_reference, RunManifest};
use crate::paths::join;
use crate::plan::plan_limit;
use crate::prompt::{assemble_prompt, PromptSources};
use crate::refute::RefuteRecord;
use crate::refute_round::{
    choose_refuter, count_outcomes, family_bar, judge_reply, record_json, refute_ledger_line, refuter_candidates, skipped, with_finding,
    JudgeContext, RefuteLedgerContext, RefuterCandidate, RefuterReply, RefuterRules,
};
use crate::routing::CriticClient;
use crate::routing_source::load_routing;
use crate::runtime::Runtime;
use crate::tripwire::ReadOnlyTripwire;
use crate::verdicts::{read_verdict, severity_override_problem, Verdict};
use crate::adjudication::verdicts_by_finding;
use model_gateway::grant_guard::GrantGuard;
use model_gateway::json::{self, Object, Value};
use model_gateway::text::dirname;
use model_gateway::timestamps::to_iso;
use std::collections::{HashMap, HashSet};

struct Input {
    run_dir: String,
    verdicts: Vec<Verdict>,
}

fn read(raw: &Value) -> Result<Input, ShapeError> {
    let fields = Fields::of(raw, "")?;
    Ok(Input { run_dir: fields.string("run_dir")?, verdicts: fields.list("verdicts", read_verdict)? })
}

/// What every refuter of one round shares: the run, its packet and tripwire, and the refuter role's prompt.
struct Round<'a> {
    manifest: &'a RunManifest,
    merged: &'a FanOutResult,
    tripwire: ReadOnlyTripwire,
    tools: CriticTools,
    packet: PacketIndex,
    candidates: Vec<RefuterCandidate>,
    /// Why each candidate client may not run now, by client name; absent while it may.
    blocked: HashMap<String, String>,
    sources: PromptSources,
    /// The check ids the refuter role declares: the answers that object.
    declared: Vec<String>,
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let input = read_input("refute", read(raw))?;
    let manifest = read_json_file(runtime, &manifest_path(&input.run_dir), "invalid-run", parse_manifest)?;
    let merged = read_json_file(runtime, &findings_path(&input.run_dir), "invalid-run", parse_fan_out_result)?;
    if runtime.files.exists(&report_path(&input.run_dir)).unwrap_or(false) {
        return Err(CritiqueError::new("already-recorded", format!("run {} is already recorded; refute comes before record", manifest.run_id)));
    }
    let path = refute_path(&input.run_dir);
    if runtime.files.exists(&path).unwrap_or(false) {
        return Err(CritiqueError::new("already-refuted", format!("run {} already had its refute round", manifest.run_id)));
    }
    let known: HashSet<String> = merged.findings.iter().map(|finding| finding.id.clone()).collect();
    let verdicts = verdicts_by_finding(&input.verdicts, &known)?;
    let targets = targets_of(&merged, &verdicts)?;
    let (ledger_warnings, results) = if targets.is_empty() { (Vec::new(), Vec::new()) } else { round(runtime, &manifest, &merged, &targets)? };
    let mut file = Object::new();
    file.set("counts", Value::Obj(count_outcomes(&results)));
    if !ledger_warnings.is_empty() {
        file.set("ledger_warnings", Value::Arr(ledger_warnings.iter().map(|warning| Value::str(warning)).collect()));
    }
    file.set("refute_path", Value::str(&path));
    file.set("results", Value::Arr(results.iter().map(record_json).collect()));
    file.set("run_id", Value::str(&manifest.run_id));
    file.set("stage", Value::str(&manifest.stage));
    file.set("status", Value::str("checked"));
    let context = RefuteLedgerContext {
        recorded_at: to_iso(runtime.clock.now_ms()),
        repository: manifest.repository.clone(),
        flow: manifest.flow.clone(),
        spec: spec_reference(&manifest),
        run_id: manifest.run_id.clone(),
        stage: manifest.stage.clone(),
        round: manifest.round,
    };
    let lines: String = results
        .iter()
        .filter_map(|result| targets.iter().find(|target| target.id == result.finding_id).map(|finding| format!("{}\n", refute_ledger_line(result, finding, &context))))
        .collect();
    if !lines.is_empty() {
        let ledger = ledger_path(&runtime.env)?;
        runtime.files.mkdir_all(dirname(&ledger)).map_err(CritiqueError::from_fs)?;
        runtime.files.append(&ledger, &lines).map_err(CritiqueError::from_fs)?;
    }
    let file = Value::Obj(file);
    runtime.files.write_text(&path, &format!("{}\n", json::pretty(&file))).map_err(CritiqueError::from_fs)?;
    Ok(file)
}

/// The findings the round refutes: those accepted as critical, with the severity that counts, which is the
/// orchestrator's override, else the one fan-out fixed.
fn targets_of(merged: &FanOutResult, verdicts: &HashMap<String, Verdict>) -> Result<Vec<RunFinding>, CritiqueError> {
    let mut targets = Vec::new();
    for finding in &merged.findings {
        let given = verdicts.get(&finding.id);
        if let Some(problem) = given.and_then(|given| severity_override_problem(finding, given)) {
            return Err(CritiqueError::new("invalid-input", problem));
        }
        let accepted = given.is_some_and(|given| given.verdict == "accepted");
        if let Some(pre) = finding.pre_verdict.as_ref().filter(|pre| accepted && pre.verdict == "rejected") {
            return Err(CritiqueError::new("invalid-input", format!("{} was rejected as {}, which a verdict cannot overturn", finding.id, pre.reason_code)));
        }
        let severity = given.and_then(|given| given.severity.clone()).unwrap_or_else(|| finding.body.severity.clone());
        if accepted && severity == "critical" {
            let mut target = finding.clone();
            target.body.severity = severity;
            targets.push(target);
        }
    }
    Ok(targets)
}

/// Refutes every target, one after the other. Returns what the gateway's ledger would not take, and each result.
fn round(runtime: &Runtime, manifest: &RunManifest, merged: &FanOutResult, targets: &[RunFinding]) -> Result<(Vec<String>, Vec<RefuteRecord>), CritiqueError> {
    let role = read_asset(runtime, "roles/refuter.md")?;
    let sources = PromptSources {
        contract: read_asset(runtime, "prompts/output-contract.md")?,
        lenses: Vec::new(),
        preamble: read_asset(runtime, "prompts/preamble.md")?,
        role: role.clone(),
        schema: read_asset(runtime, FINDING_SCHEMA)?,
    };
    let candidates = refuter_candidates(&load_routing(runtime)?, &manifest.stage, manifest.work.is_some())?;
    let blocked = blocked(runtime, manifest, &candidates)?;
    let packet = index_packet(&read_frozen_packet(runtime, manifest)?, manifest.ticket_check.as_ref().map_or(&[][..], |check| &check.ticket_ids));
    let round = Round {
        manifest,
        merged,
        tripwire: ReadOnlyTripwire::load(runtime, manifest)?,
        tools: CriticTools::new(manifest.limits.search.clone(), join(&[&manifest.run_dir, "search-proof"])),
        packet,
        candidates,
        blocked,
        sources,
        declared: declared_checks(&[&role]).into_iter().map(|check| check.id).collect(),
    };
    let mut ledger_warnings = Vec::new();
    let mut results = Vec::new();
    for finding in targets {
        results.push(refute(runtime, &round, finding, &mut ledger_warnings)?);
    }
    Ok((ledger_warnings, results))
}

fn refute(runtime: &Runtime, round: &Round<'_>, finding: &RunFinding, ledger_warnings: &mut Vec<String>) -> Result<RefuteRecord, CritiqueError> {
    let manifest = round.manifest;
    let raisers: Vec<String> =
        round.merged.critics.iter().filter(|critic| finding.raised_by.contains(&critic.critic_id)).map(|critic| critic.family.clone()).collect();
    let rules = RefuterRules { allowed_families: &manifest.allowed_families, orchestrator_family: &manifest.orchestrator_family };
    let refuter = match choose_refuter(
        &finding.id,
        &round.candidates,
        &|family| family_bar(family, &rules, &raisers),
        &|client| round.blocked.get(client).cloned(),
    ) {
        Ok(refuter) => refuter,
        Err(detail) => return Ok(skipped(&finding.id, detail)),
    };
    let critic_dir = join(&[&manifest.run_dir, "refute", &finding.id]);
    let prompt = |toolset: &_| {
        let slices = manifest.code.as_ref().and_then(|code| code.slices.as_deref()).unwrap_or(&[]);
        with_finding(&assemble_prompt(&round.sources, &listed_files(manifest), &manifest.stage, slices, toolset), finding)
    };
    let launched = run_critic(runtime, &Launch { tripwire: &round.tripwire, critic: &refuter, critic_dir: &critic_dir, prompt: &prompt, retry_of: None, tools: &round.tools })?;
    let context = InvocationContext {
        flow: manifest.flow.clone(),
        repository: manifest.repository.clone(),
        round: manifest.round,
        run_id: manifest.run_id.clone(),
        stage: manifest.stage.clone(),
    };
    let warning = FlowBudget::new(runtime).log_invocation(&context, &LedgerRun::of(&launched.run), &LedgerClient::of(&refuter.client_config), Some(&spec_reference(manifest)))?;
    ledger_warnings.extend(warning);
    let role = round.merged.critics.iter().find(|critic| finding.raised_by.first() == Some(&critic.critic_id)).map(|critic| critic.role.as_str());
    let reply = RefuterReply { run: launched.run, findings: launched.findings };
    let mut judged = judge_reply(finding, &reply, &JudgeContext { declared: &round.declared, stage: &manifest.stage, packet: &round.packet, role });
    judged.refuter = Some(reply.run);
    Ok(judged)
}

/// Why each candidate may not run now: not ready on this machine, an agy client past today's quota, or a Vibe client at
/// 100% of its plan limit. The plan and the count are read once, before the round.
fn blocked(runtime: &Runtime, manifest: &RunManifest, candidates: &[RefuterCandidate]) -> Result<HashMap<String, String>, CritiqueError> {
    let budget = FlowBudget::new(runtime);
    let plan = plan_limit(runtime, "vibe")?.unable_to_start;
    let quota = quota_reached(budget.agy_invocations_today()?, &manifest.limits.antigravity_quota, &manifest.stage);
    let mut reasons = HashMap::new();
    for candidate in candidates {
        let config = &candidate.client_config;
        if !manifest.allowed_families.iter().any(|family| family == config.family()) || config.family() == manifest.orchestrator_family {
            continue;
        }
        let limit = match config {
            CriticClient::Vibe(_) => plan.clone(),
            CriticClient::Agy(_) => quota.clone(),
            _ => None,
        };
        let reason = match limit {
            Some(limit) => Some(limit),
            None => match ungranted(runtime, manifest, config) {
                Some(reason) => Some(reason),
                None => {
                    let ready = client_readiness(runtime, config)?;
                    if ready.ready {
                        None
                    } else {
                        ready.detail
                    }
                }
            },
        };
        if let Some(reason) = reason {
            reasons.insert(candidate.client.clone(), reason);
        }
    }
    Ok(reasons)
}

/// Why Codex may not refute on a `nedap` repository: the grant that let it critique must still be live, since a grant
/// lapses and a refute round may come later. None while it is, and for any other client.
fn ungranted(runtime: &Runtime, manifest: &RunManifest, config: &CriticClient) -> Option<String> {
    if !matches!(config, CriticClient::Codex(_)) {
        return None;
    }
    let client = config.gateway().map_or("codex", |gateway| gateway.client.as_str());
    let Some(workflow) = manifest.work.as_ref().and_then(|work| work.grant_workflow.as_deref()) else {
        return Some(format!("no live grant covered the run's workflow for {client} when it was prepared"));
    };
    let live = GrantGuard { runtime: &runtime.gateway }.live(workflow, client, runtime.clock.now_ms()).is_ok_and(|grant| grant.is_some());
    (!live).then(|| format!("no live grant covers workflow {workflow} for {client}: ask the owner, then write one with model-gateway grant write"))
}
