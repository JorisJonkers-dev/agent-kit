import type { CodeStage, CritiqueStage } from './stage.js'

/**
 * What prepare freezes for a stage. `decisions`: the decisions under review
 * and the decision docs, with no tracker. `document`: the spec and its
 * references. `tickets`: the spec and every ticket cut from it. `code`: the
 * commit's tree, the diff, the ticket or tickets, and the spec.
 */
export type PacketBuilder = 'decisions' | 'document' | 'tickets' | 'code'

/** Which evidence rule fan-out applies to a stage's findings once every quote is found in the packet. */
export type EvidenceRule = 'decisions' | 'spec' | 'tickets' | 'code'

/**
 * The flow key a stage's run charges, unless the caller names one.
 * `draft`: the run starts a flow of its own, keyed by its run id, the
 * draft's key, since no tracker item exists yet. `spec`: the spec's
 * reference, so the run counts with the spec's other stages.
 */
export type FlowKey = 'draft' | 'spec'

/**
 * How `/critique-stats` judges a stage for removal. `features`: by its
 * latest features, the distinct specs it ran on.
 */
export type RemovalRule = 'features'

/** Everything the runner does differently from one stage to the next, beyond routing and depth. */
export interface StageBehaviour {
  readonly packet: PacketBuilder
  readonly evidence: EvidenceRule
  /** Whether more than three accepted critical findings call for a second round. */
  readonly secondRound: boolean
  /** The command the report says not to start while a critical question is open or the ticket graph has a cycle. */
  readonly doNotStart: '/to-tickets' | '/implement'
  readonly flowKey: FlowKey
  readonly removal: RemovalRule
}

/**
 * One entry per stage, so a stage with no entry fails to compile. Only a
 * code stage builds a code packet and takes the code evidence rule, since
 * only a code stage is allowed code and has its file and line citations
 * checked.
 */
export type StageTable = {
  readonly [S in CritiqueStage]: StageBehaviour & {
    readonly packet: S extends CodeStage ? 'code' : Exclude<PacketBuilder, 'code'>
    readonly evidence: S extends CodeStage ? 'code' : Exclude<EvidenceRule, 'code'>
  }
}

export const STAGE_TABLE: StageTable = {
  // A decisions run's accepted findings are questions for the owner and change
  // no artifact, so it never calls for a second round.
  decisions: { doNotStart: '/to-tickets', evidence: 'decisions', flowKey: 'draft', packet: 'decisions', removal: 'features', secondRound: false },
  'feature-review': { doNotStart: '/to-tickets', evidence: 'code', flowKey: 'spec', packet: 'code', removal: 'features', secondRound: true },
  spec: { doNotStart: '/to-tickets', evidence: 'spec', flowKey: 'spec', packet: 'document', removal: 'features', secondRound: true },
  'ticket-review': { doNotStart: '/to-tickets', evidence: 'code', flowKey: 'spec', packet: 'code', removal: 'features', secondRound: true },
  tickets: { doNotStart: '/implement', evidence: 'tickets', flowKey: 'spec', packet: 'tickets', removal: 'features', secondRound: true },
}
