//! What the refute round does beyond what record reads of its file: the round's choices, the judging of a refuter's
//! reply, the ledger line of each result and the files the round writes. The rest of the node runner's
//! `domain/refute.ts`.

use crate::checks::is_check_id;
use crate::critic::CriticRun;
use crate::error::CritiqueError;
use crate::evidence::{check_evidence, PacketIndex};
use crate::findings::{CriticFinding, Evidence, FindingBody, RunFinding};
use crate::flow_budget::PromptHashes;
use crate::prompt::CriticPrompt;
use crate::refute::RefuteRecord;
use crate::roster::RosterCritic;
use crate::routing::{stage_routing, CriticClient, RosterSlot, Routing};
use model_gateway::json::{self, Object, Value};

/// One client a stage routes, with the slot whose caps a refuter through it takes.
#[derive(Clone, Debug, PartialEq)]
pub struct RefuterCandidate {
    pub client: String,
    pub client_config: CriticClient,
    pub slot: RosterSlot,
}

/// Every client a stage routes, in the order a refuter is chosen: the base slots, then the deep slots whatever the
/// run's depth, each slot's client followed by its fallbacks. A client named twice keeps its first place. A `nedap`
/// repository's run (`work`) has only the stage's work slots, which have no fallbacks: no other client may see its
/// code.
pub fn refuter_candidates(routing: &Routing, stage: &str, work: bool) -> Result<Vec<RefuterCandidate>, CritiqueError> {
    let routes = stage_routing(routing, stage)?;
    let slots: Vec<&RosterSlot> = if work { routes.work.iter().collect() } else { routes.base.iter().chain(routes.deep.iter()).collect() };
    let mut candidates: Vec<RefuterCandidate> = Vec::new();
    for slot in slots {
        let mut names = vec![slot.client.clone()];
        if !work {
            names.extend(routing.fallbacks.iter().find(|(name, _)| *name == slot.client).map(|(_, list)| list.clone()).unwrap_or_default());
        }
        for client in names {
            let Some((_, config)) = routing.clients.iter().find(|(name, _)| *name == client) else {
                return Err(CritiqueError::new("invalid-routing", format!("routing stage {stage} names unknown client {client}")));
            };
            if !candidates.iter().any(|candidate| candidate.client == client) {
                candidates.push(RefuterCandidate { client, client_config: config.clone(), slot: slot.clone() });
            }
        }
    }
    Ok(candidates)
}

/// The families no refuter of a run may come from, beyond those that raised the finding.
pub struct RefuterRules<'a> {
    pub allowed_families: &'a [String],
    pub orchestrator_family: &'a str,
}

/// Why a candidate's family may not refute this finding, if it may not.
pub fn family_bar(family: &str, rules: &RefuterRules<'_>, raisers: &[String]) -> Option<String> {
    if raisers.iter().any(|raiser| raiser == family) {
        return Some(format!("{family} raised the finding"));
    }
    if family == rules.orchestrator_family {
        return Some(format!("{family} is the orchestrator's family"));
    }
    if !rules.allowed_families.iter().any(|allowed| allowed == family) {
        return Some(format!("the allowlist does not permit {family}"));
    }
    None
}

/// The first candidate whose family neither raised the finding nor is the orchestrator's, that the allowlist permits,
/// and that `blocked` lets run now; or, with none, why each candidate was passed over.
pub fn choose_refuter(
    finding_id: &str,
    candidates: &[RefuterCandidate],
    family: &dyn Fn(&str) -> Option<String>,
    blocked: &dyn Fn(&str) -> Option<String>,
) -> Result<RosterCritic, String> {
    let mut passed: Vec<String> = Vec::new();
    for candidate in candidates {
        if let Some(bar) = family(candidate.client_config.family()).or_else(|| blocked(&candidate.client)) {
            passed.push(format!("{}: {bar}", candidate.client));
            continue;
        }
        return Ok(RosterCritic {
            client: candidate.client.clone(),
            role: "refuter".to_string(),
            lenses: Vec::new(),
            max_price_usd: candidate.slot.max_price_usd,
            time_limit_s: candidate.slot.time_limit_s,
            critic_id: format!("r-{finding_id}-{}", candidate.client),
            client_config: candidate.client_config.clone(),
            fallbacks: Vec::new(),
        });
    }
    let why = if passed.is_empty() { "the stage routes none".to_string() } else { passed.join("; ") };
    Err(format!("no routed client can refute it ({why})"))
}

fn object_of(pairs: Vec<(&str, Value)>) -> Object {
    let mut object = Object::new();
    for (key, value) in pairs {
        object.set(key, value);
    }
    object
}

fn evidence_json(evidence: &Evidence) -> Value {
    let mut pairs = vec![("kind", Value::str(&evidence.kind)), ("value", Value::str(&evidence.value))];
    if let Some(file) = &evidence.file {
        pairs.push(("file", Value::str(file)));
    }
    Value::Obj(object_of(pairs))
}

/// A finding's body as the output contract spells it: category, check, claim, evidence, location, severity, suggestion.
pub fn body_json(body: &FindingBody) -> Object {
    let mut pairs = vec![("category", Value::str(&body.category))];
    if let Some(check) = &body.check {
        pairs.push(("check", Value::str(check)));
    }
    pairs.push(("claim", Value::str(&body.claim)));
    pairs.push(("evidence", Value::Arr(body.evidence.iter().map(evidence_json).collect())));
    pairs.push(("location", Value::str(&body.location)));
    pairs.push(("severity", Value::str(&body.severity)));
    if let Some(suggestion) = &body.suggestion {
        pairs.push(("suggestion", Value::str(suggestion)));
    }
    object_of(pairs)
}

/// The refuter's prompt: the stage's prompt for the refuter role, with the finding under refute after the file list.
/// The finding goes in the prompt, never into the packet, so a quote of it is no evidence.
pub fn with_finding(prompt: &CriticPrompt, finding: &RunFinding) -> CriticPrompt {
    let section = [
        "# The finding".to_string(),
        String::new(),
        "This is the finding you check, as its reviewer reported it. It is not one of the files: your evidence comes from the files.".to_string(),
        String::new(),
        "```json".to_string(),
        json::pretty(&Value::Obj(body_json(&finding.body))),
        "```".to_string(),
    ]
    .join("\n");
    CriticPrompt { text: format!("{}\n---\n\n{section}\n", prompt.text), ..prompt.clone() }
}

/// What one refuter launch gave back.
pub struct RefuterReply {
    pub run: CriticRun,
    pub findings: Vec<CriticFinding>,
}

/// What the evidence check of an objection needs besides the finding.
pub struct JudgeContext<'a> {
    pub declared: &'a [String],
    pub stage: &'a str,
    pub packet: &'a PacketIndex,
    /// The role of the critic that raised the finding first, whose evidence rule the objection meets.
    pub role: Option<&'a str>,
}

/// The result of a round for a finding no client could refute.
pub fn skipped(finding_id: &str, detail: String) -> RefuteRecord {
    let mut result = standing(finding_id);
    result.outcome = "skipped".to_string();
    result.detail = Some(detail);
    result.reason_code = Some("no-eligible-client".to_string());
    result
}

fn standing(finding_id: &str) -> RefuteRecord {
    RefuteRecord {
        finding_id: finding_id.to_string(),
        outcome: "stands".to_string(),
        answer: None,
        reason_code: None,
        detail: None,
        severity: None,
        reply: None,
        reply_id: None,
        refuter: None,
    }
}

/// Judges a refuter's reply. An objection counts only as one finding that names a check the refuter's prompt declares
/// (`refute` or `downgrade`); a downgrade must leave the finding below critical. Its evidence is then checked as the
/// stage checks any finding's, with the original finding's category and location, against the packet as prepare froze
/// it: anything the check would reject or call a hint leaves the verdict standing.
pub fn judge_reply(finding: &RunFinding, reply: &RefuterReply, context: &JudgeContext<'_>) -> RefuteRecord {
    let run = &reply.run;
    let mut judged = standing(&finding.id);
    if run.status != "ok" {
        judged.detail = Some(run.detail.clone().unwrap_or_else(|| run.status.clone()));
        judged.reason_code = Some("no-review".to_string());
        return judged;
    }
    let Some(first) = reply.findings.first() else {
        judged.outcome = "no-objection".to_string();
        judged.answer = Some("no-objection".to_string());
        judged.detail = run.no_findings_reason.clone();
        return judged;
    };
    if reply.findings.len() > 1 {
        return invalid(judged, format!("answered with {} entries; a refuter gives one", reply.findings.len()), None, None);
    }
    let objection = &first.body;
    let named = objection.check.as_deref();
    let answer = named.filter(|check| matches!(*check, "refute" | "downgrade") && context.declared.iter().any(|declared| declared == check));
    let Some(answer) = answer else {
        let detail = format!("names the check {}; a refuter answers refute or downgrade", named.unwrap_or("(none)"));
        return invalid(judged, detail, Some(first), None);
    };
    if answer == "downgrade" && objection.severity == "critical" {
        return invalid(judged, "a downgrade that leaves the finding critical".to_string(), Some(first), Some(answer));
    }
    let mut checked = objection.clone();
    checked.category = finding.body.category.clone();
    checked.location = finding.body.location.clone();
    judged.answer = Some(answer.to_string());
    judged.reply = Some(first.body.clone());
    judged.reply_id = Some(first.id.clone());
    match check_evidence(&checked, context.stage, context.packet, context.role) {
        Some(pre) => {
            judged.detail = Some(pre.reason);
            judged.reason_code = Some(pre.reason_code);
        }
        None => {
            judged.outcome = answer.to_string();
            if answer == "downgrade" {
                judged.severity = Some(objection.severity.clone());
            }
        }
    }
    judged
}

fn invalid(mut judged: RefuteRecord, detail: String, reply: Option<&CriticFinding>, answer: Option<&str>) -> RefuteRecord {
    judged.answer = answer.map(str::to_string);
    judged.detail = Some(detail);
    judged.reason_code = Some("invalid-answer".to_string());
    judged.reply = reply.map(|reply| reply.body.clone());
    judged.reply_id = reply.map(|reply| reply.id.clone());
    judged
}

/// How many results ended each way, in the order `refute.json` lists them.
pub fn count_outcomes(results: &[RefuteRecord]) -> Object {
    let names = ["downgrade", "no-objection", "refute", "skipped", "stands"];
    object_of(names.iter().map(|name| (*name, Value::count(results.iter().filter(|result| result.outcome == *name).count()))).collect())
}

/// What every refute ledger line says about its run.
pub struct RefuteLedgerContext {
    pub recorded_at: String,
    pub repository: String,
    pub flow: String,
    pub spec: String,
    pub run_id: String,
    pub stage: String,
    pub round: f64,
}

fn text_or_null(value: Option<&String>) -> Value {
    value.map_or(Value::Null, |text| Value::str(text))
}

fn number_or_null(value: Option<f64>) -> Value {
    value.map_or(Value::Null, Value::Num)
}

/// One ledger line per result, metadata only like every ledger line: the finding, the check it named, the outcome and
/// answer, and the refuter's client and cost. The refuter's claim and evidence stay in `refute.json`.
pub fn refute_ledger_line(result: &RefuteRecord, finding: &RunFinding, context: &RefuteLedgerContext) -> String {
    let refuter = result.refuter.as_ref();
    let check = finding.body.check.as_deref().filter(|check| is_check_id(Some(check)));
    let line = object_of(vec![
        ("type", Value::str("refute")),
        ("flow", Value::str(&context.flow)),
        ("recorded_at", Value::str(&context.recorded_at)),
        ("repository", Value::str(&context.repository)),
        ("round", Value::Num(context.round)),
        ("run_id", Value::str(&context.run_id)),
        ("spec", Value::str(&context.spec)),
        ("stage", Value::str(&context.stage)),
        ("answer", text_or_null(result.answer.as_ref())),
        ("charged_usd", number_or_null(refuter.map(|run| run.charged_usd))),
        ("check", check.map_or(Value::Null, Value::str)),
        ("client", text_or_null(refuter.map(|run| &run.client))),
        ("cost_usd", number_or_null(refuter.and_then(|run| run.cost_usd))),
        ("critic_id", text_or_null(refuter.map(|run| &run.critic_id))),
        ("family", text_or_null(refuter.map(|run| &run.family))),
        ("finding_id", Value::str(&result.finding_id)),
        ("model", text_or_null(refuter.map(|run| &run.model))),
        ("outcome", Value::str(&result.outcome)),
        ("prompt_hashes", refuter.map_or(Value::Null, |run| hashes_json(&run.prompt_hashes))),
        ("reason_code", text_or_null(result.reason_code.as_ref())),
        ("role", Value::str("refuter")),
        ("severity", text_or_null(result.severity.as_ref())),
        ("severity_before", Value::str(&finding.body.severity)),
        ("status", text_or_null(refuter.map(|run| &run.status))),
    ]);
    json::compact(&Value::Obj(line))
}

fn hashes_json(hashes: &PromptHashes) -> Value {
    Value::Obj(object_of(vec![
        ("contract", Value::str(&hashes.contract)),
        ("preamble", Value::str(&hashes.preamble)),
        ("role", Value::str(&hashes.role)),
        ("schema", Value::str(&hashes.schema)),
    ]))
}

/// A critic's run in the order the node runner builds it, which is the order `refute.json` and `findings.json` print.
pub fn run_json(run: &CriticRun) -> Value {
    let mut lens_hashes = Object::new();
    for (name, hash) in &run.lens_hashes {
        lens_hashes.set(name, Value::str(hash));
    }
    let mut pairs = vec![
        ("client", Value::str(&run.client)),
        ("critic_id", Value::str(&run.critic_id)),
        ("family", Value::str(&run.family)),
        ("lenses", Value::Arr(run.lenses.iter().map(|lens| Value::str(lens)).collect())),
        ("model", Value::str(&run.model)),
        ("role", Value::str(&run.role)),
        ("lens_hashes", Value::Obj(lens_hashes)),
        ("prompt_hashes", hashes_json(&run.prompt_hashes)),
        ("prompt_path", Value::str(&run.prompt_path)),
    ];
    if let Some(retry) = &run.retry_of {
        pairs.push(("retry_of", Value::str(retry)));
    }
    pairs.push(("charged_usd", Value::Num(run.charged_usd)));
    pairs.push(("cost_usd", number_or_null(run.cost_usd)));
    pairs.push(("duration_ms", Value::Num(run.duration_ms)));
    pairs.push(("exit_code", number_or_null(run.exit_code)));
    if let Some(path) = &run.export_path {
        pairs.push(("export_path", Value::str(path)));
    }
    pairs.push(("finding_count", Value::Num(run.finding_count)));
    if let Some(path) = &run.stdout_path {
        pairs.push(("stdout_path", Value::str(path)));
    }
    if let Some(vibe) = run.vibe_cost_usd {
        pairs.push(("vibe_cost_usd", number_or_null(vibe)));
    }
    if let Some(tokens) = &run.tokens {
        pairs.push(("tokens", Value::Obj(object_of(vec![("cached", Value::Num(tokens.cached)), ("input", Value::Num(tokens.input)), ("output", Value::Num(tokens.output))]))));
    }
    if let Some(tools) = &run.tools {
        pairs.push(("tools", Value::Arr(tools.iter().map(|tool| Value::str(tool)).collect())));
    }
    if let Some(failure) = &run.search_failure {
        pairs.push(("search_failure", Value::str(failure)));
    }
    if let Some(detail) = &run.detail {
        pairs.push(("detail", Value::str(detail)));
    }
    if let Some(failure) = &run.failure {
        pairs.push(("failure", Value::str(failure)));
    }
    pairs.push(("status", Value::str(&run.status)));
    if let Some(reason) = &run.no_findings_reason {
        pairs.push(("no_findings_reason", Value::str(reason)));
    }
    Value::Obj(object_of(pairs))
}

/// A refuter's objection as it wrote it: its finding's fields, then its id.
fn reply_json(body: &FindingBody, id: Option<&String>) -> Value {
    let mut object = body_json(body);
    if let Some(id) = id {
        object.set("id", Value::str(id));
    }
    Value::Obj(object)
}

/// One result as `refute.json` holds it. The keys follow the node runner's object for each way a round can end,
/// `{ finding_id, ...judged, refuter }` with whatever that way spread into `judged`.
pub fn record_json(record: &RefuteRecord) -> Value {
    let text = |value: &Option<String>| value.as_ref().map(|text| Value::str(text));
    let (answer, detail, code) = (text(&record.answer), text(&record.detail), text(&record.reason_code));
    let outcome = Some(Value::str(&record.outcome));
    let reply = record.reply.as_ref().map(|body| reply_json(body, record.reply_id.as_ref()));
    let finding = Some(Value::str(&record.finding_id));
    let reason = record.reason_code.as_deref();
    let ordered: Vec<(&str, Option<Value>)> = if record.outcome == "skipped" {
        vec![("detail", detail), ("finding_id", finding), ("outcome", outcome), ("reason_code", code)]
    } else if reason == Some("no-review") {
        vec![("finding_id", finding), ("detail", detail), ("outcome", outcome), ("reason_code", code)]
    } else if record.outcome == "no-objection" {
        vec![("finding_id", finding), ("answer", answer), ("outcome", outcome), ("detail", detail)]
    } else if reason == Some("invalid-answer") {
        vec![("finding_id", finding), ("answer", answer), ("detail", detail), ("outcome", outcome), ("reason_code", code), ("reply", reply)]
    } else if record.outcome == "stands" {
        vec![("finding_id", finding), ("answer", answer), ("detail", detail), ("outcome", outcome), ("reason_code", code), ("reply", reply)]
    } else {
        vec![("finding_id", finding), ("answer", answer), ("outcome", outcome), ("reply", reply), ("severity", text(&record.severity))]
    };
    let mut object = Object::new();
    for (key, value) in ordered {
        if let Some(value) = value {
            object.set(key, value);
        }
    }
    if let Some(refuter) = &record.refuter {
        object.set("refuter", run_json(refuter));
    }
    Value::Obj(object)
}
