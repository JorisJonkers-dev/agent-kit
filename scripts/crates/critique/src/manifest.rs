//! A prepared run's manifest: the packet it froze, the roster it resolved and the limits it runs under. The node
//! runner's `domain/manifest.ts`, its data types and its reader.

use crate::error::ShapeError;
use crate::family::{ALLOWLIST_FAMILIES, ORCHESTRATOR_FAMILIES};
use crate::fields::Fields;
use crate::limits::{read_limits, CritiqueLimits};
use crate::roster::{read_dropped_critic, read_roster_critic, DroppedCritic, RosterCritic};
use crate::stage::{CRITIQUE_STAGES, DEPTHS};
use model_gateway::json;

/// The repo map a code packet sends first, under `tree/`.
pub const REPO_MAP_PATH: &str = "tree/REPO-MAP.md";

/// The index of the facts a repository packet computed.
pub const FACTS_INDEX_PATH: &str = "facts/INDEX.md";

/// Where a repository's issues live.
pub const TRACKER_KINDS: [&str; 2] = ["github", "local-markdown"];

/// What a packet file is. A document stage freezes its `artifact` and its `reference`s. A code stage freezes the
/// `tree` at the commit under review, the `diff` against the base, the `ticket` or tickets the change delivers and
/// the `spec` they come from. The tickets stage freezes the `spec`, its `ticket`s and the ticket checker's report
/// (`check`). The decisions stage adds the `feature` the decisions under review are for. A repository packet adds a
/// sampled `tree` and the `fact`s the runner computed.
pub const PACKET_FILE_KINDS: [&str; 10] = ["artifact", "reference", "tree", "diff", "ticket", "spec", "check", "feature", "fact", "history"];

/// Where an artifact came from: a tracker, or `none` for the decisions stage's decision docs or decision summary,
/// which no tracker holds. It is `TRACKER_KINDS` plus `none`.
pub const ARTIFACT_ORIGINS: [&str; 3] = ["github", "local-markdown", "none"];

/// A run of lines of a file, at their own numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct LineRange {
    pub first: f64,
    pub last: f64,
}

/// A packet file: where it was frozen from, what it is, and its hash.
#[derive(Clone, Debug, PartialEq)]
pub struct PacketFile {
    /// Path inside the packet, which is the critic's working directory.
    pub path: String,
    /// Where it was frozen from: a repository path, an issue reference for a file read from GitHub, or
    /// `<base commit>..<head commit>` for a diff.
    pub source: String,
    pub kind: String,
    pub sha256: String,
    /// Only for a tree file a repository packet cut short: it holds lines 1 to `last` of the file's `lines`, at
    /// their own numbers.
    pub window: Option<PacketWindow>,
}

/// The lines of a file a packet sends, and how many the file has.
#[derive(Clone, Debug, PartialEq)]
pub struct PacketWindow {
    pub last: f64,
    pub lines: f64,
    /// Only when the file holds other lines than 1 to `last`: the runs of lines it holds, at their own numbers. A
    /// code packet sends slices this way.
    pub ranges: Option<Vec<LineRange>>,
}

/// The lines a windowed file holds: its ranges, or lines 1 to `last`.
pub fn held_ranges(window: &PacketWindow) -> Vec<LineRange> {
    window.ranges.clone().unwrap_or_else(|| vec![LineRange { first: 1.0, last: window.last }])
}

/// One context slice of a code packet: lines of a tree file, and why they are sent.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeSlice {
    /// The packet path, under `tree/`.
    pub path: String,
    pub first: f64,
    pub last: f64,
    /// Why: which symbol the change touches, or which changed symbol it calls or is called by.
    pub reason: String,
}

/// The commits a code stage reviewed.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeSource {
    /// The base as the caller named it.
    pub base: String,
    /// Where the change left the base: the merge base of the base and the head.
    pub base_commit: String,
    /// The commit under review as the caller named it.
    pub head: String,
    pub head_commit: String,
    /// The packet path, under `tree/`, of every file the change touched that is still in the tree and is not a
    /// generated, vendored or lock file.
    pub changed_files: Vec<String>,
    /// The context slices the packet sends, in the order they were chosen. A manifest written before slices existed
    /// has none.
    pub slices: Option<Vec<CodeSlice>>,
    /// Only when a ticket review reviewed the staged change: its hash, so a commit gate can tell whether what it is
    /// about to commit is what was reviewed.
    pub staged_diff: Option<String>,
}

/// Where the artifact under critique came from.
#[derive(Clone, Debug, PartialEq)]
pub struct ArtifactSource {
    pub tracker: String,
    /// `host/owner/repo#<number>` for a GitHub issue, the repository path for a markdown file, and `decision-docs`
    /// or `decision-summary` at the decisions stage.
    pub reference: String,
    /// The issue's labels when it was read; empty for a markdown file.
    pub labels: Vec<String>,
}

/// One measure of a depth choice: its value, and the threshold routing gave it.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasuredValue {
    pub value: f64,
    pub threshold: f64,
}

/// A depth signal that fired. Every field is configuration or a count, never artifact text, so the ledger may keep
/// it.
#[derive(Clone, Debug, PartialEq)]
pub enum DepthSignal {
    Threshold { measure: String, threshold: f64, value: f64 },
    RiskTerm { term: String },
    Label { label: String },
}

/// How the depth of a run was chosen, beside the depth itself.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthChoice {
    /// `argument` when the caller named the depth, which always wins; `signals` otherwise.
    pub chosen_by: String,
    /// Every configured measure with its value, fired or not, so the choice can be checked.
    pub measures: Vec<(String, MeasuredValue)>,
    /// The signals that fired; with `signals`, deep exactly when there is one.
    pub signals: Vec<DepthSignal>,
}

/// A ticket whose blocker names no ticket of the spec.
#[derive(Clone, Debug, PartialEq)]
pub struct DanglingBlocker {
    pub blocker: String,
    pub ticket: String,
}

/// What the ticket checker found when the packet was frozen.
#[derive(Clone, Debug, PartialEq)]
pub struct TicketCheckSummary {
    pub status: String,
    pub ticket_ids: Vec<String>,
    pub cycles: Vec<Vec<String>>,
    pub dangling_blockers: Vec<DanglingBlocker>,
    /// The user stories no ticket cites; `None` when coverage was not checkable.
    pub uncited_stories: Option<Vec<f64>>,
    /// Why coverage was not checkable, when it was not.
    pub coverage_reason: Option<String>,
}

/// The work roster's grant: the model-gateway workflow whose live grant let Codex run; a refute round asks that
/// grant again.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkRoster {
    pub grant_workflow: Option<String>,
}

/// Everything fan-out and record need to know about a prepared run.
#[derive(Clone, Debug, PartialEq)]
pub struct RunManifest {
    pub version: f64,
    pub run_id: String,
    pub created_at: String,
    pub stage: String,
    pub depth: String,
    pub round: f64,
    /// The first round's run id, on a second round.
    pub previous_run_id: Option<String>,
    /// How the depth was chosen, and the signals that decided it.
    pub depth_choice: DepthChoice,
    pub orchestrator_family: String,
    pub repository: String,
    pub repo_root: String,
    pub run_dir: String,
    pub packet_dir: String,
    pub allowed_families: Vec<String>,
    pub artifact: ArtifactSource,
    /// The flow this run belongs to: the spend of every run of one flow of one repository counts against one Vibe
    /// cap.
    pub flow: String,
    pub limits: CritiqueLimits,
    /// The model gateway's warning for each Vibe plan window at or above its warning ratio, read when the run was
    /// prepared; absent while Vibe is well inside its plan.
    pub plan_warnings: Option<Vec<String>>,
    pub packet_files: Vec<PacketFile>,
    /// Only for a code stage.
    pub code: Option<CodeSource>,
    /// Only for the tickets stage: what the ticket checker found when the packet was frozen.
    pub ticket_check: Option<TicketCheckSummary>,
    /// Whether no two critics of the stage may share a family; a fallback retry keeps to it too.
    pub distinct_families: bool,
    pub critics: Vec<RosterCritic>,
    pub dropped: Vec<DroppedCritic>,
    /// Present only for a `nedap` repository, whose critics and refuters are the work clients alone.
    pub work: Option<WorkRoster>,
}

/// The reference of the spec a run belongs to: its artifact's, unless the artifact is a ticket. A ticket review
/// critiques a ticket, so its spec is the source of the packet's spec file.
pub fn spec_reference(manifest: &RunManifest) -> String {
    let spec = if manifest.stage == "ticket-review" { manifest.packet_files.iter().find(|file| file.kind == "spec") } else { None };
    match spec {
        Some(file) => file.source.clone(),
        None => manifest.artifact.reference.clone(),
    }
}

/// The packet files a critic's prompt names. A document packet names every file. A code packet names its repo map
/// first, then the diff, the tickets, the spec and the facts index, and points at its context slices instead of
/// whole files; the rest of the tree is there for the critic to read by path.
pub fn listed_files(manifest: &RunManifest) -> Vec<PacketFile> {
    if manifest.code.is_none() {
        return manifest.packet_files.clone();
    }
    let of_kind = |kind: &str| manifest.packet_files.iter().filter(|file| file.kind == kind).cloned().collect::<Vec<_>>();
    let fact = |path: &str| of_kind("fact").into_iter().filter(|file| file.path == path).collect::<Vec<_>>();
    let mut listed = fact(REPO_MAP_PATH);
    listed.extend(of_kind("diff"));
    listed.extend(of_kind("ticket"));
    listed.extend(of_kind("spec"));
    listed.extend(fact(FACTS_INDEX_PATH));
    listed
}

/// Reads a run manifest. A JSON syntax error and a wrong shape both yield the message the node runner's `decoded`
/// prints after the file's path.
pub fn parse_manifest(text: &str) -> Result<RunManifest, String> {
    let value = json::parse(text)?;
    let file = Fields::of(&value, "").map_err(|error| error.0)?;
    read_manifest(&file).map_err(|error| error.0)
}

/// Reads a run manifest's fields, in the order the node runner evaluates them, so the first wrong field is the one
/// reported.
fn read_manifest(file: &Fields<'_>) -> Result<RunManifest, ShapeError> {
    let allowed_families = file.one_of_list("allowed_families", &ALLOWLIST_FAMILIES)?;
    let artifact = read_artifact_source(&file.object("artifact")?)?;
    let created_at = file.string("created_at")?;
    let critics = file.list("critics", |critic| read_roster_critic(critic))?;
    let depth = file.one_of("depth", &DEPTHS)?;
    let distinct_families = file.boolean("distinct_families")?;
    let depth_choice = read_depth_choice(&file.object("depth_choice")?)?;
    let dropped = file.list("dropped", |critic| read_dropped_critic(critic))?;
    let flow = file.string("flow")?;
    let limits = read_limits(&file.object("limits")?)?;
    let orchestrator_family = file.one_of("orchestrator_family", &ORCHESTRATOR_FAMILIES)?;
    let packet_dir = file.string("packet_dir")?;
    let code = if file.has("code") { Some(read_code_source(&file.object("code")?)?) } else { None };
    let work = if file.has("work") { Some(read_work_roster(&file.object("work")?)?) } else { None };
    let plan_warnings = if file.has("plan_warnings") { Some(file.strings("plan_warnings")?) } else { None };
    let packet_files = file.list("packet_files", |packet_file| read_packet_file(packet_file))?;
    let (round, previous_run_id) = read_round(file)?;
    let ticket_check = if file.has("ticket_check") { Some(read_ticket_check(&file.object("ticket_check")?)?) } else { None };
    let repo_root = file.string("repo_root")?;
    let repository = file.string("repository")?;
    let run_dir = file.string("run_dir")?;
    let run_id = file.string("run_id")?;
    let stage = file.one_of("stage", &CRITIQUE_STAGES)?;
    file.literal("version", 1.0)?;
    Ok(RunManifest {
        version: 1.0,
        run_id,
        created_at,
        stage,
        depth,
        round,
        previous_run_id,
        depth_choice,
        orchestrator_family,
        repository,
        repo_root,
        run_dir,
        packet_dir,
        allowed_families,
        artifact,
        flow,
        limits,
        plan_warnings,
        packet_files,
        code,
        ticket_check,
        distinct_families,
        critics,
        dropped,
        work,
    })
}

/// The work roster's grant, which a `nedap` repository's manifest carries.
fn read_work_roster(work: &Fields<'_>) -> Result<WorkRoster, ShapeError> {
    Ok(WorkRoster { grant_workflow: if work.has("grant_workflow") { Some(work.string("grant_workflow")?) } else { None } })
}

/// One packet file, with the window a repository packet cut it to.
fn read_packet_file(fields: &Fields<'_>) -> Result<PacketFile, ShapeError> {
    Ok(PacketFile {
        kind: fields.one_of("kind", &PACKET_FILE_KINDS)?,
        path: fields.string("path")?,
        sha256: fields.string("sha256")?,
        source: fields.string("source")?,
        window: if fields.has("window") { Some(read_window(&fields.object("window")?)?) } else { None },
    })
}

/// The lines of a file a packet sends.
fn read_window(window: &Fields<'_>) -> Result<PacketWindow, ShapeError> {
    Ok(PacketWindow {
        last: window.positive_integer("last")?,
        lines: window.positive_integer("lines")?,
        ranges: if window.has("ranges") { Some(window.list("ranges", |range| read_range(range))?) } else { None },
    })
}

/// A run of lines, at their own numbers.
fn read_range(range: &Fields<'_>) -> Result<LineRange, ShapeError> {
    Ok(LineRange { first: range.positive_integer("first")?, last: range.positive_integer("last")? })
}

/// A manifest written before rounds existed is a first round.
fn read_round(file: &Fields<'_>) -> Result<(f64, Option<String>), ShapeError> {
    if !file.has("round") {
        return Ok((1.0, None));
    }
    if file.number("round")? != 2.0 {
        file.literal("round", 1.0)?;
        return Ok((1.0, None));
    }
    Ok((2.0, Some(file.string("previous_run_id")?)))
}

/// What the ticket checker found, with the reason coverage was not checkable when it was not.
fn read_ticket_check(check: &Fields<'_>) -> Result<TicketCheckSummary, ShapeError> {
    let reason = check.optional_string("coverage_reason")?;
    Ok(TicketCheckSummary {
        cycles: check.string_list_list("cycles")?,
        dangling_blockers: check.list("dangling_blockers", |blocker| {
            Ok(DanglingBlocker { blocker: blocker.string("blocker")?, ticket: blocker.string("ticket")? })
        })?,
        status: check.one_of("status", &["passed", "failed"])?,
        ticket_ids: check.strings("ticket_ids")?,
        uncited_stories: if reason.is_none() { Some(check.numbers("uncited_stories")?) } else { None },
        coverage_reason: reason,
    })
}

/// The commits a code stage reviewed, with the slices it sent and the staged change it reviewed.
fn read_code_source(code: &Fields<'_>) -> Result<CodeSource, ShapeError> {
    Ok(CodeSource {
        base: code.string("base")?,
        base_commit: code.string("base_commit")?,
        changed_files: code.strings("changed_files")?,
        head: code.string("head")?,
        head_commit: code.string("head_commit")?,
        slices: if code.has("slices") { Some(code.list("slices", |slice| read_code_slice(slice))?) } else { None },
        staged_diff: if code.has("staged_diff") { Some(code.string("staged_diff")?) } else { None },
    })
}

/// One context slice: lines of a tree file, and why they are sent.
fn read_code_slice(slice: &Fields<'_>) -> Result<CodeSlice, ShapeError> {
    let range = read_range(slice)?;
    Ok(CodeSlice { path: slice.string("path")?, first: range.first, last: range.last, reason: slice.string("reason")? })
}

/// Where the artifact under critique came from.
fn read_artifact_source(artifact: &Fields<'_>) -> Result<ArtifactSource, ShapeError> {
    Ok(ArtifactSource {
        labels: artifact.strings("labels")?,
        reference: artifact.string("reference")?,
        tracker: artifact.one_of("tracker", &ARTIFACT_ORIGINS)?,
    })
}

/// How a run's depth was chosen, and the signals that decided it.
pub fn read_depth_choice(fields: &Fields<'_>) -> Result<DepthChoice, ShapeError> {
    Ok(DepthChoice {
        chosen_by: fields.one_of("chosen_by", &["argument", "signals"])?,
        measures: fields.entries("measures", |measure, _| {
            Ok(MeasuredValue { threshold: measure.number("threshold")?, value: measure.number("value")? })
        })?,
        signals: fields.list("signals", |signal| read_depth_signal(signal))?,
    })
}

/// One depth signal that fired: a measure past its threshold, a risk term, or a tracker label.
fn read_depth_signal(fields: &Fields<'_>) -> Result<DepthSignal, ShapeError> {
    let kind = fields.one_of("kind", &["threshold", "risk-term", "label"])?;
    match kind.as_str() {
        "threshold" => Ok(DepthSignal::Threshold {
            measure: fields.string("measure")?,
            threshold: fields.number("threshold")?,
            value: fields.number("value")?,
        }),
        "risk-term" => Ok(DepthSignal::RiskTerm { term: fields.string("term")? }),
        _ => Ok(DepthSignal::Label { label: fields.string("label")? }),
    }
}
