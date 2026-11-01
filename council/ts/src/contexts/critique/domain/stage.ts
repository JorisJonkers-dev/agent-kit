/** The critique stages of the spec-to-ticket flow, as CONTEXT.md names them. */
export const CRITIQUE_STAGES = ['decisions', 'spec', 'tickets', 'ticket-review', 'feature-review'] as const
export type CritiqueStage = (typeof CRITIQUE_STAGES)[number]

/** The stages that send code, not documents, so the allowlist entry must allow code. */
export const CODE_STAGES = ['ticket-review', 'feature-review'] as const satisfies readonly CritiqueStage[]
export type CodeStage = (typeof CODE_STAGES)[number]

export function isCodeStage(stage: CritiqueStage): stage is CodeStage {
  return (CODE_STAGES as readonly CritiqueStage[]).includes(stage)
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
