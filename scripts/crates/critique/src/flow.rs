//! One feature's flow state and its single next step, computed from the ledger, the run directories, the review and
//! gate indexes and the tracker. The node runner's `domain/flow.ts`.

use crate::collate::locale_compare;
use crate::error::ShapeError;
use crate::fields::Fields;
use crate::ledger::FlowEvent;
use crate::stage::CRITIQUE_STAGES;
use crate::stage_table::second_round;
use crate::verdicts::SECOND_ROUND_AFTER_CRITICAL;
use model_gateway::json::{self, Value};
use model_gateway::sha256::sha256_hex;
use std::cmp::Ordering;

/// The spans of one feature's flow.
pub const FLOW_PHASES: [&str; 3] = ["planning", "implementation", "finishing"];
/// The stages a flow runs for the feature as a whole, in the order the computed state lists them.
pub const FLOW_STAGES: [&str; 5] = ["decisions", "plan", "feature-review", "spec", "tickets"];
const STAGE_STATUSES: [&str; 5] = ["not-run", "in-progress", "second-round-due", "check-failed", "recorded"];
const REVIEW_STATUSES: [&str; 5] = ["not-reviewed", "skipped-low-risk", "reviewed", "findings-open", "degraded"];
const GATE_STATUSES: [&str; 3] = ["none", "pass", "fail"];
const SEVERITIES: [&str; 3] = ["critical", "major", "minor"];

#[derive(Clone, Debug, PartialEq)]
pub struct StageProgress {
    pub status: String,
    /// Every run of the stage in the flow, oldest first, recorded or not.
    pub run_ids: Vec<String>,
    pub latest_run_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowTicket {
    pub id: String,
    pub reference: String,
    pub title: String,
    pub blocked_by: Vec<String>,
    /// Closed on the tracker, marked landed by the owner, or reviewed (or skipped as low-risk) at its commit.
    pub landed: bool,
    pub review: String,
    /// The ticket-review and triage runs that decided `review`, oldest first.
    pub review_run_ids: Vec<String>,
}

/// A needs-human verdict the owner has not answered.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenQuestion {
    pub run_id: String,
    pub stage: String,
    pub finding_id: String,
    pub severity: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowGate {
    pub status: String,
    pub run_id: Option<String>,
    pub head_commit: Option<String>,
    pub line: Option<String>,
}

/// One feature's flow state. `stages` keeps the order the state was made or read in, which is the order its JSON
/// writes them: the computed state and a state read back list them differently, as the node runner's object literals do.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowState {
    pub flow: String,
    pub spec: Option<String>,
    pub draft: Option<String>,
    pub repository: String,
    pub updated_at: String,
    pub head_commit: String,
    pub phase: String,
    pub no_critique: bool,
    pub grilled: bool,
    pub refactor: bool,
    pub plan_path: Option<String>,
    pub stages: Vec<(String, StageProgress)>,
    pub tickets: Vec<FlowTicket>,
    pub open_questions: Vec<OpenQuestion>,
    pub gate: FlowGate,
    pub vibe_spend_usd: f64,
}

impl FlowState {
    pub fn stage(&self, name: &str) -> &StageProgress {
        &self.stages.iter().find(|(stage, _)| stage == name).expect("every flow stage has progress").1
    }
}

/// One run of the flow, from its run directory, the ledger, or both.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowRun {
    pub run_id: String,
    pub stage: String,
    pub round: f64,
    pub recorded: bool,
    /// The tickets stage's ticket check as the packet froze it.
    pub ticket_check: Option<String>,
}

/// A finding line of one of the flow's runs.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowFinding {
    pub run_id: String,
    pub stage: String,
    pub finding_id: String,
    pub severity: String,
    pub verdict: String,
    pub answered: bool,
}

/// One of the spec's tickets, and whether the tracker still holds it open.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceTicket {
    pub id: String,
    pub reference: String,
    pub title: String,
    pub blocked_by: Vec<String>,
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceReview {
    pub run_id: String,
    pub recorded_at: String,
    pub ticket: String,
    pub review: String,
    pub degraded: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceSkip {
    pub run_id: String,
    pub recorded_at: String,
    pub ticket: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceGate {
    pub run_id: String,
    pub head_commit: String,
    pub gate: String,
    pub line: String,
}

/// Everything the flow state is computed from, already narrowed to this flow and repository.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowSources {
    pub flow: String,
    pub spec: Option<String>,
    pub draft: Option<String>,
    pub repository: String,
    pub now: String,
    pub head_commit: String,
    pub plan_path: Option<String>,
    pub plan_exists: bool,
    pub runs: Vec<FlowRun>,
    pub findings: Vec<FlowFinding>,
    pub tickets: Vec<SourceTicket>,
    pub reviews: Vec<SourceReview>,
    pub skipped_reviews: Vec<SourceSkip>,
    pub gates: Vec<SourceGate>,
    pub events: Vec<FlowEvent>,
    pub vibe_spend_usd: f64,
}

/// The flow key of a draft: the flow before `/to-spec` has written a spec.
pub fn draft_flow(name: &str) -> String {
    format!("draft:{name}")
}

/// The state file's name: readable, and unique per flow however its key is spelt.
pub fn flow_state_file_name(flow: &str) -> String {
    let lower = flow.to_lowercase();
    let mut slug = String::new();
    let mut gap = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap {
                slug.push('-');
            }
            gap = false;
            slug.push(c);
        } else {
            gap = true;
        }
    }
    // A run of other characters becomes one dash; one at either end is dropped.
    if gap && !slug.is_empty() {
        slug.push('-');
    }
    let slug = slug.trim_matches('-');
    let slug: String = slug.chars().take(60).collect();
    format!("{slug}-{}.json", &sha256_hex(flow.as_bytes())[..12])
}

/// Run ids start with their compact timestamp, and a run id taken twice in a second gets `-2`, `-3`, ...
fn compare_run_ids(a: &str, b: &str) -> Ordering {
    locale_compare(a, b, true)
}

/// Computes the flow state from its sources.
pub fn flow_state(sources: &FlowSources) -> FlowState {
    let stages: Vec<(String, StageProgress)> = FLOW_STAGES.iter().map(|name| (name.to_string(), stage_progress(sources, name))).collect();
    let no_critique = sources.events.iter().fold(false, |value, event| if let FlowEvent::NoCritique(set) = event { *set } else { value });
    let refactor = sources.plan_exists;
    let ran = |name: &str| stages.iter().any(|(stage, progress)| stage == name && !progress.run_ids.is_empty());
    let grilled = sources.spec.is_some() || refactor || ran("decisions") || ran("plan") || sources.events.iter().any(|event| matches!(event, FlowEvent::Grilled));
    let tickets: Vec<FlowTicket> = dependency_order(&sources.tickets).into_iter().map(|ticket| flow_ticket(sources, ticket)).collect();
    let gate = match sources.gates.last() {
        None => FlowGate { head_commit: None, line: None, run_id: None, status: "none".to_string() },
        Some(gate) => FlowGate { head_commit: Some(gate.head_commit.clone()), line: Some(gate.line.clone()), run_id: Some(gate.run_id.clone()), status: gate.gate.clone() },
    };
    let mut state = FlowState {
        draft: sources.draft.clone(),
        flow: sources.flow.clone(),
        gate,
        grilled,
        head_commit: sources.head_commit.clone(),
        no_critique,
        open_questions: open_questions(sources),
        phase: String::new(),
        plan_path: sources.plan_path.clone(),
        refactor,
        repository: sources.repository.clone(),
        spec: sources.spec.clone(),
        stages,
        tickets,
        updated_at: sources.now.clone(),
        vibe_spend_usd: sources.vibe_spend_usd,
    };
    state.phase = flow_phase(&state).to_string();
    state
}

fn stage_progress(sources: &FlowSources, stage: &str) -> StageProgress {
    let mut runs: Vec<&FlowRun> = sources.runs.iter().filter(|run| run.stage == stage).collect();
    runs.sort_by(|a, b| compare_run_ids(&a.run_id, &b.run_id));
    let latest = runs.last();
    let progress = |status: &str| StageProgress { latest_run_id: latest.map(|run| run.run_id.clone()), run_ids: runs.iter().map(|run| run.run_id.clone()).collect(), status: status.to_string() };
    let Some(latest) = latest else { return progress("not-run") };
    if !latest.recorded {
        return progress("in-progress");
    }
    if latest.ticket_check.as_deref() == Some("failed") {
        return progress("check-failed");
    }
    let critical = sources.findings.iter().filter(|finding| finding.run_id == latest.run_id && finding.severity == "critical" && finding.verdict == "accepted").count();
    if latest.round == 1.0 && second_round(stage) && critical > SECOND_ROUND_AFTER_CRITICAL {
        return progress("second-round-due");
    }
    progress("recorded")
}

/// A record written before #259 says `pass` with `degraded: true` for a run no critic reported in.
fn review_status(review: &SourceReview) -> &'static str {
    if review.review == "fail" {
        "findings-open"
    } else if review.review == "degraded" || review.degraded {
        "degraded"
    } else {
        "reviewed"
    }
}

fn flow_ticket(sources: &FlowSources, ticket: &SourceTicket) -> FlowTicket {
    let mut decisions: Vec<(&str, &str, &str)> = sources
        .reviews
        .iter()
        .filter(|review| review.ticket == ticket.reference)
        .map(|review| (review.recorded_at.as_str(), review_status(review), review.run_id.as_str()))
        .chain(sources.skipped_reviews.iter().filter(|skip| skip.ticket == ticket.reference).map(|skip| (skip.recorded_at.as_str(), "skipped-low-risk", skip.run_id.as_str())))
        .collect();
    decisions.sort_by(|a, b| locale_compare(a.0, b.0, false).then_with(|| compare_run_ids(a.2, b.2)));
    let review = decisions.last().map_or("not-reviewed", |decision| decision.1);
    let marked = sources.events.iter().any(|event| matches!(event, FlowEvent::Landed(landed) if *landed == ticket.reference));
    FlowTicket {
        blocked_by: ticket.blocked_by.clone(),
        id: ticket.id.clone(),
        landed: !ticket.open || marked || review == "reviewed" || review == "skipped-low-risk",
        reference: ticket.reference.clone(),
        review: review.to_string(),
        review_run_ids: decisions.iter().map(|decision| decision.2.to_string()).collect(),
        title: ticket.title.clone(),
    }
}

fn open_questions(sources: &FlowSources) -> Vec<OpenQuestion> {
    let answered: Vec<(&str, &str)> = sources
        .events
        .iter()
        .filter_map(|event| match event {
            FlowEvent::Answered { run_id, finding_id } => Some((run_id.as_str(), finding_id.as_str())),
            _ => None,
        })
        .collect();
    sources
        .findings
        .iter()
        .filter(|finding| finding.verdict == "needs-human" && !finding.answered && !answered.contains(&(finding.run_id.as_str(), finding.finding_id.as_str())))
        .map(|finding| OpenQuestion { finding_id: finding.finding_id.clone(), run_id: finding.run_id.clone(), severity: finding.severity.clone(), stage: finding.stage.clone() })
        .collect()
}

/// Orders tickets so each comes after every ticket of the spec that blocks it, keeping tracker order otherwise. A
/// blocker outside the spec is not waited for, and a cycle is broken at its earliest ticket in tracker order.
pub fn dependency_order(tickets: &[SourceTicket]) -> Vec<&SourceTicket> {
    let ids: Vec<&str> = tickets.iter().map(|ticket| ticket.id.as_str()).collect();
    let mut left: Vec<&SourceTicket> = tickets.iter().collect();
    let mut placed: Vec<&str> = Vec::new();
    let mut ordered = Vec::new();
    while !left.is_empty() {
        let ready = |ticket: &&SourceTicket| ticket.blocked_by.iter().all(|blocker| !ids.contains(&blocker.as_str()) || placed.contains(&blocker.as_str()));
        let at = left.iter().position(ready).unwrap_or(0);
        let next = left[at];
        // `left.filter(ticket => ticket !== next)` drops this ticket only; an equal one elsewhere is another object.
        left.remove(at);
        placed.push(&next.id);
        ordered.push(next);
    }
    ordered
}

/// Which span of the flow the state is in.
pub fn flow_phase(state: &FlowState) -> &'static str {
    let critique = !state.no_critique;
    if state.spec.is_none() || (critique && state.stage("spec").status != "recorded") || state.tickets.is_empty() || (critique && state.stage("tickets").status != "recorded") {
        return "planning";
    }
    if state.tickets.iter().any(|ticket| !ticket.landed) {
        "implementation"
    } else {
        "finishing"
    }
}

/// The single next step of a flow, as the JSON the node runner writes for it.
pub fn next_step(state: &FlowState) -> Value {
    if state.spec.is_none() {
        return draft_step(state);
    }
    planning_step(state).unwrap_or_else(|| work_step(state))
}

fn command(skill: &str, reason: &str, argument: Option<&str>) -> Value {
    let command = argument.map_or(skill.to_string(), |argument| format!("{skill} {argument}"));
    json::object(vec![("command", Value::Str(command)), ("kind", Value::str("command")), ("reason", Value::str(reason)), ("skill", Value::str(skill))])
}

fn stage_value(stage: &str, reason: &str) -> Value {
    json::object(vec![("kind", Value::str("stage")), ("reason", Value::str(reason)), ("stage", Value::str(stage))])
}

/// What a stage still owes the flow, or `None` once it is recorded.
fn stage_step(state: &FlowState, stage: &str, not_run: &str) -> Option<Value> {
    let progress = state.stage(stage);
    let run = progress.latest_run_id.clone().unwrap_or_default();
    match progress.status.as_str() {
        "not-run" => Some(stage_value(stage, not_run)),
        "in-progress" => Some(json::object(vec![
            ("kind", Value::str("stage")),
            ("reason", Value::Str(format!("Run {run} of the {stage} stage is not recorded yet: finish it before moving on."))),
            ("resume_run_id", Value::Str(run.clone())),
            ("stage", Value::str(stage)),
        ])),
        "second-round-due" => Some(json::object(vec![
            ("kind", Value::str("stage")),
            ("previous_run_id", Value::Str(run.clone())),
            ("reason", Value::Str(format!("Run {run} of the {stage} stage accepted more than three critical findings: run its second round on the revised artifact."))),
            ("stage", Value::str(stage)),
        ])),
        "check-failed" => Some(stage_value(stage, "The ticket check found a dependency cycle: fix the blocker links and check the tickets again.")),
        _ => None,
    }
}

fn draft_step(state: &FlowState) -> Value {
    if !state.grilled {
        return command("/grill-with-docs", "Every flow starts by grilling the idea against the domain docs; type /flow again when the grilling is done.", None);
    }
    // A refactor's plan file takes the decisions stage's place (ADR-0005).
    if state.refactor {
        let plan = if state.no_critique {
            None
        } else {
            stage_step(state, "plan", "The refactoring plan is drafted: have critics from other model families check its steps, their order and their anchors before the spec is written.")
        };
        return plan.unwrap_or_else(|| command("/to-spec", "The plan is settled, so the spec can be written from it; its Open decisions section must be empty first.", None));
    }
    let decisions = if state.no_critique {
        None
    } else {
        stage_step(state, "decisions", "The grilling is done: have critics from other model families look for decisions it left open before the spec is written.")
    };
    decisions.unwrap_or_else(|| command("/to-spec", "The decisions are settled, so the spec can be written from them.", None))
}

fn open_critical(state: &FlowState) -> Option<&OpenQuestion> {
    if state.no_critique {
        return None;
    }
    state.open_questions.iter().find(|question| question.severity == "critical")
}

fn question_step(question: &OpenQuestion, next: &str) -> Value {
    let reason = format!("Critical question {} from the {} critique (run {}) is unanswered: settle it before {next}.", question.finding_id, question.stage, question.run_id);
    command("/grill-me", &reason, None)
}

/// What planning still owes, or `None` once the spec is critiqued and its tickets are cut and checked.
fn planning_step(state: &FlowState) -> Option<Value> {
    let critique = !state.no_critique;
    if critique {
        if let Some(step) = stage_step(state, "spec", "The spec is written: have critics from other model families red-team it before it is cut into tickets.") {
            return Some(step);
        }
    }
    if state.tickets.is_empty() {
        return Some(match open_critical(state) {
            None => command("/to-tickets", "The spec is settled, so it can be cut into tickets.", None),
            Some(question) => question_step(question, "cutting tickets from the spec"),
        });
    }
    if critique {
        stage_step(state, "tickets", "The tickets are cut: check them for cycles, untestable criteria and missing seams before implementing.")
    } else {
        None
    }
}

/// Implementation while a ticket has not landed, finishing once all have.
fn work_step(state: &FlowState) -> Value {
    let waiting: Vec<&FlowTicket> = state.tickets.iter().filter(|ticket| !ticket.landed).collect();
    let Some(ticket) = waiting.first() else { return finishing_step(state) };
    if let Some(question) = open_critical(state) {
        return question_step(question, "implementing");
    }
    let review_again = |reason: String| json::object(vec![("kind", Value::str("stage")), ("reason", Value::Str(reason)), ("stage", Value::str("ticket-review")), ("ticket", Value::str(&ticket.id))]);
    if ticket.review == "findings-open" && !state.no_critique {
        return review_again(format!("The review of ticket {} accepted a critical finding that is not fixed: fix it, then review the staged change again.", ticket.id));
    }
    if ticket.review == "degraded" && !state.no_critique {
        return review_again(format!("No critic reported in the review of ticket {}, so nobody reviewed it: review the staged change again.", ticket.id));
    }
    let left = if waiting.len() == 1 { "it is the last ticket left".to_string() } else { format!("it is the next of {} tickets left in dependency order", waiting.len()) };
    // `/implement` is given an issue's number, or a local ticket's path, which names it unambiguously.
    let argument = if ticket.reference.contains('#') { &ticket.id } else { &ticket.reference };
    command("/implement", &format!("Ticket {} has not landed and {left}; start it in a fresh session.", ticket.id), Some(argument))
}

fn finishing_step(state: &FlowState) -> Value {
    let done = |reason: &str| json::object(vec![("kind", Value::str("done")), ("reason", Value::str(reason))]);
    if state.no_critique {
        return done("Every ticket has landed, and critique was skipped on purpose, so there is no feature review to run.");
    }
    if let Some(step) = stage_step(state, "feature-review", "Every ticket has landed: review the feature as a whole before the pull request.") {
        return step;
    }
    let gate = &state.gate;
    if gate.status == "pass" && gate.head_commit.as_deref() == Some(state.head_commit.as_str()) {
        return done("GATE: PASS for the current head: the feature is ready for its pull request.");
    }
    let reason = match gate.status.as_str() {
        "fail" => format!(
            "The last feature review held the gate shut ({}) and its follow-ups have landed: review the feature again on the new head.",
            gate.line.as_deref().unwrap_or("GATE: FAIL")
        ),
        "none" => "The feature review left no gate: review the feature again.".to_string(),
        _ => "The gate passed for an earlier head: review the feature again on the current one.".to_string(),
    };
    stage_value("feature-review", &reason)
}

fn text(value: &Option<String>) -> Value {
    value.as_deref().map_or(Value::Null, Value::str)
}

fn strings(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

/// The state as the node runner writes it.
pub fn state_json(state: &FlowState) -> Value {
    let stages = json::object(
        state
            .stages
            .iter()
            .map(|(name, progress)| {
                (name.as_str(), json::object(vec![("latest_run_id", text(&progress.latest_run_id)), ("run_ids", strings(&progress.run_ids)), ("status", Value::str(&progress.status))]))
            })
            .collect(),
    );
    let tickets = state
        .tickets
        .iter()
        .map(|ticket| {
            json::object(vec![
                ("blocked_by", strings(&ticket.blocked_by)),
                ("id", Value::str(&ticket.id)),
                ("landed", Value::Bool(ticket.landed)),
                ("reference", Value::str(&ticket.reference)),
                ("review", Value::str(&ticket.review)),
                ("review_run_ids", strings(&ticket.review_run_ids)),
                ("title", Value::str(&ticket.title)),
            ])
        })
        .collect();
    let questions = state
        .open_questions
        .iter()
        .map(|question| {
            json::object(vec![("finding_id", Value::str(&question.finding_id)), ("run_id", Value::str(&question.run_id)), ("severity", Value::str(&question.severity)), ("stage", Value::str(&question.stage))])
        })
        .collect();
    let gate = json::object(vec![("head_commit", text(&state.gate.head_commit)), ("line", text(&state.gate.line)), ("run_id", text(&state.gate.run_id)), ("status", Value::str(&state.gate.status))]);
    json::object(vec![
        ("draft", text(&state.draft)),
        ("flow", Value::str(&state.flow)),
        ("gate", gate),
        ("grilled", Value::Bool(state.grilled)),
        ("head_commit", Value::str(&state.head_commit)),
        ("no_critique", Value::Bool(state.no_critique)),
        ("open_questions", Value::Arr(questions)),
        ("phase", Value::str(&state.phase)),
        ("plan_path", text(&state.plan_path)),
        ("refactor", Value::Bool(state.refactor)),
        ("repository", Value::str(&state.repository)),
        ("spec", text(&state.spec)),
        ("stages", stages),
        ("tickets", Value::Arr(tickets)),
        ("updated_at", Value::str(&state.updated_at)),
        ("version", Value::Num(1.0)),
        ("vibe_spend_usd", Value::Num(state.vibe_spend_usd)),
    ])
}

/// Reads a flow state file back, field by field, in the order the node runner reads them.
pub fn parse_flow_state(text: &str) -> Result<FlowState, String> {
    let value = json::parse(text)?;
    read_flow_state(&value).map_err(|error| error.0)
}

fn read_flow_state(value: &Value) -> Result<FlowState, ShapeError> {
    let file = Fields::of(value, "")?;
    let stages_fields = file.object("stages")?;
    let progress = |name: &str| -> Result<(String, StageProgress), ShapeError> {
        let stage = stages_fields.object(name)?;
        let latest_run_id = stage.nullable_string("latest_run_id")?;
        let run_ids = stage.strings("run_ids")?;
        let status = stage.one_of("status", &STAGE_STATUSES)?;
        Ok((name.to_string(), StageProgress { latest_run_id, run_ids, status }))
    };
    let gate_fields = file.object("gate")?;
    let draft = file.nullable_string("draft")?;
    let flow = file.string("flow")?;
    let gate = FlowGate {
        head_commit: gate_fields.nullable_string("head_commit")?,
        line: gate_fields.nullable_string("line")?,
        run_id: gate_fields.nullable_string("run_id")?,
        status: gate_fields.one_of("status", &GATE_STATUSES)?,
    };
    let grilled = file.boolean("grilled")?;
    let head_commit = file.string("head_commit")?;
    let no_critique = file.boolean("no_critique")?;
    let open_questions = file.list("open_questions", |question| {
        Ok(OpenQuestion {
            finding_id: question.string("finding_id")?,
            run_id: question.string("run_id")?,
            severity: question.one_of("severity", &SEVERITIES)?,
            stage: question.one_of("stage", &CRITIQUE_STAGES)?,
        })
    })?;
    let phase = file.one_of("phase", &FLOW_PHASES)?;
    let plan_path = file.nullable_string("plan_path")?;
    let refactor = file.boolean("refactor")?;
    let repository = file.string("repository")?;
    let spec = file.nullable_string("spec")?;
    let stages = ["decisions", "feature-review", "plan", "spec", "tickets"].iter().map(|name| progress(name)).collect::<Result<Vec<_>, _>>()?;
    let tickets = file.list("tickets", |ticket| {
        Ok(FlowTicket {
            blocked_by: ticket.strings("blocked_by")?,
            id: ticket.string("id")?,
            landed: ticket.boolean("landed")?,
            reference: ticket.string("reference")?,
            review: ticket.one_of("review", &REVIEW_STATUSES)?,
            review_run_ids: ticket.strings("review_run_ids")?,
            title: ticket.string("title")?,
        })
    })?;
    let updated_at = file.string("updated_at")?;
    file.literal("version", 1.0)?;
    let vibe_spend_usd = file.number("vibe_spend_usd")?;
    Ok(FlowState {
        draft,
        flow,
        gate,
        grilled,
        head_commit,
        no_critique,
        open_questions,
        phase,
        plan_path,
        refactor,
        repository,
        spec,
        stages,
        tickets,
        updated_at,
        vibe_spend_usd,
    })
}
