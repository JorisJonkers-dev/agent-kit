//! `prepare`: resolves the repository and its family allowlist, reads the artifact from the repository's tracker,
//! chooses the depth from the artifact (or takes the caller's), then resolves the roster, and only then freezes the
//! artifact and its decision docs into a packet. A refused repository, an unsupported tracker and an issue gh cannot
//! read all return before anything is written. A roster that drops every critic still prepares a run: fan-out and
//! record then report it degraded, so the flow goes on. The node runner's `application/prepare.ts`.

use crate::adr_packet::{read_adr_packet, AdrRequest, ADR_ARTIFACT};
use crate::allowlist::Entry;
use crate::architecture_packet::read_architecture_packet;
use crate::check_tickets::{read_checked_tickets, CheckedRead, CheckedTickets};
use crate::clients::client_readiness;
use crate::code_packet::{read_code_packet, ticket_file, CodeRequest, CodeSourceOut};
use crate::decision_docs_reader::resolve_decision_docs;
use crate::decisions_packet::{read_decisions_packet, DecisionsRequest};
use crate::depth::{choose_depth, DepthEvidence};
use crate::error::{CritiqueError, ShapeError};
use crate::family::ORCHESTRATOR_FAMILIES;
use crate::fields::Fields;
use crate::flow_budget::FlowBudget;
use crate::locations::{manifest_path, report_path};
use crate::manifest::parse_manifest;
use crate::packet_files::{FrozenWindow, Packet, PreparedRepository, RepositoryPacket, UnfrozenFile};
use crate::paths::{inside_repository, join};
use crate::plan::plan_limit;
use crate::plan_file::{draft_of, named_plan_path, plan_file_path};
use crate::plan_packet::{read_plan_packet, PlanRequest};
use crate::prepared_json::{critic_value, depth_choice_value, dropped_value, identity_value, limits_value};
use crate::quota::quota_reached;
use crate::recovery_packet::{read_recovery_packet, RecoveryRequest};
use crate::repository::{read_allowlist, refusal, resolve_for_roster, work_entry, Resolved, RosterResolution};
use crate::roster_resolve::{
    candidate_clients, resolve_roster, resolve_work_roster, work_candidate_clients, work_grant_client, Availability, Roster, RosterRequest, WorkGrant, WorkRosterRequest,
};
use crate::routing::{stage_routing, CriticClient, Routing};
use crate::routing_source::load_routing;
use crate::runtime::Runtime;
use crate::stage::{CRITIQUE_STAGES, DEPTHS};
use crate::stage_table::{flow_key, FlowKey};
use crate::staged_change::{snapshot, staged_input, StagedInputError};
use crate::tickets::{size_report, summarise_check, TicketSize};
use crate::tracker_resolver::{resolve_tracker, ArtifactRead, TrackedArtifact, Tracker, TrackerResolution};
use crate::triage::{risk_path_hits, CodeChange};
use model_gateway::grant_guard::GrantGuard;
use model_gateway::json::{self, Object, Value};
use model_gateway::run::compact_timestamp;
use model_gateway::sha256::sha256_hex;
use model_gateway::timestamps::to_iso;
use std::collections::HashMap;

struct PrepareInput {
    stage: String,
    orchestrator_family: String,
    repo: String,
    /// The artifact as its tracker names it: an issue reference for GitHub, a path for local markdown.
    spec: Option<String>,
    /// The model-gateway workflow whose grant lets Codex critique a `nedap` repository; the run's flow when not named.
    workflow: Option<String>,
    /// The decisions stage's feature statement, and its decision summary after `/grill-me`.
    feature: Option<String>,
    decision_summary: Option<String>,
    /// Whether a GitHub issue's comments are frozen with its body.
    comments: bool,
    /// The reference paths the caller names; `None` means the repository's decision docs.
    references: Option<Vec<String>>,
    /// An ADR review's records under review; `None` means every record.
    adrs: Option<Vec<String>>,
    /// The flow whose Vibe spend this run adds to.
    flow: Option<String>,
    /// The recorded first round, when this run is the second round of the same stage over the same artifact.
    previous_run_dir: Option<String>,
    /// The depth the caller names, which always wins over the signals.
    depth: Option<String>,
    /// A code stage's base, and its commit under review.
    base: Option<String>,
    head: String,
    /// Whether a ticket review reviews the staged change instead of a commit.
    staged: bool,
    /// The ticket a ticket review delivers.
    ticket: Option<String>,
    /// The plan stage's plan file; `None` means the one the draft's tracker convention names.
    plan: Option<String>,
}

/// Reads the input in the order the node runner evaluates its fields, so the first wrong field is the one reported.
fn read_input(raw: &Value) -> Result<PrepareInput, CritiqueError> {
    let shape = |error: ShapeError| error.into_error("invalid-input", "prepare input");
    let fields = Fields::of(raw, "").map_err(shape)?;
    let adrs = if fields.has("adrs") { Some(fields.strings("adrs").map_err(shape)?) } else { None };
    let base = fields.optional_string("base").map_err(shape)?;
    let comments = if fields.has("comments") { fields.boolean("comments").map_err(shape)? } else { false };
    let decision_summary = fields.optional_string("decision_summary").map_err(shape)?;
    let feature = fields.optional_string("feature").map_err(shape)?;
    let flow = fields.optional_string("flow").map_err(shape)?;
    let depth = if fields.has("depth") { Some(fields.one_of("depth", &DEPTHS).map_err(shape)?) } else { None };
    let head = fields.optional_string("head").map_err(shape)?.unwrap_or_else(|| "HEAD".to_string());
    let orchestrator_family = fields.one_of("orchestrator_family", &ORCHESTRATOR_FAMILIES).map_err(shape)?;
    let plan = fields.optional_string("plan").map_err(shape)?;
    let previous_run_dir = if fields.has("previous_run_dir") { Some(fields.string("previous_run_dir").map_err(shape)?) } else { None };
    let references = if fields.has("references") { Some(fields.strings("references").map_err(shape)?) } else { None };
    let repo = fields.string("repo").map_err(shape)?;
    let spec = fields.optional_string("spec").map_err(shape)?;
    let stage = fields.one_of("stage", &CRITIQUE_STAGES).map_err(shape)?;
    let staged = staged_input(&fields, &stage).map_err(|error| match error {
        StagedInputError::Shape(error) => shape(error),
        StagedInputError::Refused(error) => error,
    })?;
    let ticket = fields.optional_string("ticket").map_err(shape)?;
    let workflow = fields.optional_string("workflow").map_err(shape)?;
    Ok(PrepareInput { stage, orchestrator_family, repo, spec, workflow, feature, decision_summary, comments, references, adrs, flow, previous_run_dir, depth, base, head, staged, ticket, plan })
}

/// What the stage critiques, whose labels and text its depth signals read.
struct Artifact {
    labels: Vec<String>,
    reference: String,
    tracker: &'static str,
    text: String,
}

fn from_tracked(artifact: &TrackedArtifact) -> Artifact {
    Artifact { labels: artifact.labels.clone(), reference: artifact.reference.clone(), tracker: artifact.tracker, text: artifact.text.clone() }
}

/// What a stage freezes, before the routing decides who reads it.
struct Contents {
    artifact: Artifact,
    files: Vec<UnfrozenFile>,
    /// Only for a code stage.
    code: Option<CodeSourceOut>,
    change: Option<CodeChange>,
    /// Only for the tickets stage: what the ticket checker found.
    ticket_check: Option<CheckedTickets>,
    /// The spec's reference when the artifact is not the spec itself: a ticket review's.
    spec_reference: Option<String>,
    adrs: Option<usize>,
    source_lines: Option<usize>,
    context_maps: Option<usize>,
    steps: Option<usize>,
    stored_data_steps: Option<usize>,
    holds_adr: bool,
    repository: Option<PreparedRepository>,
    /// Only for a code stage: how many files of the tree the cap left out, and the cap.
    sample: Option<(usize, f64)>,
}

impl Contents {
    fn of(artifact: Artifact, files: Vec<UnfrozenFile>) -> Contents {
        Contents {
            artifact,
            files,
            code: None,
            change: None,
            ticket_check: None,
            spec_reference: None,
            adrs: None,
            source_lines: None,
            context_maps: None,
            steps: None,
            stored_data_steps: None,
            holds_adr: false,
            repository: None,
            sample: None,
        }
    }

    fn of_repository(packet: RepositoryPacket) -> Contents {
        let artifact = Artifact { labels: Vec::new(), reference: packet.reference, tracker: "none", text: packet.artifact_text };
        Contents {
            adrs: packet.adrs,
            context_maps: packet.context_maps,
            repository: Some(packet.repository),
            source_lines: packet.source_lines,
            steps: packet.steps,
            stored_data_steps: packet.stored_data_steps,
            ..Contents::of(artifact, packet.files)
        }
    }
}

enum Read {
    Contents(Box<Contents>),
    Refused(Value),
}

fn read(contents: Contents) -> Result<Read, CritiqueError> {
    Ok(Read::Contents(Box::new(contents)))
}

/// What a code stage's prepare reports about the commits it froze.
fn prepared_code_value(code: &CodeSourceOut, files: &[UnfrozenFile], cap_bytes: f64, dropped_files: usize) -> Value {
    json::object(vec![
        ("base_commit", Value::str(&code.base_commit)),
        ("cap_bytes", Value::Num(cap_bytes)),
        ("changed_files", strings_value(&code.changed_files)),
        ("dropped_files", Value::count(dropped_files)),
        ("head_commit", Value::str(&code.head_commit)),
        ("packet_bytes", Value::count(files.iter().map(|file| file.text.len()).sum())),
        ("tree_files", Value::count(files.iter().filter(|file| file.kind == "tree").count())),
    ])
}

fn strings_value(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

fn repository_value(repository: &PreparedRepository) -> Value {
    json::object(vec![
        ("cap_bytes", Value::Num(repository.cap_bytes)),
        ("dropped_files", Value::count(repository.dropped_files)),
        ("head_commit", Value::str(&repository.head_commit)),
        ("packet_bytes", Value::count(repository.packet_bytes)),
        ("tree_files", Value::count(repository.tree_files)),
    ])
}

fn needs_code_permission(stage: &str) -> bool {
    matches!(stage, "ticket-review" | "feature-review" | "decision-recovery" | "architecture-review" | "plan")
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let input = read_input(raw)?;
    let stage = input.stage.clone();
    if flow_key(&stage) == FlowKey::Run && input.flow.is_some() {
        return Err(CritiqueError::new("invalid-input", format!("{stage} is a review stage: each run is a flow of its own, so it takes no flow")));
    }
    let (repository, work) = match resolve_for_roster(runtime, &input.repo)? {
        RosterResolution::Refused(refusal) => return Ok(refusal),
        RosterResolution::Resolved(repository, work) => (repository, work),
    };
    // A nedap repository has the work-only roster whatever the allowlist says, so it needs no entry.
    let entry: Entry = if work {
        work_entry()
    } else {
        match read_allowlist(runtime)?.get(&repository.key) {
            Some(entry) => entry.clone(),
            None => return Ok(refusal("not-opted-in", &format!("{} has no family allowlist entry on this machine; record one with allow", repository.key))),
        }
    };
    if needs_code_permission(&stage) && !entry.code {
        return Ok(refusal(
            "code-not-allowed",
            &format!("{} allows documents but not code; record the code permission with allow and \"code\": true", repository.key),
        ));
    }
    let snapshot = if input.staged { Some(snapshot(runtime, &repository.root)?) } else { None };
    let head = snapshot.as_ref().map_or(input.head.clone(), |snapshot| snapshot.commit.clone());
    let mut contents = match contents(runtime, &repository, &input, &head, &entry)? {
        Read::Refused(refusal) => return Ok(refusal),
        Read::Contents(contents) => *contents,
    };
    let staged_diff = snapshot.as_ref().map(|snapshot| snapshot.staged.sha256.clone());
    let round = match round(runtime, &input, &repository, &stage, &contents.artifact.reference)? {
        Round::Refused(refusal) => return Ok(refusal),
        Round::Taken(round) => round,
    };
    let routing = load_routing(runtime)?;
    let change_measure = match &contents.change {
        None => None,
        Some(change) => {
            let risk_paths: &[String] = if let Some(triage) = &routing.triage { &triage.risk_paths } else { &[] };
            let hits = risk_path_hits(change, risk_paths).map_err(CritiqueError::unexpected)?;
            Some((change.changed_lines, hits as f64))
        }
    };
    let stage_routes = stage_routing(&routing, &stage)?;
    let check_text = contents.ticket_check.as_ref().map(|_| contents.files.iter().map(|file| file.text.as_str()).collect::<Vec<_>>().join("\n\n"));
    let evidence = match (&contents.ticket_check, &check_text) {
        (Some(check), Some(text)) => DepthEvidence { text, labels: &contents.artifact.labels, tickets: Some(check.tickets.len()), ..DepthEvidence::default() },
        _ => DepthEvidence {
            text: &contents.artifact.text,
            labels: &contents.artifact.labels,
            change: change_measure,
            tickets: None,
            adrs: contents.adrs,
            source_lines: contents.source_lines,
            context_maps: contents.context_maps,
            plan_steps: contents.steps,
            stored_data_steps: contents.stored_data_steps,
        },
    };
    let (depth, choice) = choose_depth(&stage, &stage_routes.depth_signals, &evidence, input.depth.as_deref());
    let grant = if work { Some(work_grant(runtime, &input, &work_grant_client(&routing, &stage)?)) } else { None };
    let allowed: Vec<String> = entry.families.clone();
    let roster = match &grant {
        Some(grant) => work_roster(runtime, &routing, &input, &stage, grant)?,
        None => open_roster(runtime, &routing, &RosterRequest { stage: &stage, depth: &depth, allowed_families: &allowed, orchestrator_family: &input.orchestrator_family, adr: contents.holds_adr })?,
    };
    let plan_warnings = if roster.critics.iter().any(|critic| matches!(critic.client_config, CriticClient::Vibe(_))) { plan_limit(runtime, "vibe")?.warnings } else { Vec::new() };
    let run_id = fresh_run_id(runtime, &repository, &stage);
    let run_dir = join(&[&repository.common_dir, "critique", &run_id]);
    let packet_dir = join(&[&run_dir, "packet"]);
    if let Some(check) = &contents.ticket_check {
        contents.files.push(check_report_file(check, &stage_routes.ticket_size));
    }
    let packet_files = freeze_packet(runtime, &packet_dir, &contents.files)?;
    let flow = match (&input.flow, &round.previous_flow) {
        (Some(flow), _) => flow.clone(),
        (None, Some(flow)) => flow.clone(),
        (None, None) if flow_key(&stage) == FlowKey::Spec => contents.spec_reference.clone().unwrap_or_else(|| contents.artifact.reference.clone()),
        (None, None) => run_id.clone(),
    };
    let artifact_value = json::object(vec![
        ("labels", strings_value(&contents.artifact.labels)),
        ("reference", Value::str(&contents.artifact.reference)),
        ("tracker", Value::str(contents.artifact.tracker)),
    ]);
    let mut manifest = Object::new();
    manifest.set("allowed_families", strings_value(&entry.families));
    manifest.set("artifact", artifact_value.clone());
    if let Some(code) = &contents.code {
        manifest.set("code", code_value(code, staged_diff.as_deref()));
    }
    manifest.set("created_at", Value::Str(to_iso(runtime.clock.now_ms())));
    manifest.set("critics", Value::Arr(roster.critics.iter().map(critic_value).collect()));
    manifest.set("depth", Value::str(&roster.depth));
    manifest.set("distinct_families", Value::Bool(roster.distinct_families));
    manifest.set("depth_choice", depth_choice_value(&choice));
    manifest.set("dropped", Value::Arr(roster.dropped.iter().map(dropped_value).collect()));
    manifest.set("flow", Value::Str(flow));
    manifest.set("limits", limits_value(&routing.limits));
    manifest.set("orchestrator_family", Value::str(&input.orchestrator_family));
    manifest.set("packet_dir", Value::str(&packet_dir));
    manifest.set("packet_files", Value::Arr(packet_files.iter().map(|file| file.value.clone()).collect()));
    if !plan_warnings.is_empty() {
        manifest.set("plan_warnings", strings_value(&plan_warnings));
    }
    manifest.set("repo_root", Value::str(&repository.root));
    manifest.set("repository", Value::str(&repository.key));
    if let Some(grant) = &grant {
        let work = match grant {
            WorkGrant::Live { workflow } => json::object(vec![("grant_workflow", Value::str(workflow))]),
            WorkGrant::Missing { .. } => json::object(Vec::new()),
        };
        manifest.set("work", work);
    }
    if let Some(previous) = &round.previous_run_id {
        manifest.set("previous_run_id", Value::str(previous));
    }
    manifest.set("round", Value::Num(round.round));
    manifest.set("run_dir", Value::str(&run_dir));
    manifest.set("run_id", Value::str(&run_id));
    manifest.set("stage", Value::str(&stage));
    if let Some(check) = &contents.ticket_check {
        manifest.set("ticket_check", summarise_check(&check.check));
    }
    manifest.set("version", Value::Num(1.0));
    runtime.files.write_text(&manifest_path(&run_dir), &format!("{}\n", json::pretty(&Value::Obj(manifest)))).map_err(CritiqueError::from_fs)?;
    let mut result = Object::new();
    result.set("artifact", artifact_value);
    if let (Some(code), Some((dropped, cap))) = (&contents.code, contents.sample) {
        result.set("code", prepared_code_value(code, &contents.files, cap, dropped));
    }
    if let Some(repository) = &contents.repository {
        result.set("repository", repository_value(repository));
    }
    result.set("critics", Value::Arr(roster.critics.iter().map(identity_value).collect()));
    result.set("depth", Value::str(&roster.depth));
    result.set("depth_choice", depth_choice_value(&choice));
    result.set("dropped", Value::Arr(roster.dropped.iter().map(dropped_value).collect()));
    result.set("history", Value::Bool(entry.history));
    result.set("manifest_path", Value::str(&manifest_path(&run_dir)));
    result.set("packet_dir", Value::str(&packet_dir));
    result.set("packet_files", strings_value(&listed_paths(&packet_files, contents.code.is_some())));
    result.set("round", Value::Num(round.round));
    result.set("run_dir", Value::str(&run_dir));
    result.set("run_id", Value::str(&run_id));
    result.set("status", Value::str("ready"));
    Ok(Value::Obj(result))
}

/// The packet files a critic's prompt names. A document packet names every file. A code packet names its repo map
/// first, then the diff, the tickets, the spec and the facts index, and points at its context slices instead of whole
/// files; the rest of the tree is there for the critic to read by path.
fn listed_paths(files: &[Frozen], code: bool) -> Vec<String> {
    if !code {
        return files.iter().map(|file| file.path.clone()).collect();
    }
    let of_kind = |kind: &str, path: Option<&str>| -> Vec<String> {
        files.iter().filter(|file| file.kind == kind && path.is_none_or(|path| file.path == path)).map(|file| file.path.clone()).collect()
    };
    [of_kind("fact", Some("tree/REPO-MAP.md")), of_kind("diff", None), of_kind("ticket", None), of_kind("spec", None), of_kind("fact", Some("facts/INDEX.md"))].concat()
}

/// What a code stage froze of the commits, as the manifest holds it.
fn code_value(code: &CodeSourceOut, staged_diff: Option<&str>) -> Value {
    let slices = code.slices.iter().map(|slice| {
        json::object(vec![("first", Value::count(slice.first)), ("last", Value::count(slice.last)), ("path", Value::str(&slice.path)), ("reason", Value::str(&slice.reason))])
    });
    let mut pairs = vec![
        ("base", Value::str(&code.base)),
        ("base_commit", Value::str(&code.base_commit)),
        ("changed_files", strings_value(&code.changed_files)),
        ("head", Value::str(&code.head)),
        ("head_commit", Value::str(&code.head_commit)),
        ("slices", Value::Arr(slices.collect())),
    ];
    if let Some(staged) = staged_diff {
        pairs.push(("staged_diff", Value::str(staged)));
    }
    json::object(pairs)
}

/// What the stage freezes. The decisions stage reads no tracker: grilling comes before any spec, and neither does an
/// ADR review, which samples the tree beside its records only when the repository allows code, and decision recovery
/// reads the tracker only for the issues its history cites. Every other stage reads its spec, or its ticket, from the
/// repository's tracker.
fn contents(runtime: &Runtime, repository: &Resolved, input: &PrepareInput, head: &str, entry: &Entry) -> Result<Read, CritiqueError> {
    let stage = input.stage.as_str();
    let root = repository.root.as_str();
    match stage {
        "decisions" => {
            let request = DecisionsRequest { feature: input.feature.as_deref(), summary: input.decision_summary.as_deref(), references: input.references.as_deref() };
            match read_decisions_packet(runtime, root, &request)? {
                Packet::Refused(refusal) => Ok(Read::Refused(refusal)),
                Packet::Read(packet) => {
                    let artifact = Artifact { labels: Vec::new(), reference: packet.reference.to_string(), tracker: "none", text: packet.artifact_text };
                    read(Contents { holds_adr: packet.holds_adr, ..Contents::of(artifact, packet.files) })
                }
            }
        }
        "adr-review" => {
            let routing = if entry.code { Some(load_routing(runtime)?) } else { None };
            let code = routing.as_ref().map(|routing| (head, &routing.packet));
            match read_adr_packet(runtime, root, &AdrRequest { adrs: input.adrs.as_deref(), references: input.references.as_deref(), code })? {
                Packet::Refused(refusal) => Ok(Read::Refused(refusal)),
                Packet::Read(packet) => {
                    let artifact = Artifact { labels: Vec::new(), reference: ADR_ARTIFACT.to_string(), tracker: "none", text: packet.artifact_text };
                    read(Contents { adrs: Some(packet.adrs), repository: packet.repository, ..Contents::of(artifact, packet.files) })
                }
            }
        }
        "architecture-review" => match read_architecture_packet(runtime, root, head, &load_routing(runtime)?.packet)? {
            Packet::Refused(refusal) => Ok(Read::Refused(refusal)),
            Packet::Read(packet) => read(Contents::of_repository(packet)),
        },
        "plan" => plan_contents(runtime, repository, input, head),
        "decision-recovery" => {
            // The tracker is read only for the issues the history cites, so a tracker it cannot read leaves them out and refuses nothing.
            let resolved = if entry.history { Some(resolve_tracker(runtime, repository)?) } else { None };
            let tracker = match &resolved {
                Some(TrackerResolution::Resolved(tracker)) => Some(tracker),
                _ => None,
            };
            let routing = load_routing(runtime)?;
            let history = if entry.history { Some(tracker) } else { None };
            match read_recovery_packet(runtime, root, &RecoveryRequest { head, limits: &routing.packet, history })? {
                Packet::Refused(refusal) => Ok(Read::Refused(refusal)),
                Packet::Read(packet) => read(Contents::of_repository(packet)),
            }
        }
        _ => {
            let tracker = match resolve_tracker(runtime, repository)? {
                TrackerResolution::Refused(refusal) => return Ok(Read::Refused(refusal)),
                TrackerResolution::Resolved(tracker) => tracker,
            };
            let Some(spec) = input.spec.as_deref() else {
                return Err(CritiqueError::new("invalid-input", format!("{stage} needs spec: the spec, named the way its tracker names it")));
            };
            match stage {
                "ticket-review" | "feature-review" => code_contents(runtime, repository, &tracker, input, spec, head),
                "tickets" => tickets_contents(runtime, &tracker, input, spec),
                _ => document_contents(runtime, repository, &tracker, input, spec),
            }
        }
    }
}

fn code_contents(runtime: &Runtime, repository: &Resolved, tracker: &Tracker, input: &PrepareInput, spec: &str, head: &str) -> Result<Read, CritiqueError> {
    let routing = load_routing(runtime)?;
    let request = CodeRequest { stage: &input.stage, spec, ticket: input.ticket.as_deref(), base: input.base.as_deref(), head, comments: input.comments, limits: &routing.packet };
    match read_code_packet(runtime, repository, tracker, &request)? {
        Packet::Refused(refusal) => Ok(Read::Refused(refusal)),
        Packet::Read(packet) => read(Contents {
            code: Some(packet.code),
            change: Some(packet.change),
            spec_reference: Some(packet.spec_reference),
            sample: Some((packet.dropped_files, packet.cap_bytes)),
            ..Contents::of(from_tracked(&packet.artifact), packet.files)
        }),
    }
}

/// The plan stage's packet. The plan is the file the caller names, else the one its draft's tracker convention puts
/// it in, named by the flow key `draft:<name>`; no spec exists yet, so nothing else is read.
fn plan_contents(runtime: &Runtime, repository: &Resolved, input: &PrepareInput, head: &str) -> Result<Read, CritiqueError> {
    let draft = draft_of(input.flow.as_deref());
    if input.plan.is_none() && draft.is_none() {
        return Err(CritiqueError::new(
            "invalid-input",
            "plan needs plan: the plan file, or flow: the draft flow draft:<name> whose plan file the tracker convention names",
        ));
    }
    let path = match &input.plan {
        None => {
            let kind = match resolve_tracker(runtime, repository)? {
                TrackerResolution::Refused(refusal) => return Ok(Read::Refused(refusal)),
                TrackerResolution::Resolved(Tracker::Github(_)) => "github",
                TrackerResolution::Resolved(Tracker::LocalMarkdown(_)) => "local-markdown",
            };
            plan_file_path(&repository.root, &repository.common_dir, kind, draft.unwrap_or_default())?
        }
        Some(plan) => named_plan_path(&repository.root, &repository.common_dir, plan)?,
    };
    let routing = load_routing(runtime)?;
    match read_plan_packet(runtime, &repository.root, &PlanRequest { path: &path, head, limits: &routing.packet })? {
        Packet::Refused(refusal) => Ok(Read::Refused(refusal)),
        Packet::Read(packet) => read(Contents::of_repository(packet)),
    }
}

/// A document stage's packet: the artifact goes to `artifact/` and each reference to `references/` under its
/// repository path. Nothing else is ever put at the packet root, so a `.vibe/`, `.agents/` or `AGENTS.md` there can
/// never become critic config.
fn document_contents(runtime: &Runtime, repository: &Resolved, tracker: &Tracker, input: &PrepareInput, spec: &str) -> Result<Read, CritiqueError> {
    let artifact = match tracker.artifact(runtime, spec, input.comments)? {
        ArtifactRead::Refused(refusal) => return Ok(Read::Refused(refusal)),
        ArtifactRead::Read(artifact) => artifact,
    };
    let references: Vec<String> = match &input.references {
        None => resolve_decision_docs(runtime, &repository.root)?,
        Some(paths) => paths.iter().map(|path| inside_repository(&repository.root, path)).collect::<Result<_, _>>()?,
    };
    let mut files = vec![UnfrozenFile::new("artifact", &join(&["artifact", &artifact.name]), &artifact.reference, &artifact.text)];
    for source in &references {
        let text = runtime.files.read_text(&join(&[&repository.root, source])).map_err(CritiqueError::from_fs)?;
        files.push(UnfrozenFile::new("reference", &join(&["references", source]), source, &text));
    }
    read(Contents::of(from_tracked(&artifact), files))
}

/// The tickets stage's packet: the spec under `spec/` and each of its tickets under `tickets/`, as a code stage
/// freezes them, read through the tracker the way check-tickets reads them. A spec without tickets is refused: there
/// is nothing to check yet.
fn tickets_contents(runtime: &Runtime, tracker: &Tracker, input: &PrepareInput, spec: &str) -> Result<Read, CritiqueError> {
    let checked = match read_checked_tickets(runtime, tracker, spec, input.comments)? {
        CheckedRead::Refused(refusal) => return Ok(Read::Refused(refusal)),
        CheckedRead::Read(checked) => checked,
    };
    if checked.tickets.is_empty() {
        return Ok(Read::Refused(refusal("no-tickets", &format!("{} has no tickets yet; cut them with /to-tickets first", checked.spec.reference))));
    }
    let mut files = vec![UnfrozenFile::new("spec", &join(&["spec", &checked.spec.name]), &checked.spec.reference, &checked.spec.text)];
    files.extend(checked.tickets.iter().map(|tracked| ticket_file(&tracked.name, &tracked.ticket.reference, &tracked.ticket.text)));
    let artifact = from_tracked(&checked.spec);
    read(Contents { ticket_check: Some(checked), ..Contents::of(artifact, files) })
}

/// The checker's report as the ticket linter reads it, with the size limits routing sets.
fn check_report_file(check: &CheckedTickets, size: &TicketSize) -> UnfrozenFile {
    let report = check.result_with(vec![("size", size_report(&check.check, size))]);
    UnfrozenFile::new("check", "check/tickets.json", &format!("check-tickets {}", check.spec.reference), &format!("{}\n", json::pretty(&report)))
}

/// The round a run is: the first, or the second after the recorded first round named by `previous_run_dir`. A run that
/// was itself a second round is refused, so a stage never runs a third time over one artifact.
struct RoundOf {
    round: f64,
    previous_run_id: Option<String>,
    /// A second round's first-round flow, so both rounds count against one Vibe cap.
    previous_flow: Option<String>,
}

enum Round {
    Taken(RoundOf),
    Refused(Value),
}

fn round(runtime: &Runtime, input: &PrepareInput, repository: &Resolved, stage: &str, reference: &str) -> Result<Round, CritiqueError> {
    let Some(previous_dir) = &input.previous_run_dir else {
        return Ok(Round::Taken(RoundOf { round: 1.0, previous_run_id: None, previous_flow: None }));
    };
    let previous = crate::decode::read_json_file(runtime, &manifest_path(previous_dir), "invalid-run", parse_manifest)?;
    if previous.round == 2.0 {
        return Ok(Round::Refused(refusal("round-limit", &format!("run {} was already the second round; a stage runs at most two rounds", previous.run_id))));
    }
    if !runtime.files.exists(&report_path(previous_dir)).map_err(CritiqueError::from_fs)? {
        return Err(CritiqueError::new("invalid-run", format!("run {} is not recorded yet; record it before its second round", previous.run_id)));
    }
    if previous.repository != repository.key || previous.stage != stage || previous.artifact.reference != reference {
        return Err(CritiqueError::new(
            "invalid-input",
            format!("run {} critiqued {} at the {} stage, not {reference}", previous.run_id, previous.artifact.reference, previous.stage),
        ));
    }
    Ok(Round::Taken(RoundOf { round: 2.0, previous_run_id: Some(previous.run_id), previous_flow: Some(previous.flow) }))
}

/// Whether a live grant covers the run's workflow for the stage's Codex client (named by routing), or what is missing.
fn work_grant(runtime: &Runtime, input: &PrepareInput, client: &str) -> WorkGrant {
    let Some(workflow) = input.workflow.as_ref().or(input.flow.as_ref()) else {
        return WorkGrant::Missing { detail: "name the model-gateway workflow with workflow so a grant for it can be found".to_string() };
    };
    match (GrantGuard { runtime: &runtime.gateway }).live(workflow, client, runtime.clock.now_ms()) {
        Ok(Some(_)) => WorkGrant::Live { workflow: workflow.clone() },
        Ok(None) => WorkGrant::Missing { detail: format!("no live grant covers workflow {workflow} for {client}: ask the owner, then write one with model-gateway grant write") },
        Err(error) => WorkGrant::Missing { detail: format!("the grants could not be read, so {client} is left out: {}", error.message) },
    }
}

/// A `nedap` repository's roster: only the work clients, each asked whether it can run here once the family and grant
/// rules let it.
fn work_roster(runtime: &Runtime, routing: &Routing, input: &PrepareInput, stage: &str, grant: &WorkGrant) -> Result<Roster, CritiqueError> {
    let request = WorkRosterRequest { stage, orchestrator_family: &input.orchestrator_family, grant };
    let mut readiness: HashMap<String, Availability> = HashMap::new();
    for (name, client) in work_candidate_clients(routing, &request)? {
        let ready = client_readiness(runtime, client)?;
        readiness.insert(name, Availability { ready: ready.ready, detail: ready.detail.unwrap_or_default(), reason: None });
    }
    resolve_work_roster(routing, &request, &readiness)
}

/// Asks each client the stage could run, and that the family rules permit, whether it can run here, holds an agy
/// client that can to the day's Antigravity quota, and holds a Vibe client that can to Vibe's plan limit: at 100% of
/// any window it counts as unable to start, so it is dropped the way an agy past its quota is.
fn open_roster(runtime: &Runtime, routing: &Routing, request: &RosterRequest) -> Result<Roster, CritiqueError> {
    let agy_today = FlowBudget::new(runtime).agy_invocations_today()?;
    let plan = plan_limit(runtime, "vibe")?.unable_to_start;
    let mut readiness: HashMap<String, Availability> = HashMap::new();
    for (name, client) in candidate_clients(routing, request)? {
        let ready = client_readiness(runtime, client)?;
        let limit = match client {
            CriticClient::Agy(_) if ready.ready => quota_reached(agy_today, &routing.limits.antigravity_quota, request.stage),
            CriticClient::Vibe(_) if ready.ready => plan.clone(),
            _ => None,
        };
        let availability = match limit {
            Some(detail) => Availability { ready: false, detail, reason: Some("quota") },
            None => Availability { ready: ready.ready, detail: ready.detail.unwrap_or_default(), reason: None },
        };
        readiness.insert(name, availability);
    }
    resolve_roster(routing, request, &readiness)
}

fn fresh_run_id(runtime: &Runtime, repository: &Resolved, stage: &str) -> String {
    let base = format!("{}-{stage}", compact_timestamp(runtime.clock.now_ms()));
    let mut candidate = base.clone();
    let mut n = 2;
    while runtime.files.exists(&manifest_path(&join(&[&repository.common_dir, "critique", &candidate]))).unwrap_or(false) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
}

/// One frozen packet file, as the manifest lists it.
struct Frozen {
    kind: &'static str,
    path: String,
    value: Value,
}

fn window_value(window: &FrozenWindow) -> Value {
    let mut pairs = vec![("last", Value::count(window.last)), ("lines", Value::count(window.lines))];
    if let Some(ranges) = &window.ranges {
        let ranges = ranges.iter().map(|range| {
            let mut object = Object::new();
            object.set("first", Value::count(range.first));
            object.set("last", Value::count(range.last));
            for (key, value) in &range.extra {
                object.set(key, Value::str(value));
            }
            Value::Obj(object)
        });
        pairs.push(("ranges", Value::Arr(ranges.collect())));
    }
    json::object(pairs)
}

fn freeze_packet(runtime: &Runtime, packet_dir: &str, files: &[UnfrozenFile]) -> Result<Vec<Frozen>, CritiqueError> {
    let mut frozen = Vec::new();
    for file in files {
        runtime.files.write_text(&join(&[packet_dir, &file.path]), &file.text).map_err(CritiqueError::from_fs)?;
        let mut pairs = vec![("kind", Value::str(file.kind)), ("path", Value::str(&file.path)), ("source", Value::str(&file.source))];
        if let Some(window) = &file.window {
            pairs.push(("window", window_value(window)));
        }
        pairs.push(("sha256", Value::Str(sha256_hex(file.text.as_bytes()))));
        frozen.push(Frozen { kind: file.kind, path: file.path.clone(), value: json::object(pairs) });
    }
    Ok(frozen)
}
