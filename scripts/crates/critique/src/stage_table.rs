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
