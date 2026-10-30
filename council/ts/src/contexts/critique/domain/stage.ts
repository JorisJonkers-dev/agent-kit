/** The critique stages of the spec-to-ticket flow, as CONTEXT.md names them. */
export const CRITIQUE_STAGES = ['decisions', 'spec', 'tickets', 'ticket-review', 'feature-review'] as const
export type CritiqueStage = (typeof CRITIQUE_STAGES)[number]

/** How many critics a stage runs: base (two families) or deep (base plus every further allowed family). */
export const DEPTHS = ['base', 'deep'] as const
export type Depth = (typeof DEPTHS)[number]
