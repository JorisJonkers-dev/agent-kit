//! What the runner does differently from one stage to the next, as far as the flow budget, the review list and stats
//! read it: whose flow a run charges, and how a stage is judged for removal. The node runner's `domain/stage-table.ts`.

/// The review stages: run on demand over a repository, outside any feature's flow, each run a flow of its own keyed by
/// its run id.
pub const REVIEW_STAGES: [&str; 3] = ["adr-review", "decision-recovery", "architecture-review"];

pub fn is_review_stage(stage: &str) -> bool {
    REVIEW_STAGES.contains(&stage)
}

/// The flow key a stage's run charges, unless the caller names one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlowKey {
    /// The draft's key: the run starts a flow of its own before a tracker item exists.
    Draft,
    /// The spec's reference, so the run counts with the spec's other stages.
    Spec,
    /// A review stage's run is a flow of its own, keyed by its run id.
    Run,
}

/// How `/critique-stats` judges a stage for removal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RemovalRule {
    /// By its latest features, the distinct specs it ran on.
    Features,
    /// By its latest runs, since a review stage belongs to no feature.
    Runs,
}

pub fn flow_key(stage: &str) -> FlowKey {
    match stage {
        _ if is_review_stage(stage) => FlowKey::Run,
        "plan" | "decisions" => FlowKey::Draft,
        _ => FlowKey::Spec,
    }
}

pub fn removal_rule(stage: &str) -> RemovalRule {
    if is_review_stage(stage) {
        RemovalRule::Runs
    } else {
        RemovalRule::Features
    }
}

/// How many recovered candidates one run asks the owner about; the rest are deferred in the report.
pub const RECOVERY_QUESTION_CAP: usize = 10;

/// Whether more than three accepted critical findings of the stage call for a second round. The decisions stage never
/// does, nor do the stages whose accepted findings revise no artifact.
pub fn second_round(stage: &str) -> bool {
    !matches!(stage, "architecture-review" | "decision-recovery" | "decisions")
}

/// For a stage whose findings are candidates for the owner to confirm: the most candidates asked at once.
pub fn candidate_questions(stage: &str) -> Option<usize> {
    matches!(stage, "architecture-review" | "decision-recovery").then_some(RECOVERY_QUESTION_CAP)
}

/// Whether the stage's recovered candidates sit among findings that may be accepted: only a finding of category
/// `undocumented-decision` is a candidate then.
pub fn mixed_candidates(stage: &str) -> bool {
    stage == "architecture-review"
}

/// Whether the stage's findings are judged by the architecture evidence rule, which reports its accepted findings as
/// candidates.
pub fn architecture_evidence(stage: &str) -> bool {
    stage == "architecture-review"
}

/// Whether the stage freezes the repository's decision records as its packet.
pub fn adrs_packet(stage: &str) -> bool {
    stage == "adr-review"
}

/// The command the report says not to start while a critical question is open or the ticket graph has a cycle; `None`
/// for a review stage, which sits before no command of a feature's flow.
pub fn do_not_start(stage: &str) -> Option<&'static str> {
    match stage {
        "plan" => Some("/to-spec"),
        "decisions" | "feature-review" | "spec" | "ticket-review" => Some("/to-tickets"),
        "tickets" => Some("/implement"),
        _ => None,
    }
}
