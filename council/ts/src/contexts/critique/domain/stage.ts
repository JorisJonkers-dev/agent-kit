/** The critique stages of the spec-to-ticket flow, as CONTEXT.md names them. */
export const CRITIQUE_STAGES = ['decisions', 'spec', 'tickets', 'ticket-review', 'feature-review'] as const
export type CritiqueStage = (typeof CRITIQUE_STAGES)[number]

/** The stages that send code, not documents, so the allowlist entry must allow code. */
export const CODE_STAGES = ['ticket-review', 'feature-review'] as const satisfies readonly CritiqueStage[]
export type CodeStage = (typeof CODE_STAGES)[number]

export function isCodeStage(stage: CritiqueStage): stage is CodeStage {
  return (CODE_STAGES as readonly CritiqueStage[]).includes(stage)
}

/** How many critics a stage runs: base (two families) or deep (base plus every further allowed family). */
export const DEPTHS = ['base', 'deep'] as const
export type Depth = (typeof DEPTHS)[number]
