import type { CodeStage, CritiqueStage, ReviewStage } from './stage.js'

/**
 * What prepare freezes for a stage. `decisions`: the decisions under review
 * and the decision docs, with no tracker. `document`: the spec and its
 * references. `tickets`: the spec and every ticket cut from it. `code`: the
 * commit's tree, the diff, the ticket or tickets, and the spec. `adrs`: the
 * repository's decision records under review, with its glossary and any
 * other records as references, and no tracker; when the repository allows
 * code, also a sample of its tree, the tree's manifest and its facts.
 * `recovery`: the repository's tree, its facts and its decision records as
 * references, and its history when the allowlist entry allows it, with no
 * artifact of its own and no tracker item.
 * `architecture`: the same tree, facts and decision records, and beside them
 * the README and the build configuration as references, and never history.
 * `plan`: a refactoring plan under `artifact/` with the checker's report under
 * `check/plan.json`, the tree and facts at the head the plan targets (the files
 * the plan anchors and touches first), and the ADRs in force and the glossary as
 * references; no tracker item.
 */
export type PacketBuilder = 'decisions' | 'document' | 'tickets' | 'code' | 'adrs' | 'recovery' | 'architecture' | 'plan'

/** Which evidence rule fan-out applies to a stage's findings once every quote is found in the packet. */
/**
 * `records` is no stage's own: it is the rule for the decision records a
 * role reviews at a stage whose own rule is another.
 */
export type EvidenceRule = 'decisions' | 'spec' | 'tickets' | 'code' | 'adrs' | 'records' | 'recovery' | 'architecture' | 'plan'

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
  readonly doNotStart: '/to-spec' | '/to-tickets' | '/implement' | null
  readonly flowKey: FlowKey
  readonly removal: RemovalRule
  /**
   * For a stage whose findings are candidates for the owner to confirm: the
   * most candidates asked at once. The report ranks them by severity, which
   * is their confidence, asks that many and lists the rest as deferred; no
   * candidate is ever accepted without the owner. Null for every other stage.
   */
  readonly candidateQuestions: number | null
  /**
   * Set when the stage's recovered candidates sit among findings that may be
   * accepted: only a finding of category `undocumented-decision` is a candidate
   * then, never accepted, and the cap counts every question the run asks.
   */
  readonly mixedCandidates?: true
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
    readonly doNotStart: S extends ReviewStage ? null : '/to-spec' | '/to-tickets' | '/implement'
  }
}

/**
 * The roles whose findings a stage judges by another evidence rule than its
 * own. The decisions stage's ADR critic reviews the records among the
 * decisions, so its findings need a quote of them, not a gap's scenario.
 */
const ROLE_EVIDENCE: Readonly<Partial<Record<CritiqueStage, Readonly<Record<string, EvidenceRule>>>>> = {
  decisions: { 'adr-reviewer': 'records' },
  // A deep architecture review adds a decision recoverer, whose candidates follow the recovery rules.
  'architecture-review': { 'decision-recoverer': 'recovery' },
}

/** The evidence rule for a finding the critic of `role` raised at `stage`: the role's own there, else the stage's. */
export function evidenceRule(stage: CritiqueStage, role: string | undefined): EvidenceRule {
  return (role === undefined ? undefined : ROLE_EVIDENCE[stage]?.[role]) ?? STAGE_TABLE[stage].evidence
}

/** How many recovered candidates one run asks the owner about; the rest are deferred in the report. */
export const RECOVERY_QUESTION_CAP = 10

export const STAGE_TABLE: StageTable = {
  // A deep run's recovered candidates are never accepted and share the run's ten owner questions with the findings that need the owner; the rest are accepted or rejected as findings.
  'architecture-review': { candidateQuestions: RECOVERY_QUESTION_CAP, doNotStart: null, evidence: 'architecture', flowKey: 'run', mixedCandidates: true, packet: 'architecture', removal: 'runs', secondRound: false },
  // With the code permission on, the ADR packet is a repository packet (#221);
  // with it off, the evidence rule skips what needs code (#249).
  'adr-review': { candidateQuestions: null, doNotStart: null, evidence: 'adrs', flowKey: 'run', packet: 'adrs', removal: 'runs', secondRound: true },
  // A recovered decision is never accepted: only the owner's answer makes an ADR, so no round revises anything.
  'decision-recovery': { candidateQuestions: RECOVERY_QUESTION_CAP, doNotStart: null, evidence: 'recovery', flowKey: 'run', packet: 'recovery', removal: 'runs', secondRound: false },
  // A decisions run's accepted findings are questions for the owner and change
  // no artifact, so it never calls for a second round.
  // A plan takes the decisions stage's place for a refactor, so it sits before /to-spec; its accepted findings revise the plan, so it may need a second round.
  plan: { candidateQuestions: null, doNotStart: '/to-spec', evidence: 'plan', flowKey: 'draft', packet: 'plan', removal: 'features', secondRound: true },
  decisions: { candidateQuestions: null, doNotStart: '/to-tickets', evidence: 'decisions', flowKey: 'draft', packet: 'decisions', removal: 'features', secondRound: false },
  'feature-review': { candidateQuestions: null, doNotStart: '/to-tickets', evidence: 'code', flowKey: 'spec', packet: 'code', removal: 'features', secondRound: true },
  spec: { candidateQuestions: null, doNotStart: '/to-tickets', evidence: 'spec', flowKey: 'spec', packet: 'document', removal: 'features', secondRound: true },
  'ticket-review': { candidateQuestions: null, doNotStart: '/to-tickets', evidence: 'code', flowKey: 'spec', packet: 'code', removal: 'features', secondRound: true },
  tickets: { candidateQuestions: null, doNotStart: '/implement', evidence: 'tickets', flowKey: 'spec', packet: 'tickets', removal: 'features', secondRound: true },
}
