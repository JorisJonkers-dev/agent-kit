//! The stages the runner knows, and the depths a stage runs at.

/// Every stage the runner knows: the critique stages of the spec-to-ticket flow, then the review stages, as
/// CONTEXT.md names them.
pub const CRITIQUE_STAGES: [&str; 9] =
    ["decisions", "spec", "tickets", "ticket-review", "feature-review", "adr-review", "decision-recovery", "architecture-review", "plan"];

/// How many critics a stage runs: base (the stage's base roster in routing) or deep (base plus its allowed deep
/// critics).
pub const DEPTHS: [&str; 2] = ["base", "deep"];
