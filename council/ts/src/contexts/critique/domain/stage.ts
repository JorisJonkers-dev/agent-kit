/**
 * Every stage the runner knows: the critique stages of the spec-to-ticket
 * flow, then the review stages, as CONTEXT.md names them.
 */
export const CRITIQUE_STAGES = ['decisions', 'spec', 'tickets', 'ticket-review', 'feature-review', 'adr-review', 'decision-recovery', 'architecture-review', 'plan'] as const
export type CritiqueStage = (typeof CRITIQUE_STAGES)[number]

/**
 * The review stages: run on demand over a repository, outside any feature's
 * flow, each run a flow of its own keyed by its run id (ADR-0003).
 */
export const REVIEW_STAGES = ['adr-review', 'decision-recovery', 'architecture-review'] as const satisfies readonly CritiqueStage[]
export type ReviewStage = (typeof REVIEW_STAGES)[number]

export function isReviewStage(stage: CritiqueStage): stage is ReviewStage {
  return (REVIEW_STAGES as readonly CritiqueStage[]).includes(stage)
}

/** The stages that send code, not documents, so the allowlist entry must allow code. */
export const CODE_STAGES = ['ticket-review', 'feature-review'] as const satisfies readonly CritiqueStage[]
export type CodeStage = (typeof CODE_STAGES)[number]

export function isCodeStage(stage: CritiqueStage): stage is CodeStage {
  return (CODE_STAGES as readonly CritiqueStage[]).includes(stage)
}

/**
 * The stages that send code but build no code packet: a review stage or the
 * plan stage, which samples the repository's tree. Like a code stage, each needs the code
 * permission of the family allowlist entry.
 */
export const REPOSITORY_CODE_STAGES = ['decision-recovery', 'architecture-review', 'plan'] as const satisfies readonly CritiqueStage[]

/** Whether the stage sends code, so the allowlist entry must allow code: a code stage, or one that samples the tree. */
export function needsCodePermission(stage: CritiqueStage): boolean {
  return isCodeStage(stage) || (REPOSITORY_CODE_STAGES as readonly CritiqueStage[]).includes(stage)
}

/** How many critics a stage runs: base (the stage's base roster in routing) or deep (base plus its allowed deep critics). */
export const DEPTHS = ['base', 'deep'] as const
export type Depth = (typeof DEPTHS)[number]

/**
 * A stage runs once, and a second time on the revised artifact only when the
 * first round accepted more than three critical findings; never a third time.
 */
export type Round = 1 | 2
export const SECOND_ROUND_AFTER_CRITICAL = 3
