import type { CodeStage, CritiqueStage, ReviewStage } from './stage.js'

/**
 * What prepare freezes for a stage. `decisions`: the decisions under review
 * and the decision docs, with no tracker. `document`: the spec and its
 * references. `tickets`: the spec and every ticket cut from it. `code`: the
 * commit's tree, the diff, the ticket or tickets, and the spec. `adrs`: the
 * repository's decision records under review, with its glossary and any
 * other records as references, and no tracker; when the repository allows
 * code, also a sample of its tree, the tree's manifest and its facts.
 */
export type PacketBuilder = 'decisions' | 'document' | 'tickets' | 'code' | 'adrs'

/** Which evidence rule fan-out applies to a stage's findings once every quote is found in the packet. */
export type EvidenceRule = 'decisions' | 'spec' | 'tickets' | 'code' | 'adrs'

/**
 * The flow key a stage's run charges, unless the caller names one.
 * `draft`: the run starts a flow of its own, keyed by its run id, the
 * draft's key, since no tracker item exists yet. `spec`: the spec's
 * reference, so the run counts with the spec's other stages. `run`: a review
 * stage's run is a flow of its own, keyed by its run id, and never joins a
 * feature's flow, so it takes no flow from the caller (ADR-0003).
 */
export type FlowKey = 'draft' | 'spec' | 'run'

/**
 * How `/critique-stats` judges a stage for removal. `features`: by its
 * latest features, the distinct specs it ran on. `runs`: by its latest runs,
 * since a review stage belongs to no feature.
 */
export type RemovalRule = 'features' | 'runs'

/** Everything the runner does differently from one stage to the next, beyond routing and depth. */
export interface StageBehaviour {
  readonly packet: PacketBuilder
  readonly evidence: EvidenceRule
  /** Whether more than three accepted critical findings call for a second round. */
  readonly secondRound: boolean
  /**
   * The command the report says not to start while a critical question is
   * open or the ticket graph has a cycle; null for a review stage, which sits
   * before no command of a feature's flow.
   */
  readonly doNotStart: '/to-tickets' | '/implement' | null
  readonly flowKey: FlowKey
  readonly removal: RemovalRule
}

/**
 * One entry per stage, so a stage with no entry fails to compile. Only a
 * code stage builds a code packet and takes the code evidence rule, since
 * only a code stage is allowed code and has its file and line citations
 * checked. Exactly the review stages key each run by its run id and are
 * judged for removal by their runs.
 */
export type StageTable = {
  readonly [S in CritiqueStage]: StageBehaviour & {
    readonly packet: S extends CodeStage ? 'code' : Exclude<PacketBuilder, 'code'>
    readonly evidence: S extends CodeStage ? 'code' : Exclude<EvidenceRule, 'code'>
    readonly flowKey: S extends ReviewStage ? 'run' : Exclude<FlowKey, 'run'>
    readonly removal: S extends ReviewStage ? 'runs' : Exclude<RemovalRule, 'runs'>
    readonly doNotStart: S extends ReviewStage ? null : '/to-tickets' | '/implement'
  }
}

export const STAGE_TABLE: StageTable = {
  // With the code permission on, the ADR packet is a repository packet (#221);
  // with it off, the evidence rule skips what needs code (#249).
  'adr-review': { doNotStart: null, evidence: 'adrs', flowKey: 'run', packet: 'adrs', removal: 'runs', secondRound: true },
  // A decisions run's accepted findings are questions for the owner and change
  // no artifact, so it never calls for a second round.
  decisions: { doNotStart: '/to-tickets', evidence: 'decisions', flowKey: 'draft', packet: 'decisions', removal: 'features', secondRound: false },
  'feature-review': { doNotStart: '/to-tickets', evidence: 'code', flowKey: 'spec', packet: 'code', removal: 'features', secondRound: true },
  spec: { doNotStart: '/to-tickets', evidence: 'spec', flowKey: 'spec', packet: 'document', removal: 'features', secondRound: true },
  'ticket-review': { doNotStart: '/to-tickets', evidence: 'code', flowKey: 'spec', packet: 'code', removal: 'features', secondRound: true },
  tickets: { doNotStart: '/implement', evidence: 'tickets', flowKey: 'spec', packet: 'tickets', removal: 'features', secondRound: true },
}
