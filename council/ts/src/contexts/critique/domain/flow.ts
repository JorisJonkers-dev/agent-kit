import type { GateRecord } from './feature-review.js'
import { SEVERITIES, type Severity } from './findings.js'
import { JsonFields } from './json.js'
import type { FlowEvent, LedgerFinding } from './ledger.js'
import { sha256 } from './manifest.js'
import { CRITIQUE_STAGES, SECOND_ROUND_AFTER_CRITICAL, type CritiqueStage } from './stage.js'
import { STAGE_TABLE } from './stage-table.js'
import type { Ticket } from './tickets.js'
import type { TicketReviewRecord } from './ticket-review.js'

/**
 * The spans of one feature's flow: planning (grilling, the decisions
 * critique, or for a refactor the plan critique, `/to-spec`, the spec critique, `/to-tickets`, the ticket check),
 * implementation (one `/implement` per ticket, a ticket review for the risky
 * ones) and finishing (the feature review and its gate).
 */
export const FLOW_PHASES = ['planning', 'implementation', 'finishing'] as const
export type FlowPhase = (typeof FLOW_PHASES)[number]

/** The stages a flow runs for the feature as a whole; a ticket review runs per ticket. */
export const FLOW_STAGES = ['decisions', 'plan', 'spec', 'tickets', 'feature-review'] as const satisfies readonly CritiqueStage[]
export type FlowStage = (typeof FLOW_STAGES)[number]

/**
 * Where a stage stands in the flow: never run, prepared or fanned out but
 * not recorded, recorded but owed a second round, recorded with a failed
 * ticket check, or recorded.
 */
export const STAGE_STATUSES = ['not-run', 'in-progress', 'second-round-due', 'check-failed', 'recorded'] as const
export type StageStatus = (typeof STAGE_STATUSES)[number]

/**
 * A ticket's review: none yet, skipped by triage as low-risk, passed, failed
 * with an accepted critical finding unfixed, or degraded because no critic
 * reported.
 */
export const REVIEW_STATUSES = ['not-reviewed', 'skipped-low-risk', 'reviewed', 'findings-open', 'degraded'] as const
export type ReviewStatus = (typeof REVIEW_STATUSES)[number]

export const GATE_STATUSES = ['none', 'pass', 'fail'] as const
export type GateStatus = (typeof GATE_STATUSES)[number]

export interface StageProgress {
  readonly status: StageStatus
  /** Every run of the stage in the flow, oldest first, recorded or not. */
  readonly run_ids: readonly string[]
  readonly latest_run_id: string | null
}

export interface FlowTicket {
  /** `#184`, or the `NN` of a local ticket file. */
  readonly id: string
  readonly reference: string
  readonly title: string
  readonly blocked_by: readonly string[]
  /** Closed on the tracker, marked landed by the owner, or reviewed (or skipped as low-risk) at its commit. */
  readonly landed: boolean
  readonly review: ReviewStatus
  /** The ticket-review and triage runs that decided `review`, oldest first. */
  readonly review_run_ids: readonly string[]
}

/** A needs-human verdict the owner has not answered. */
export interface OpenQuestion {
  readonly run_id: string
  readonly stage: CritiqueStage
  readonly finding_id: string
  readonly severity: Severity
}

export interface FlowGate {
  readonly status: GateStatus
  readonly run_id: string | null
  /** The head the gate holds for. */
  readonly head_commit: string | null
  readonly line: string | null
}

/**
 * One feature's flow state: a cache over the ledger, the run directories,
 * the review and gate indexes and the tracker, kept per feature in the
 * critique folder of the git common directory. Its key is the spec's
 * reference, or `draft:<name>` before `/to-spec` has written a spec.
 */
export interface FlowState {
  readonly version: 1
  readonly flow: string
  /** The spec's reference; null for a draft flow. */
  readonly spec: string | null
  /** The draft flow the spec grew from, if any. */
  readonly draft: string | null
  readonly repository: string
  readonly updated_at: string
  /** The commit HEAD named when the state was computed. */
  readonly head_commit: string
  readonly phase: FlowPhase
  /** The owner chose to step through the plain Pocock flow, skipping critique on purpose. */
  readonly no_critique: boolean
  /** Grilling is done: the owner said so, a decisions or plan run exists, a plan file exists, or a spec does. */
  readonly grilled: boolean
  /** A refactoring plan file exists for the draft, which marks it a refactor: the plan stage takes the decisions stage's place. */
  readonly refactor: boolean
  /** Where the draft's plan file lives by the tracker convention, whether or not it exists yet; null for a flow with no draft. */
  readonly plan_path: string | null
  readonly stages: Readonly<Record<FlowStage, StageProgress>>
  /** In dependency order: a ticket comes after every ticket of the spec it is blocked by, tracker order otherwise. */
  readonly tickets: readonly FlowTicket[]
  readonly open_questions: readonly OpenQuestion[]
  readonly gate: FlowGate
  /** What the flow's Vibe critics have been charged, the draft's included. */
  readonly vibe_spend_usd: number
}

/** One run of the flow, from its run directory, the ledger, or both. */
export interface FlowRun {
  readonly run_id: string
  readonly stage: CritiqueStage
  readonly round: number
  readonly recorded: boolean
  /** The tickets stage's ticket check as the packet froze it; null at any other stage or without the run directory. */
  readonly ticket_check: 'passed' | 'failed' | null
}

/** Everything the flow state is computed from, already narrowed to this flow and repository. */
export interface FlowSources {
  readonly flow: string
  readonly spec: string | null
  readonly draft: string | null
  readonly repository: string
  readonly now: string
  readonly head_commit: string
  /** The draft's plan file: where the tracker convention puts it, and whether it exists; the path is null for a flow with no draft. */
  readonly plan: { readonly path: string | null; readonly exists: boolean }
  readonly runs: readonly FlowRun[]
  /** The ledger's finding lines of the flow's runs. */
  readonly findings: readonly Pick<LedgerFinding, 'run_id' | 'stage' | 'finding_id' | 'severity' | 'verdict' | 'answered'>[]
  /** The spec's tickets in tracker order, and whether the tracker still holds each open. */
  readonly tickets: readonly (Pick<Ticket, 'id' | 'reference' | 'title' | 'blockedBy'> & { readonly open: boolean })[]
  readonly reviews: readonly Pick<TicketReviewRecord, 'run_id' | 'recorded_at' | 'ticket' | 'review' | 'degraded'>[]
  readonly skipped_reviews: readonly { readonly run_id: string; readonly recorded_at: string; readonly ticket: string }[]
  /** The spec's feature-review gates, oldest first. */
  readonly gates: readonly Pick<GateRecord, 'run_id' | 'head_commit' | 'gate' | 'line'>[]
  /** The owner's events for this flow and its draft, in the order they were recorded. */
  readonly events: readonly FlowEvent[]
  readonly vibe_spend_usd: number
}

/** The flow key of a draft: the flow before `/to-spec` has written a spec. */
export function draftFlow(name: string): string {
  return `draft:${name}`
}

/** The state file's name: readable, and unique per flow however its key is spelt. */
export function flowStateFileName(flow: string): string {
  const slug = flow
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 60)
  return `${slug}-${sha256(flow).slice(0, 12)}.json`
}

/** Computes the flow state from its sources. */
export function flowState(sources: FlowSources): FlowState {
  const stage = (name: FlowStage) => stageProgress(sources, name)
  const stages: Record<FlowStage, StageProgress> = {
    decisions: stage('decisions'),
    plan: stage('plan'),
    'feature-review': stage('feature-review'),
    spec: stage('spec'),
    tickets: stage('tickets'),
  }
  const noCritique = sources.events.reduce((value, event) => (event.event === 'no-critique' ? event.value : value), false)
  const refactor = sources.plan.exists
  const grilled =
    sources.spec !== null || refactor || stages.decisions.run_ids.length > 0 || stages.plan.run_ids.length > 0 || sources.events.some((event) => event.event === 'grilled')
  const tickets = dependencyOrder(sources.tickets).map((ticket) => flowTicket(sources, ticket))
  const latestGate = sources.gates.at(-1)
  const partial = { no_critique: noCritique, spec: sources.spec, stages, tickets }
  return {
    draft: sources.draft,
    flow: sources.flow,
    gate:
      latestGate === undefined
        ? { head_commit: null, line: null, run_id: null, status: 'none' }
        : { head_commit: latestGate.head_commit, line: latestGate.line, run_id: latestGate.run_id, status: latestGate.gate },
    grilled,
    head_commit: sources.head_commit,
    no_critique: noCritique,
    open_questions: openQuestions(sources),
    phase: flowPhase(partial),
    plan_path: sources.plan.path,
    refactor,
    repository: sources.repository,
    spec: sources.spec,
    stages,
    tickets,
    updated_at: sources.now,
    version: 1,
    vibe_spend_usd: sources.vibe_spend_usd,
  }
}

function stageProgress(sources: FlowSources, stage: FlowStage): StageProgress {
  const runs = sources.runs.filter((run) => run.stage === stage).sort((a, b) => compareRunIds(a.run_id, b.run_id))
  const latest = runs.at(-1)
  const progress = (status: StageStatus): StageProgress => ({ latest_run_id: latest?.run_id ?? null, run_ids: runs.map((run) => run.run_id), status })
  if (latest === undefined) return progress('not-run')
  if (!latest.recorded) return progress('in-progress')
  if (latest.ticket_check === 'failed') return progress('check-failed')
  const critical = sources.findings.filter(
    (finding) => finding.run_id === latest.run_id && finding.severity === 'critical' && finding.verdict === 'accepted',
  )
  if (latest.round === 1 && STAGE_TABLE[stage].secondRound && critical.length > SECOND_ROUND_AFTER_CRITICAL) return progress('second-round-due')
  return progress('recorded')
}

/** Run ids start with their compact timestamp, and a run id taken twice in a second gets `-2`, `-3`, ... */
function compareRunIds(a: string, b: string): number {
  return a.localeCompare(b, 'en', { numeric: true })
}

/** A record written before #259 says `pass` with `degraded: true` for a run no critic reported in. */
function reviewStatus(review: Pick<TicketReviewRecord, 'review' | 'degraded'>): ReviewStatus {
  if (review.review === 'fail') return 'findings-open'
  return review.review === 'degraded' || review.degraded ? 'degraded' : 'reviewed'
}

function flowTicket(sources: FlowSources, ticket: FlowSources['tickets'][number]): FlowTicket {
  const decisions = [
    ...sources.reviews
      .filter((review) => review.ticket === ticket.reference)
      .map((review) => ({ at: review.recorded_at, review: reviewStatus(review), run_id: review.run_id })),
    ...sources.skipped_reviews
      .filter((skip) => skip.ticket === ticket.reference)
      .map((skip) => ({ at: skip.recorded_at, review: 'skipped-low-risk' as const, run_id: skip.run_id })),
  ].sort((a, b) => a.at.localeCompare(b.at) || compareRunIds(a.run_id, b.run_id))
  const review = decisions.at(-1)?.review ?? 'not-reviewed'
  const marked = sources.events.some((event) => event.event === 'landed' && event.ticket === ticket.reference)
  return {
    blocked_by: ticket.blockedBy,
    id: ticket.id,
    landed: !ticket.open || marked || review === 'reviewed' || review === 'skipped-low-risk',
    reference: ticket.reference,
    review,
    review_run_ids: decisions.map((decision) => decision.run_id),
    title: ticket.title,
  }
}

function openQuestions(sources: FlowSources): readonly OpenQuestion[] {
  const answered = new Set(
    sources.events.flatMap((event) => (event.event === 'answered' ? [`${event.run_id} ${event.finding_id}`] : [])),
  )
  return sources.findings
    .filter((finding) => finding.verdict === 'needs-human' && !finding.answered && !answered.has(`${finding.run_id} ${finding.finding_id}`))
    .map((finding) => ({ finding_id: finding.finding_id, run_id: finding.run_id, severity: finding.severity, stage: finding.stage }))
}

/**
 * Orders tickets so each comes after every ticket of the spec that blocks
 * it, keeping tracker order otherwise. A blocker outside the spec is not
 * waited for, and a cycle is broken at its earliest ticket in tracker order.
 */
export function dependencyOrder<T extends Pick<Ticket, 'id' | 'blockedBy'>>(tickets: readonly T[]): readonly T[] {
  const ids = new Set(tickets.map((ticket) => ticket.id))
  const place = (left: readonly T[], placed: ReadonlySet<string>): readonly T[] => {
    const [first] = left
    if (first === undefined) return []
    const next = left.find((ticket) => ticket.blockedBy.every((blocker) => !ids.has(blocker) || placed.has(blocker))) ?? first
    return [next, ...place(left.filter((ticket) => ticket !== next), new Set([...placed, next.id]))]
  }
  return place(tickets, new Set())
}

/** Which span of the flow the state is in. */
export function flowPhase(state: Pick<FlowState, 'spec' | 'no_critique' | 'stages' | 'tickets'>): FlowPhase {
  const critique = !state.no_critique
  if (state.spec === null) return 'planning'
  if (critique && state.stages.spec.status !== 'recorded') return 'planning'
  if (state.tickets.length === 0) return 'planning'
  if (critique && state.stages.tickets.status !== 'recorded') return 'planning'
  return state.tickets.some((ticket) => !ticket.landed) ? 'implementation' : 'finishing'
}

/** Pocock's skills a model cannot invoke, so the flow only ever names them for the owner to type. */
export const POCOCK_SKILLS = ['/grill-with-docs', '/grill-me', '/to-spec', '/to-tickets', '/implement'] as const
export type PocockSkill = (typeof POCOCK_SKILLS)[number]

/**
 * The single next step of a flow: a critique stage the stepper may run on
 * the owner's word, the exact Pocock command the owner types, or nothing
 * left to do. Each comes with one sentence on why.
 */
export type NextStep =
  | {
      readonly kind: 'stage'
      readonly stage: CritiqueStage
      readonly reason: string
      /** At ticket review: the ticket to review. */
      readonly ticket?: string
      /** A run that was prepared but not recorded: finish it rather than prepare another. */
      readonly resume_run_id?: string
      /** A second round: prepare with this run as `previous_run_dir`. */
      readonly previous_run_id?: string
    }
  | { readonly kind: 'command'; readonly skill: PocockSkill; readonly command: string; readonly reason: string }
  | { readonly kind: 'done'; readonly reason: string }

/** The next step, as a pure function of the flow state. */
export function nextStep(state: FlowState): NextStep {
  if (state.spec === null) return draftStep(state)
  return planningStep(state) ?? workStep(state)
}

function command(skill: PocockSkill, reason: string, argument?: string): NextStep {
  return { command: argument === undefined ? skill : `${skill} ${argument}`, kind: 'command', reason, skill }
}

/** What a stage still owes the flow, or undefined once it is recorded. */
function stageStep(state: FlowState, stage: FlowStage, notRun: string): NextStep | undefined {
  const progress = state.stages[stage]
  const run = progress.latest_run_id ?? ''
  switch (progress.status) {
    case 'not-run':
      return { kind: 'stage', reason: notRun, stage }
    case 'in-progress':
      return { kind: 'stage', reason: `Run ${run} of the ${stage} stage is not recorded yet: finish it before moving on.`, resume_run_id: run, stage }
    case 'second-round-due':
      return {
        kind: 'stage',
        previous_run_id: run,
        reason: `Run ${run} of the ${stage} stage accepted more than three critical findings: run its second round on the revised artifact.`,
        stage,
      }
    case 'check-failed':
      return { kind: 'stage', reason: 'The ticket check found a dependency cycle: fix the blocker links and check the tickets again.', stage }
    case 'recorded':
      return undefined
  }
}

function draftStep(state: FlowState): NextStep {
  if (!state.grilled) return command('/grill-with-docs', 'Every flow starts by grilling the idea against the domain docs; type /flow again when the grilling is done.')
  // A refactor's plan file takes the decisions stage's place (ADR-0005): its critics check the plan, not the decisions.
  if (state.refactor) {
    const plan = state.no_critique
      ? undefined
      : stageStep(state, 'plan', 'The refactoring plan is drafted: have critics from other model families check its steps, their order and their anchors before the spec is written.')
    return plan ?? command('/to-spec', 'The plan is settled, so the spec can be written from it; its Open decisions section must be empty first.')
  }
  const decisions = state.no_critique
    ? undefined
    : stageStep(state, 'decisions', 'The grilling is done: have critics from other model families look for decisions it left open before the spec is written.')
  return decisions ?? command('/to-spec', 'The decisions are settled, so the spec can be written from them.')
}

function openCritical(state: FlowState): OpenQuestion | undefined {
  return state.no_critique ? undefined : state.open_questions.find((question) => question.severity === 'critical')
}

function questionStep(question: OpenQuestion, next: string): NextStep {
  return command(
    '/grill-me',
    `Critical question ${question.finding_id} from the ${question.stage} critique (run ${question.run_id}) is unanswered: settle it before ${next}.`,
  )
}

/** What planning still owes, or undefined once the spec is critiqued and its tickets are cut and checked. */
function planningStep(state: FlowState): NextStep | undefined {
  const critique = !state.no_critique
  const spec = critique
    ? stageStep(state, 'spec', 'The spec is written: have critics from other model families red-team it before it is cut into tickets.')
    : undefined
  if (spec !== undefined) return spec
  if (state.tickets.length === 0) {
    const question = openCritical(state)
    return question === undefined
      ? command('/to-tickets', 'The spec is settled, so it can be cut into tickets.')
      : questionStep(question, 'cutting tickets from the spec')
  }
  return critique
    ? stageStep(state, 'tickets', 'The tickets are cut: check them for cycles, untestable criteria and missing seams before implementing.')
    : undefined
}

/**
 * Implementation while a ticket has not landed, finishing once all have.
 * Tickets are in dependency order, so the first that has not landed is
 * blocked by none that has not.
 */
function workStep(state: FlowState): NextStep {
  const waiting = state.tickets.filter((ticket) => !ticket.landed)
  const [ticket] = waiting
  if (ticket === undefined) return finishingStep(state)
  const question = openCritical(state)
  if (question !== undefined) return questionStep(question, 'implementing')
  if (ticket.review === 'findings-open' && !state.no_critique) {
    return {
      kind: 'stage',
      reason: `The review of ticket ${ticket.id} accepted a critical finding that is not fixed: fix it, then review the staged change again.`,
      stage: 'ticket-review',
      ticket: ticket.id,
    }
  }
  if (ticket.review === 'degraded' && !state.no_critique) {
    return {
      kind: 'stage',
      reason: `No critic reported in the review of ticket ${ticket.id}, so nobody reviewed it: review the staged change again.`,
      stage: 'ticket-review',
      ticket: ticket.id,
    }
  }
  const left = waiting.length === 1 ? 'it is the last ticket left' : `it is the next of ${String(waiting.length)} tickets left in dependency order`
  return command('/implement', `Ticket ${ticket.id} has not landed and ${left}; start it in a fresh session.`, implementArgument(ticket))
}

/** What `/implement` is given: an issue's number, or a local ticket's path, which names it unambiguously. */
function implementArgument(ticket: FlowTicket): string {
  return ticket.reference.includes('#') ? ticket.id : ticket.reference
}

function finishingStep(state: FlowState): NextStep {
  if (state.no_critique) return { kind: 'done', reason: 'Every ticket has landed, and critique was skipped on purpose, so there is no feature review to run.' }
  const finish = stageStep(state, 'feature-review', 'Every ticket has landed: review the feature as a whole before the pull request.')
  if (finish !== undefined) return finish
  const gate = state.gate
  if (gate.status === 'pass' && gate.head_commit === state.head_commit) {
    return { kind: 'done', reason: 'GATE: PASS for the current head: the feature is ready for its pull request.' }
  }
  const reasons: Record<GateStatus, string> = {
    fail: `The last feature review held the gate shut (${gate.line ?? 'GATE: FAIL'}) and its follow-ups have landed: review the feature again on the new head.`,
    none: 'The feature review left no gate: review the feature again.',
    pass: 'The gate passed for an earlier head: review the feature again on the current one.',
  }
  return { kind: 'stage', reason: reasons[gate.status], stage: 'feature-review' }
}

/** Reads a flow state file back, field by field. */
export function parseFlowState(text: string): FlowState {
  const file = JsonFields.parse(text)
  const stages = file.object('stages')
  const progress = (name: FlowStage): StageProgress => {
    const stage = stages.object(name)
    return { latest_run_id: stage.nullableString('latest_run_id'), run_ids: stage.strings('run_ids'), status: stage.oneOf('status', STAGE_STATUSES) }
  }
  const gate = file.object('gate')
  return {
    draft: file.nullableString('draft'),
    flow: file.string('flow'),
    gate: {
      head_commit: gate.nullableString('head_commit'),
      line: gate.nullableString('line'),
      run_id: gate.nullableString('run_id'),
      status: gate.oneOf('status', GATE_STATUSES),
    },
    grilled: file.boolean('grilled'),
    head_commit: file.string('head_commit'),
    no_critique: file.boolean('no_critique'),
    open_questions: file.list('open_questions', (question) => ({
      finding_id: question.string('finding_id'),
      run_id: question.string('run_id'),
      severity: question.oneOf('severity', SEVERITIES),
      stage: question.oneOf('stage', CRITIQUE_STAGES),
    })),
    phase: file.oneOf('phase', FLOW_PHASES),
    plan_path: file.nullableString('plan_path'),
    refactor: file.boolean('refactor'),
    repository: file.string('repository'),
    spec: file.nullableString('spec'),
    stages: { decisions: progress('decisions'), 'feature-review': progress('feature-review'), plan: progress('plan'), spec: progress('spec'), tickets: progress('tickets') },
    tickets: file.list('tickets', (ticket) => ({
      blocked_by: ticket.strings('blocked_by'),
      id: ticket.string('id'),
      landed: ticket.boolean('landed'),
      reference: ticket.string('reference'),
      review: ticket.oneOf('review', REVIEW_STATUSES),
      review_run_ids: ticket.strings('review_run_ids'),
      title: ticket.string('title'),
    })),
    updated_at: file.string('updated_at'),
    version: file.literal('version', 1),
    vibe_spend_usd: file.number('vibe_spend_usd'),
  }
}
