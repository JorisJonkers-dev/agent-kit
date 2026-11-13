//! `state`: reads, updates and rebuilds one feature's flow state, and says what the single next step is. The state is
//! computed from the ledger, the run directories, the review and gate indexes and the tracker, then saved as a cache
//! other readers can take without recomputing it. It sends nothing anywhere, so it needs no family allowlist entry,
//! but a repository whose remotes refuse a critique is refused here too. The node runner's `application/flow-state.ts`;
//! `FlowLocator`, `flow_keys` and `read_flow_name` are shared with `budget`.

use crate::budget::spend_by_flow;
use crate::decode::{decoded, read_json_file};
use crate::error::{CritiqueError, ShapeError};
use crate::feature_gate::gate_index_path;
use crate::feature_review::parse_gate_record;
use crate::fields::Fields;
use crate::flow::{draft_flow, flow_state, flow_state_file_name, next_step, parse_flow_state, state_json, FlowFinding, FlowRun, FlowSources, FlowState, SourceGate, SourceReview, SourceSkip, SourceTicket};
use crate::gate_command::parse_ticket_review_record;
use crate::ledger::{FlowEvent, Ledger, LedgerRunContext};
use crate::ledger_source::LedgerSource;
use crate::locations::{ledger_path, manifest_path, report_path};
use crate::manifest::{parse_manifest, spec_reference};
use crate::paths::join;
use crate::plan_file::{draft_of, plan_file_path_or_null};
use crate::repository::{resolve, Resolution, Resolved};
use crate::runtime::Runtime;
use crate::ticket_review::ticket_review_index_path;
use crate::tracker_resolver::{resolve_tracker, ArtifactRead, TicketsRead, TrackedArtifact, TrackedTicket, Tracker, TrackerResolution};
use model_gateway::json::{self, Object, Value};
use model_gateway::text::{dirname, js_trim};
use model_gateway::timestamps::to_iso;

/// How a caller names a feature: by its spec, or by a draft name before `/to-spec` has written one.
pub struct FlowName {
    pub repo: String,
    pub spec: Option<String>,
    pub draft: Option<String>,
}

/// One feature's flow: its repository and tracker, its spec when it has one, and the key its runs carry.
pub struct LocatedFlow<'a> {
    pub repository: &'a Resolved,
    pub tracker: Tracker<'a>,
    pub spec: Option<TrackedArtifact>,
    pub flow: String,
}

/// Whether a draft name is one: `/^[A-Za-z0-9][A-Za-z0-9._/-]{0,99}$/`.
fn is_draft_name(value: &str) -> bool {
    let mut chars = value.chars();
    let first = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    first && value.chars().count() <= 100 && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

/// Reads `repo` with `spec` or `draft`, at least one of the two, for the command `command`.
pub fn read_flow_name(fields: &Fields<'_>, command: &str) -> Result<FlowName, CritiqueError> {
    let shape = |error: ShapeError| error.into_error("invalid-input", &format!("{command} input"));
    let spec = fields.optional_string("spec").map_err(shape)?;
    let draft = fields.optional_matching("draft", is_draft_name, "a draft name: letters, digits, dots, dashes, underscores and slashes").map_err(shape)?;
    if spec.is_none() && draft.is_none() {
        return Err(CritiqueError::new("invalid-input", format!("{command} input: name the feature by its spec, or by a draft name before /to-spec has written one")));
    }
    Ok(FlowName { draft, repo: fields.string("repo").map_err(shape)?, spec })
}

/// Where a flow is, or the refusal its repository, tracker or spec earns.
pub enum Located<'a> {
    Located(LocatedFlow<'a>),
    Refused(Value),
}

/// Finds the flow a caller names in a resolved repository: a spec, read through the repository's tracker, keys it by
/// its reference, and a draft name by `draft:<name>`.
pub fn locate<'a>(runtime: &Runtime, repository: &'a Resolved, name: &FlowName) -> Result<Located<'a>, CritiqueError> {
    let tracker = match resolve_tracker(runtime, repository)? {
        TrackerResolution::Refused(refusal) => return Ok(Located::Refused(refusal)),
        TrackerResolution::Resolved(tracker) => tracker,
    };
    let spec = match &name.spec {
        None => None,
        Some(spec) => match tracker.artifact(runtime, spec, false)? {
            ArtifactRead::Refused(refusal) => return Ok(Located::Refused(refusal)),
            ArtifactRead::Read(read) => Some(read),
        },
    };
    let flow = spec.as_ref().map_or_else(|| draft_flow(name.draft.as_deref().unwrap_or_default()), |spec| spec.reference.clone());
    Ok(Located::Located(LocatedFlow { flow, repository, spec, tracker }))
}

/// The draft a spec flow grew from, as the last `draft` event of the flow names it.
fn linked_draft(ledger: &Ledger, repository: &str, flow: &str) -> Option<String> {
    ledger.flow_events.iter().fold(None, |draft, event| match &event.event {
        FlowEvent::Draft(name) if event.repository == repository && event.flow == flow => Some(name.clone()),
        _ => draft,
    })
}

/// The keys a flow's runs and launches carry: its own, and once a spec has a draft (named now, or linked earlier), the
/// draft's too, so the draft's runs and spend count with the spec's.
pub fn flow_keys(ledger: &Ledger, located: &LocatedFlow<'_>, draft: Option<&str>) -> (Option<String>, Vec<String>) {
    let linked = draft.map(str::to_string).or_else(|| linked_draft(ledger, &located.repository.key, &located.flow));
    let keys = match (&located.spec, &linked) {
        (Some(_), Some(linked)) => vec![located.flow.clone(), draft_flow(linked)],
        _ => vec![located.flow.clone()],
    };
    (linked, keys)
}

/// What the owner tells the flow state; each becomes a flow event in the ledger, so a rebuild keeps it.
struct StateRecord {
    grilled: bool,
    no_critique: Option<bool>,
    landed: Vec<String>,
    answered: Vec<(String, String)>,
}

fn read_input(raw: &Value) -> Result<(FlowName, bool, Option<StateRecord>), CritiqueError> {
    let shape = |error: ShapeError| error.into_error("invalid-input", "state input");
    let fields = Fields::of(raw, "").map_err(shape)?;
    let name = read_flow_name(&fields, "state")?;
    let cached = fields.has("cached") && fields.boolean("cached").map_err(shape)?;
    let record = if fields.has("record") { Some(read_record(&fields.object("record").map_err(shape)?).map_err(shape)?) } else { None };
    Ok((name, cached, record))
}

fn read_record(fields: &Fields<'_>) -> Result<StateRecord, ShapeError> {
    let answered = if fields.has("answered") { fields.list("answered", |answer| Ok((answer.string("finding_id")?, answer.string("run_id")?)))? } else { Vec::new() };
    let grilled = fields.has("grilled") && fields.boolean("grilled")?;
    let landed = if fields.has("landed") { fields.strings("landed")? } else { Vec::new() };
    let no_critique = if fields.has("no_critique") { Some(fields.boolean("no_critique")?) } else { None };
    Ok(StateRecord { answered, grilled, landed, no_critique })
}

/// Runs `state`.
pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let (name, cached, record) = read_input(raw)?;
    let repository = match resolve(runtime, &name.repo)? {
        Resolution::Refused(refusal) => return Ok(refusal),
        Resolution::Resolved(repository) => repository,
    };
    let located = match locate(runtime, &repository, &name)? {
        Located::Refused(refusal) => return Ok(refusal),
        Located::Located(located) => located,
    };
    let critique_dir = join(&[&repository.common_dir, "critique"]);
    let path = join(&[&critique_dir, "flows", &flow_state_file_name(&located.flow)]);
    let saved = runtime.files.exists(&path).unwrap_or(false);
    if cached && record.is_none() && saved {
        if let Some(state) = cached_state(runtime, &path) {
            return Ok(result(&state, "cache", &path));
        }
    }
    let tickets = match (&located.spec, &name.spec) {
        (Some(_), Some(spec)) => match located.tracker.tickets(runtime, spec)? {
            TicketsRead::Refused(refusal) => return Ok(refusal),
            TicketsRead::Read(tickets) => tickets,
        },
        _ => Vec::new(),
    };
    let read = LedgerSource::new(runtime).read()?;
    let (draft, keys) = flow_keys(&read.ledger, &located, name.draft.as_deref());
    let plan = plan(runtime, &located, draft.as_deref());
    let gather = Gather { critique_dir: &critique_dir, draft: located.spec.as_ref().and(draft.clone()), keys: &keys, ledger: &read.ledger, plan, tickets: &tickets };
    let mut sources = sources(runtime, &located, &gather, &read.invocations)?;
    let link = located.spec.is_some() && name.draft.is_some() && linked_draft(&read.ledger, &repository.key, &located.flow) != name.draft;
    let events = new_events(&name, record.as_ref(), &flow_state(&sources), link)?;
    record_events(runtime, &events, &repository.key, &located.flow)?;
    sources.events.extend(events);
    let state = flow_state(&sources);
    runtime.files.mkdir_all(dirname(&path)).map_err(CritiqueError::from_fs)?;
    runtime.files.write_text(&path, &format!("{}\n", json::pretty(&state_json(&state)))).map_err(CritiqueError::from_fs)?;
    Ok(result(&state, if saved { "refreshed" } else { "rebuilt" }, &path))
}

fn result(state: &FlowState, source: &str, path: &str) -> Value {
    json::object(vec![("next", next_step(state)), ("source", Value::str(source)), ("state", state_json(state)), ("state_path", Value::str(path)), ("status", Value::str("read"))])
}

/// The saved state, or `None` when it no longer reads as one, so it is rebuilt.
fn cached_state(runtime: &Runtime, path: &str) -> Option<FlowState> {
    runtime.files.read_text(path).ok().and_then(|text| parse_flow_state(&text).ok())
}

/// Where the draft's plan file lives by the repository's tracker convention, and whether it exists: a plan file marks
/// the draft as a refactor. A spec flow looks for the plan of the draft it grew from.
fn plan(runtime: &Runtime, located: &LocatedFlow<'_>, linked: Option<&str>) -> (Option<String>, bool) {
    let Some(name) = draft_of(Some(&located.flow)).or(linked) else { return (None, false) };
    let kind = match located.tracker {
        Tracker::Github(_) => "github",
        Tracker::LocalMarkdown(_) => "local-markdown",
    };
    // A draft name that names no folder has no plan file; prepare refuses it when a plan is asked for.
    match plan_file_path_or_null(&located.repository.root, &located.repository.common_dir, kind, name) {
        None => (None, false),
        Some(path) => {
            let exists = runtime.files.exists(&path).unwrap_or(false);
            (Some(path), exists)
        }
    }
}

struct Gather<'a> {
    critique_dir: &'a str,
    draft: Option<String>,
    keys: &'a [String],
    ledger: &'a Ledger,
    plan: (Option<String>, bool),
    tickets: &'a [TrackedTicket],
}

/// Whether a run line belongs to the flow: of its repository, and of one of its keys or of its spec.
fn belongs(repository: &str, keys: &[String], spec: Option<&str>, line_repository: &str, flow: Option<&str>, line_spec: Option<&str>) -> bool {
    line_repository == repository && (flow.is_some_and(|flow| keys.iter().any(|key| key == flow)) || (spec.is_some() && line_spec == spec))
}

fn sources(runtime: &Runtime, located: &LocatedFlow<'_>, gather: &Gather<'_>, invocations: &[crate::budget::LoggedInvocation]) -> Result<FlowSources, CritiqueError> {
    let repository = &located.repository.key;
    let spec = located.spec.as_ref().map(|spec| spec.reference.clone());
    let runs = runs(runtime, gather, repository, spec.as_deref())?;
    let reviews = index(runtime, &ticket_review_index_path(gather.critique_dir), parse_ticket_review_record)?
        .into_iter()
        .filter(|review| &review.repository == repository)
        .map(|review| SourceReview { degraded: review.degraded, recorded_at: review.recorded_at, review: review.review, run_id: review.run_id, ticket: review.ticket })
        .collect();
    let gates = index(runtime, &gate_index_path(gather.critique_dir), parse_gate_record)?
        .into_iter()
        .filter(|gate| &gate.repository == repository && Some(&gate.spec) == spec.as_ref())
        .map(|gate| SourceGate { gate: gate.gate, head_commit: gate.head_commit, line: gate.line, run_id: gate.run_id })
        .collect();
    let run_ids: Vec<&str> = runs.iter().map(|run| run.run_id.as_str()).collect();
    let findings = gather
        .ledger
        .findings
        .iter()
        .filter(|finding| run_ids.contains(&finding.context.run_id.as_str()))
        .map(|finding| FlowFinding {
            answered: finding.answered,
            finding_id: finding.finding_id.clone(),
            run_id: finding.context.run_id.clone(),
            severity: finding.severity.clone(),
            stage: finding.context.stage.clone(),
            verdict: finding.verdict.clone(),
        })
        .collect();
    let events = gather.ledger.flow_events.iter().filter(|event| &event.repository == repository && gather.keys.contains(&event.flow)).map(|event| event.event.clone()).collect();
    let head_commit = runtime.git.resolve_commit(&located.repository.root, "HEAD").map_err(CritiqueError::unexpected)?;
    let skipped_reviews = gather
        .ledger
        .skipped_reviews
        .iter()
        .filter(|skip| &skip.repository == repository)
        .map(|skip| SourceSkip { recorded_at: skip.recorded_at.clone(), run_id: skip.run_id.clone(), ticket: skip.ticket.clone() })
        .collect();
    let tickets = gather
        .tickets
        .iter()
        .map(|tracked| SourceTicket { blocked_by: tracked.ticket.blocked_by.clone(), id: tracked.ticket.id.clone(), open: tracked.open, reference: tracked.ticket.reference.clone(), title: tracked.ticket.title.clone() })
        .collect();
    let vibe_spend_usd = spend_by_flow(invocations, repository, gather.keys).iter().fold(0.0, |total, spend| total + spend.spent_usd);
    Ok(FlowSources {
        draft: gather.draft.clone(),
        events,
        findings,
        flow: located.flow.clone(),
        gates,
        head_commit,
        now: to_iso(runtime.clock.now_ms()),
        plan_exists: gather.plan.1,
        plan_path: gather.plan.0.clone(),
        repository: repository.clone(),
        reviews,
        runs,
        skipped_reviews,
        spec,
        tickets,
        vibe_spend_usd,
    })
}

/// The flow's runs, from the run directories (which hold runs not yet recorded, and recorded runs no critic took part
/// in) and from the ledger (which still holds runs whose directory is gone), in the order they were first seen.
fn runs(runtime: &Runtime, gather: &Gather<'_>, repository: &str, spec: Option<&str>) -> Result<Vec<FlowRun>, CritiqueError> {
    let mut runs: Vec<FlowRun> = Vec::new();
    for name in runtime.dirs.list_directories(gather.critique_dir) {
        let run_dir = join(&[gather.critique_dir, &name]);
        if !runtime.files.exists(&manifest_path(&run_dir)).unwrap_or(false) {
            continue;
        }
        let manifest = read_json_file(runtime, &manifest_path(&run_dir), "invalid-run", parse_manifest)?;
        if !belongs(repository, gather.keys, spec, &manifest.repository, Some(&manifest.flow), Some(&spec_reference(&manifest))) {
            continue;
        }
        let run = FlowRun {
            recorded: runtime.files.exists(&report_path(&run_dir)).unwrap_or(false),
            round: manifest.round,
            run_id: manifest.run_id.clone(),
            stage: manifest.stage.clone(),
            ticket_check: manifest.ticket_check.as_ref().map(|check| check.status.clone()),
        };
        match runs.iter_mut().find(|known| known.run_id == run.run_id) {
            Some(known) => *known = run,
            None => runs.push(run),
        }
    }
    for line in &gather.ledger.runs {
        let LedgerRunContext { repository: line_repository, flow, spec: line_spec, run_id, round, stage, .. } = &line.context;
        if !belongs(repository, gather.keys, spec, line_repository, flow.as_deref(), line_spec.as_deref()) {
            continue;
        }
        match runs.iter_mut().find(|known| &known.run_id == run_id) {
            Some(known) => known.recorded = true,
            None => runs.push(FlowRun { recorded: true, round: *round, run_id: run_id.clone(), stage: stage.clone(), ticket_check: None }),
        }
    }
    Ok(runs)
}

/// One JSON line per record, oldest first; a line that does not parse is an unreadable run record.
fn index<T>(runtime: &Runtime, path: &str, parse: fn(&str) -> Result<T, String>) -> Result<Vec<T>, CritiqueError> {
    if !runtime.files.exists(path).unwrap_or(false) {
        return Ok(Vec::new());
    }
    let text = runtime.files.read_text(path).map_err(CritiqueError::from_fs)?;
    text.split('\n').filter(|line| !js_trim(line).is_empty()).enumerate().map(|(n, line)| decoded("invalid-run", &format!("{path} line {}", n + 1), parse(line))).collect()
}

/// The events the input records, each checked against the state as it stood.
fn new_events(name: &FlowName, record: Option<&StateRecord>, state: &FlowState, link: bool) -> Result<Vec<FlowEvent>, CritiqueError> {
    let mut events = Vec::new();
    if link {
        events.push(FlowEvent::Draft(name.draft.clone().unwrap_or_default()));
    }
    let Some(record) = record else { return Ok(events) };
    if record.grilled {
        events.push(FlowEvent::Grilled);
    }
    if let Some(value) = record.no_critique {
        events.push(FlowEvent::NoCritique(value));
    }
    for named in &record.landed {
        let Some(ticket) = state.tickets.iter().find(|ticket| &ticket.id == named || &ticket.reference == named) else {
            return Err(CritiqueError::new("invalid-input", format!("state input: {named} is not a ticket of {}", state.flow)));
        };
        events.push(FlowEvent::Landed(ticket.reference.clone()));
    }
    for (finding_id, run_id) in &record.answered {
        let open = state.open_questions.iter().any(|question| &question.run_id == run_id && &question.finding_id == finding_id);
        if !open {
            return Err(CritiqueError::new("invalid-input", format!("state input: {finding_id} of run {run_id} is not an open question of {}", state.flow)));
        }
        events.push(FlowEvent::Answered { finding_id: finding_id.clone(), run_id: run_id.clone() });
    }
    Ok(events)
}

/// One flow event as the ledger holds it: `{ type: 'flow', ...context, ...event }`.
fn flow_event_line(event: &FlowEvent, flow: &str, recorded_at: &str, repository: &str) -> String {
    let mut line = Object::new();
    for (key, value) in [("type", Value::str("flow")), ("flow", Value::str(flow)), ("recorded_at", Value::str(recorded_at)), ("repository", Value::str(repository))] {
        line.set(key, value);
    }
    let fields: Vec<(&str, Value)> = match event {
        FlowEvent::Draft(draft) => vec![("draft", Value::str(draft)), ("event", Value::str("draft"))],
        FlowEvent::Grilled => vec![("event", Value::str("grilled"))],
        FlowEvent::NoCritique(value) => vec![("event", Value::str("no-critique")), ("value", Value::Bool(*value))],
        FlowEvent::Landed(ticket) => vec![("event", Value::str("landed")), ("ticket", Value::str(ticket))],
        FlowEvent::Answered { run_id, finding_id } => vec![("event", Value::str("answered")), ("finding_id", Value::str(finding_id)), ("run_id", Value::str(run_id))],
    };
    for (key, value) in fields {
        line.set(key, value);
    }
    json::compact(&Value::Obj(line))
}

fn record_events(runtime: &Runtime, events: &[FlowEvent], repository: &str, flow: &str) -> Result<(), CritiqueError> {
    if events.is_empty() {
        return Ok(());
    }
    let at = to_iso(runtime.clock.now_ms());
    let text: String = events.iter().map(|event| format!("{}\n", flow_event_line(event, flow, &at, repository))).collect();
    let path = ledger_path(&runtime.env)?;
    runtime.files.mkdir_all(dirname(&path)).map_err(CritiqueError::from_fs)?;
    runtime.files.append(&path, &text).map_err(CritiqueError::from_fs)
}
