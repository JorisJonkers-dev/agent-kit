/** The critique stages of the spec-to-ticket flow, as CONTEXT.md names them. */
export const CRITIQUE_STAGES = ['decisions', 'spec', 'tickets', 'ticket-review', 'feature-review'] as const
export type CritiqueStage = (typeof CRITIQUE_STAGES)[number]

/** How many critics a stage runs: base (two families) or deep (base plus every further allowed family). */
export const DEPTHS = ['base', 'deep'] as const
export type Depth = (typeof DEPTHS)[number]

/**
 * A stage runs once, and a second time on the revised artifact only when the
 * first round accepted more than three critical findings; never a third time.
 */
export type Round = 1 | 2
export const SECOND_ROUND_AFTER_CRITICAL = 3
