//! `triage`: risk triage for ticket review. The risk rules decide first, from git and the ticket's labels alone: a
//! touched risk path, a change above the size threshold, a risk label. Only when none fires is the triage model asked
//! whether the change warrants a review, and only where the family allowlist lets the repository send code to the
//! model's family and the flow's Vibe budget is not spent. A model that cannot run, or gives no yes or no, leaves the
//! change risky. Every decision is logged; a low-risk one as a run with status `skipped-low-risk`. The node runner's
//! `application/triage.ts`.

use crate::assets::read_asset;
use crate::clients::{ModelAnswer, ModelAnswerReport, ModelQuestion};
use crate::code_change::{read_change, ReadChange};
use crate::error::{CritiqueError, ShapeError};
use crate::fields::Fields;
use crate::flow_budget::{FlowBudget, GatewayClient, InvocationContext, LedgerClient, LedgerRun};
use crate::gateway_ledger::gateway_ledger_warning;
use crate::locations::ledger_path;
use crate::paths::join;
use crate::plan::plan_limit;
use crate::repository::{read_allowlist, resolve, Resolution, Resolved};
use crate::routing::TriageRouting;
use crate::routing_source::load_routing;
use crate::runtime::Runtime;
use crate::staged_change::{snapshot, staged_input, StagedInputError};
use crate::tracker_resolver::{resolve_tracker, ArtifactRead, TrackedArtifact, TrackerResolution};
use crate::triage::{
    read_triage_answer, risk_signals, signal_json, triage_ledger_line, triage_summary, RecordModel, RiskRules, RiskSignal, TriageRecord, TriageSignal,
    BUDGET, CODE_NOT_ALLOWED, FAILED, FAMILY_NOT_ALLOWED, LOW_RISK, NOT_OPTED_IN, RISKY, UNAVAILABLE, UNCLEAR_ANSWER,
};
use crate::triage_index::{append_triage_index, TriageIndexRecord};
use crate::vibe_client::{vibe_ask, vibe_readiness};
use model_gateway::json::{self, number_text, Object, Value};
use model_gateway::run::compact_timestamp;
use model_gateway::sha256::sha256_hex;
use model_gateway::text::{dirname, js_trim};
use model_gateway::timestamps::to_iso;

/// The triage prompt, under the shipped assets.
const TRIAGE_PROMPT: &str = "prompts/triage.md";
/// Where the diff sits in the triage model's working directory.
const DIFF_PATH: &str = "diff/change.diff";

struct Input {
    repo: String,
    /// The ticket the change delivers, named the way prepare names it; its labels are read for `risk:high`.
    ticket: String,
    base: String,
    head: String,
    /// Whether the change is the staged one rather than `head`.
    staged: bool,
    /// The spec the ticket belongs to; only its reference is read, for the default flow.
    spec: Option<String>,
    /// The flow whose Vibe spend the triage call adds to; the spec's reference, else the ticket's, by default.
    flow: Option<String>,
}

/// Reads the input in the order the node runner evaluates its fields (the object literal's, alphabetical), so the
/// first wrong field is the one reported.
fn read_input(raw: &Value) -> Result<Input, CritiqueError> {
    let shape = |error: ShapeError| error.into_error("invalid-input", "triage input");
    let fields = Fields::of(raw, "").map_err(shape)?;
    let base = fields.string("base").map_err(shape)?;
    let flow = fields.optional_string("flow").map_err(shape)?;
    let head = fields.optional_string("head").map_err(shape)?.unwrap_or_else(|| "HEAD".to_string());
    let repo = fields.string("repo").map_err(shape)?;
    let spec = fields.optional_string("spec").map_err(shape)?;
    let staged = staged_input(&fields, "ticket-review").map_err(|error| match error {
        StagedInputError::Shape(error) => shape(error),
        StagedInputError::Refused(error) => error,
    })?;
    let ticket = fields.string("ticket").map_err(shape)?;
    Ok(Input { base, flow, head, repo, spec, staged, ticket })
}

/// What was decided, and when the model was launched, what it was asked and answered.
struct Decided {
    decision: &'static str,
    signal: TriageSignal,
    /// Why the gateway's ledger would not take the model's run, where it would not.
    ledger_warning: Option<String>,
    model: Option<RecordModel>,
    /// The triage prompt's hash, when the model was launched.
    prompt_hash: Option<String>,
}

fn risky(signal: TriageSignal) -> Decided {
    Decided { decision: RISKY, signal, ledger_warning: None, model: None, prompt_hash: None }
}

fn unavailable(reason: &'static str, detail: String) -> Decided {
    risky(TriageSignal::ModelUnavailable { reason, detail })
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let input = read_input(raw)?;
    let repository = match resolve(runtime, &input.repo)? {
        Resolution::Refused(refusal) => return Ok(refusal),
        Resolution::Resolved(repository) => repository,
    };
    let routing = load_routing(runtime)?;
    let Some(triage) = routing.triage.as_ref() else {
        return Err(CritiqueError::new("invalid-routing", "routing has no triage, so a change cannot be triaged"));
    };
    let tracker = match resolve_tracker(runtime, &repository)? {
        TrackerResolution::Refused(refusal) => return Ok(refusal),
        TrackerResolution::Resolved(tracker) => tracker,
    };
    let ticket = match tracker.artifact(runtime, &input.ticket, false)? {
        ArtifactRead::Refused(refusal) => return Ok(refusal),
        ArtifactRead::Read(ticket) => ticket,
    };
    let spec = match input.spec.as_deref().map(|spec| tracker.artifact(runtime, spec, false)).transpose()? {
        Some(ArtifactRead::Refused(refusal)) => return Ok(refusal),
        Some(ArtifactRead::Read(spec)) => Some(spec),
        None => None,
    };
    let staged = if input.staged { Some(snapshot(runtime, &repository.root)?) } else { None };
    let head = staged.as_ref().map_or(input.head.as_str(), |staged| staged.commit.as_str());
    let read = read_change(runtime, &repository.root, &input.base, head, &routing.packet)?;
    let run_id = fresh_run_id(runtime, &repository);
    let run_dir = join(&[&repository.common_dir, "critique", &run_id]);
    let flow = input.flow.clone().or_else(|| spec.as_ref().map(|spec| spec.reference.clone())).unwrap_or_else(|| ticket.reference.clone());
    let rules = RiskRules { changed_lines_threshold: triage.changed_lines_threshold, labels: &triage.labels, risk_paths: &triage.risk_paths };
    let signals = risk_signals(&read.change, &ticket.labels, &rules).map_err(CritiqueError::unexpected)?;
    let asked = match signals.first() {
        Some(first) => risky(TriageSignal::Risk(first.clone())),
        None => {
            let request = Ask { diff: &read.diff, flow: &flow, repository: &repository, run_dir: &run_dir, run_id: &run_id, spec: input.spec.as_deref(), triage };
            ask(runtime, &request)?
        }
    };
    let summary = [Some(triage_summary(asked.decision, &asked.signal, &triage.model.client)), asked.ledger_warning.as_deref().map(gateway_ledger_warning)]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    let report = Report { change: &read, flow: &flow, run_id: &run_id, signals: &signals, summary, threshold: triage.changed_lines_threshold, ticket: &ticket };
    let result = result_json(&asked, &report);
    runtime.files.write_text(&join(&[&run_dir, "triage.json"]), &format!("{}\n", json::pretty(&result))).map_err(CritiqueError::from_fs)?;
    let recorded_at = to_iso(runtime.clock.now_ms());
    let record = TriageRecord {
        changed_files: read.change.paths.len() as f64,
        changed_lines: read.change.changed_lines,
        changed_lines_threshold: triage.changed_lines_threshold,
        flow: flow.clone(),
        model: asked.model.clone(),
        prompt_hash: asked.prompt_hash.clone(),
        recorded_at: recorded_at.clone(),
        repository: repository.key.clone(),
        run_id: run_id.clone(),
        signal: asked.signal.clone(),
        signals: signals.clone(),
        status: asked.decision.to_string(),
        ticket: ticket.reference.clone(),
    };
    let path = ledger_path(&runtime.env)?;
    runtime.files.mkdir_all(dirname(&path)).map_err(CritiqueError::from_fs)?;
    runtime.files.append(&path, &format!("{}\n", json::compact(&triage_ledger_line(&record)))).map_err(CritiqueError::from_fs)?;
    if let Some(staged) = &staged {
        let index = TriageIndexRecord {
            labels: ticket.labels.clone(),
            recorded_at,
            repository: repository.key.clone(),
            run_id,
            staged_diff: staged.staged.sha256.clone(),
            staged_on: staged.staged.head_commit.clone(),
            status: asked.decision.to_string(),
            ticket: ticket.reference.clone(),
        };
        append_triage_index(runtime, &repository.common_dir, &index)?;
    }
    Ok(result)
}

/// What the result reports beside the decision.
struct Report<'a> {
    change: &'a ReadChange,
    flow: &'a str,
    run_id: &'a str,
    signals: &'a [RiskSignal],
    summary: String,
    threshold: f64,
    ticket: &'a TrackedArtifact,
}

/// The result, with its keys in the order the node runner's object literal gives them.
fn result_json(asked: &Decided, report: &Report<'_>) -> Value {
    let mut change = Object::new();
    change.set("base_commit", Value::str(&report.change.base_commit));
    change.set("changed_files", Value::count(report.change.change.paths.len()));
    change.set("changed_lines", Value::Num(report.change.change.changed_lines));
    change.set("changed_lines_threshold", Value::Num(report.threshold));
    change.set("head_commit", Value::str(&report.change.head_commit));
    let mut ticket = Object::new();
    ticket.set("labels", Value::Arr(report.ticket.labels.iter().map(|label| Value::str(label)).collect()));
    ticket.set("reference", Value::str(&report.ticket.reference));
    let mut result = Object::new();
    result.set("change", Value::Obj(change));
    result.set("flow", Value::str(report.flow));
    if let Some(model) = &asked.model {
        let mut call = Object::new();
        call.set("charged_usd", Value::Num(model.charged_usd));
        call.set("client", Value::str(&model.client));
        call.set("cost_usd", model.cost_usd.map_or(Value::Null, Value::Num));
        call.set("model", Value::str(&model.model));
        if let Some(answer) = model.answer {
            call.set("answer", Value::str(answer));
        }
        result.set("model", Value::Obj(call));
    }
    result.set("run_id", Value::str(report.run_id));
    result.set("signal", signal_json(&asked.signal));
    result.set("signals", Value::Arr(report.signals.iter().map(|signal| signal_json(&TriageSignal::Risk(signal.clone()))).collect()));
    result.set("status", Value::str(asked.decision));
    result.set("summary", Value::str(&report.summary));
    result.set("ticket", Value::Obj(ticket));
    Value::Obj(result)
}

/// What the triage model is asked about.
struct Ask<'a> {
    triage: &'a TriageRouting,
    repository: &'a Resolved,
    run_dir: &'a str,
    run_id: &'a str,
    flow: &'a str,
    diff: &'a str,
    /// The spec the ticket belongs to, where the input names one.
    spec: Option<&'a str>,
}

/// Asks the triage model, when it may run, whether the change warrants a second review. Each reason it may not run
/// leaves the change risky.
fn ask(runtime: &Runtime, request: &Ask<'_>) -> Result<Decided, CritiqueError> {
    let (repository, triage) = (request.repository, request.triage);
    let config = &triage.model.client_config;
    let allowlist = read_allowlist(runtime)?;
    let Some(entry) = allowlist.get(&repository.key) else {
        return Ok(unavailable(NOT_OPTED_IN, format!("{} has no family allowlist entry on this machine", repository.key)));
    };
    if !entry.code {
        return Ok(unavailable(CODE_NOT_ALLOWED, format!("{} allows documents but not code", repository.key)));
    }
    if !entry.families.contains(&config.family) {
        return Ok(unavailable(FAMILY_NOT_ALLOWED, format!("{} does not allow the {} family", repository.key, config.family)));
    }
    if let Some(reason) = plan_limit(runtime, "vibe")?.unable_to_start {
        return Ok(unavailable(BUDGET, reason));
    }
    let ready = vibe_readiness(runtime, config)?;
    if !ready.ready {
        return Ok(unavailable(UNAVAILABLE, ready.detail.unwrap_or_default()));
    }

    let prompt_asset = read_asset(runtime, TRIAGE_PROMPT)?;
    let work_dir = join(&[request.run_dir, "packet"]);
    let prompt_path = join(&[request.run_dir, "prompt.md"]);
    let write = |place: &str, content: &str| runtime.files.write_text(place, content).map_err(CritiqueError::from_fs);
    write(&join(&[&work_dir, DIFF_PATH]), request.diff)?;
    write(&prompt_path, &format!("{}\n\n# Files\n\n- `{DIFF_PATH}`: the change under review, as a diff against its base\n", js_trim(&prompt_asset)))?;
    let question = ModelQuestion {
        max_price_usd: triage.model.max_price_usd,
        output_dir: request.run_dir.to_string(),
        prompt_path,
        time_limit_s: triage.model.time_limit_s,
        work_dir,
    };
    let report = vibe_ask(runtime, &question, config)?;
    write(&join(&[request.run_dir, "stdout.json"]), &report.stdout)?;
    write(&join(&[request.run_dir, "stderr.txt"]), &report.stderr)?;
    let (cost_usd, failed) = match &report.answer {
        ModelAnswer::Answered { cost_usd, .. } => (*cost_usd, false),
        ModelAnswer::Failed { cost_usd, .. } => (*cost_usd, true),
    };
    let context = InvocationContext {
        flow: request.flow.to_string(),
        repository: repository.key.clone(),
        round: 1.0,
        run_id: request.run_id.to_string(),
        stage: "ticket-review".to_string(),
    };
    let run = LedgerRun {
        charged_usd: report.charged_usd,
        client: triage.model.client.clone(),
        cost_usd,
        critic_id: "triage".to_string(),
        duration_ms: None,
        exit_code: Some(f64::from(report.exit_code)),
        failure: None,
        family: config.family.clone(),
        finding_count: None,
        lens_hashes: None,
        lenses: None,
        model: config.model.clone(),
        prompt_hashes: None,
        retry_of: None,
        role: None,
        status: if failed { "failed" } else { "ok" }.to_string(),
        tokens: None,
        vibe_cost_usd: Some(report.vibe_cost_usd),
    };
    let client = LedgerClient {
        kind: "vibe".to_string(),
        gateway: config.gateway.as_ref().map(|gateway| GatewayClient { client: gateway.client.clone(), model: gateway.model.clone() }),
    };
    let ledger_warning = FlowBudget::new(runtime).log_invocation(&context, &run, &client, request.spec)?;
    let mut decided = answered(&report);
    decided.ledger_warning = ledger_warning;
    decided.model = Some(model_call(&triage.model.client, &config.model, &report));
    decided.prompt_hash = Some(sha256_hex(prompt_asset.as_bytes()));
    Ok(decided)
}

/// A yes is risky and a no low-risk; a failed run or any other answer leaves the change risky.
fn answered(report: &ModelAnswerReport) -> Decided {
    match &report.answer {
        ModelAnswer::Failed { failure, detail, .. } => unavailable(FAILED, format!("{failure}: {detail}")),
        ModelAnswer::Answered { text, .. } => match read_triage_answer(text) {
            None => unavailable(UNCLEAR_ANSWER, "the triage model answered neither yes nor no".to_string()),
            Some(word) => Decided {
                decision: if word == "yes" { RISKY } else { LOW_RISK },
                signal: TriageSignal::Model { answer: word },
                ledger_warning: None,
                model: None,
                prompt_hash: None,
            },
        },
    }
}

fn model_call(client: &str, model: &str, report: &ModelAnswerReport) -> RecordModel {
    let (cost_usd, answer) = match &report.answer {
        ModelAnswer::Answered { text, cost_usd } => (*cost_usd, read_triage_answer(text)),
        ModelAnswer::Failed { cost_usd, .. } => (*cost_usd, None),
    };
    RecordModel { answer, charged_usd: report.charged_usd, client: client.to_string(), cost_usd, model: model.to_string() }
}

/// A run id no earlier triage took: the moment, `-triage`, and a count from 2 where the moment is taken.
fn fresh_run_id(runtime: &Runtime, repository: &Resolved) -> String {
    let base = format!("{}-triage", compact_timestamp(runtime.clock.now_ms()));
    let mut candidate = base.clone();
    let mut n = 2.0;
    while runtime.files.exists(&join(&[&repository.common_dir, "critique", &candidate, "triage.json"])).unwrap_or(false) {
        candidate = format!("{base}-{}", number_text(n));
        n += 1.0;
    }
    candidate
}
