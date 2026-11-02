import { isCheckId } from './checks.js'
import { CRITIC_STATUSES, type CriticStatus } from './critic.js'
import { SEVERITIES, type Severity } from './findings.js'
import { JsonFields, JsonShapeError } from './json.js'
import type { PromptHashes } from './prompt.js'
import { CRITIQUE_STAGES, type CritiqueStage } from './stage.js'
import { familiesOf, VERDICTS, type AdjudicatedRun, type VerdictKind } from './verdicts.js'

/**
 * Ledger lines hold metadata only: identifiers, enums, numbers and hashes.
 * Claims, locations, evidence, suggestions and free-text reasons stay in the
 * report, so the ledger can be kept indefinitely.
 */
export function ledgerLines(run: AdjudicatedRun, recordedAt: Date): readonly string[] {
  const common = {
    depth: run.depth,
    flow: run.flow,
    depth_chosen_by: run.depth_choice.chosen_by,
    depth_measures: run.depth_choice.measures,
    depth_signals: run.depth_choice.signals,
    recorded_at: recordedAt.toISOString(),
    repository: run.repository,
    round: run.round,
    run_id: run.run_id,
    spec: run.spec,
    stage: run.stage,
  }
  const criticFields = (criticId: string) => {
    const critic = run.critics.find((c) => c.critic_id === criticId)
    return {
      client: critic?.client,
      critic_id: criticId,
      family: critic?.family,
      lens_hashes: critic?.lens_hashes,
      lenses: critic?.lenses,
      model: critic?.model,
      prompt_hashes: critic?.prompt_hashes,
      role: critic?.role,
    }
  }
  const runLines = run.critics.map((critic) => ({
    type: 'run',
    ...common,
    ...criticFields(critic.critic_id),
    charged_usd: critic.charged_usd,
    cost_usd: critic.cost_usd,
    duration_ms: critic.duration_ms,
    exit_code: critic.exit_code,
    failure: critic.failure ?? null,
    finding_count: critic.finding_count,
    retry_of: critic.retry_of ?? null,
    status: critic.status,
  }))
  const findingLines = run.findings.map((finding) => ({
    type: 'finding',
    ...common,
    ...criticFields(finding.raised_by[0] ?? ''),
    category: finding.category,
    /** The lens check or role rule the finding names, so stats can count precision per check; free text stays out. */
    check: isCheckId(finding.check) ? finding.check : null,
    evidence_kinds: finding.evidence.map((e) => e.kind),
    finding_id: finding.id,
    /** Fan-out's mechanical pre-verdict, so the ledger shows how often a hint is overruled. */
    pre_verdict: finding.pre_verdict?.reason_code ?? null,
    /** Whether the owner answered a needs-human question before the run was recorded. */
    answered: finding.verdict.answered === true,
    raised_by: finding.raised_by,
    raised_by_families: familiesOf(run, finding),
    reason_code: finding.verdict.reason_code ?? null,
    /** The severity as recorded: the check's, the critic's, or the orchestrator's override. */
    severity: finding.severity,
    /** The severity the finding's check fixed, so stats can measure how often the orchestrator overrides it. */
    declared_severity: finding.declared_severity ?? null,
    severity_override: finding.verdict.severity ?? null,
    severity_reason: finding.verdict.severity_reason ?? null,
    verdict: finding.verdict.verdict,
  }))
  return [...runLines, ...findingLines].map((line) => JSON.stringify(line))
}

/** What every ledger line says about the run it belongs to. */
export interface LedgerRunContext {
  readonly run_id: string
  readonly stage: CritiqueStage
  readonly repository: string
  readonly recorded_at: string
  /** Missing on lines recorded before the ledger named the spec. */
  readonly spec: string | undefined
  /** The flow whose Vibe cap the run counted against; missing on lines recorded before flows. */
  readonly flow: string | undefined
  /** 1 on lines recorded before rounds. */
  readonly round: number
}

/** One critic's run, as stats reads it back. */
export interface LedgerCriticRun extends LedgerRunContext {
  readonly critic_id: string
  readonly client: string
  readonly model: string
  readonly role: string
  readonly lenses: readonly string[]
  readonly prompt_hashes: PromptHashes
  readonly lens_hashes: Readonly<Record<string, string>>
  readonly cost_usd: number | null
  readonly status: CriticStatus
  readonly failure: string | null
}

/** One adjudicated finding, as stats reads it back. */
export interface LedgerFinding extends LedgerRunContext {
  readonly finding_id: string
  readonly severity: Severity
  readonly raised_by: readonly string[]
  readonly verdict: VerdictKind
  readonly pre_verdict: string | null
  /** False on lines recorded before the ledger kept answers. */
  readonly answered: boolean
}

/** The status of the run line a low-risk triage writes for the ticket review it skipped. */
export const SKIPPED_LOW_RISK = 'skipped-low-risk'

/** A ticket review a low-risk triage skipped, as the flow state reads it back. */
export interface LedgerSkippedReview {
  readonly run_id: string
  readonly recorded_at: string
  readonly repository: string
  readonly flow: string
  /** The ticket's reference. */
  readonly ticket: string
}

/**
 * What the owner told the flow state that no run records: that grilling is
 * done, that critique is skipped on purpose (or back on), that a ticket has
 * landed, that a question is answered, or which draft flow a spec grew from.
 */
export type FlowEvent =
  | { readonly event: 'grilled' }
  | { readonly event: 'no-critique'; readonly value: boolean }
  | { readonly event: 'landed'; readonly ticket: string }
  | { readonly event: 'answered'; readonly run_id: string; readonly finding_id: string }
  | { readonly event: 'draft'; readonly draft: string }

export const FLOW_EVENTS = ['grilled', 'no-critique', 'landed', 'answered', 'draft'] as const

/** Where and when a flow event was recorded. */
export interface FlowEventContext {
  readonly recorded_at: string
  readonly repository: string
  readonly flow: string
}

export type LedgerFlowEvent = FlowEvent & FlowEventContext

/** One flow event as a ledger line: metadata only, like every other line. */
export function flowEventLine(event: FlowEvent, context: FlowEventContext): string {
  return JSON.stringify({ type: 'flow', ...context, ...event })
}

export interface Ledger {
  readonly runs: readonly LedgerCriticRun[]
  readonly findings: readonly LedgerFinding[]
  readonly skipped_reviews: readonly LedgerSkippedReview[]
  readonly flow_events: readonly LedgerFlowEvent[]
}

export const EMPTY_LEDGER: Ledger = { findings: [], flow_events: [], runs: [], skipped_reviews: [] }

/**
 * Reads a ledger back, field by field. A blank line is skipped, and so is a
 * line of a type this version does not know, so a later ticket can add one.
 * Any other line without the shape record writes is a JsonShapeError naming
 * its line number.
 */
export function parseLedger(text: string): Ledger {
  const runs: LedgerCriticRun[] = []
  const findings: LedgerFinding[] = []
  const skipped: LedgerSkippedReview[] = []
  const events: LedgerFlowEvent[] = []
  text.split('\n').forEach((line, index) => {
    if (line.trim() === '') return
    const fields = JsonFields.of(parseLine(line, index + 1), `line ${String(index + 1)}`)
    const type = fields.string('type')
    // A low-risk triage writes a run line for the review it skipped, which no critic ran.
    if (type === 'run' && fields.optionalString('status') === SKIPPED_LOW_RISK) skipped.push(readSkippedReview(fields))
    else if (type === 'run') runs.push(readCriticRunLine(fields))
    if (type === 'finding') findings.push(readFindingLine(fields))
    if (type === 'flow') events.push(readFlowEvent(fields))
  })
  return { findings, flow_events: events, runs, skipped_reviews: skipped }
}

function readSkippedReview(fields: JsonFields): LedgerSkippedReview {
  return {
    flow: fields.string('flow'),
    recorded_at: fields.string('recorded_at'),
    repository: fields.string('repository'),
    run_id: fields.string('run_id'),
    ticket: fields.string('ticket'),
  }
}

function readFlowEvent(fields: JsonFields): LedgerFlowEvent {
  const context = { flow: fields.string('flow'), recorded_at: fields.string('recorded_at'), repository: fields.string('repository') }
  const event = fields.oneOf('event', FLOW_EVENTS)
  if (event === 'no-critique') return { ...context, event, value: fields.boolean('value') }
  if (event === 'landed') return { ...context, event, ticket: fields.string('ticket') }
  if (event === 'answered') return { ...context, event, finding_id: fields.string('finding_id'), run_id: fields.string('run_id') }
  if (event === 'draft') return { ...context, draft: fields.string('draft'), event }
  return { ...context, event }
}

function parseLine(line: string, number: number): unknown {
  try {
    return JSON.parse(line) as unknown
  } catch {
    throw new JsonShapeError(`line ${String(number)} is not JSON`)
  }
}

function readContext(fields: JsonFields): LedgerRunContext {
  return {
    recorded_at: fields.string('recorded_at'),
    repository: fields.string('repository'),
    run_id: fields.string('run_id'),
    spec: fields.optionalString('spec'),
    flow: fields.optionalString('flow'),
    round: fields.has('round') ? fields.number('round') : 1,
    stage: fields.oneOf('stage', CRITIQUE_STAGES),
  }
}

function readCriticRunLine(fields: JsonFields): LedgerCriticRun {
  const hashes = fields.object('prompt_hashes')
  return {
    ...readContext(fields),
    client: fields.string('client'),
    cost_usd: fields.nullableNumber('cost_usd'),
    critic_id: fields.string('critic_id'),
    failure: fields.nullableString('failure'),
    lens_hashes: fields.stringMap('lens_hashes'),
    lenses: fields.strings('lenses'),
    model: fields.string('model'),
    prompt_hashes: {
      contract: hashes.string('contract'),
      preamble: hashes.string('preamble'),
      role: hashes.string('role'),
      schema: hashes.string('schema'),
    },
    role: fields.string('role'),
    status: fields.oneOf('status', CRITIC_STATUSES),
  }
}

function readFindingLine(fields: JsonFields): LedgerFinding {
  return {
    ...readContext(fields),
    answered: fields.has('answered') && fields.boolean('answered'),
    finding_id: fields.string('finding_id'),
    pre_verdict: fields.nullableString('pre_verdict'),
    severity: fields.oneOf('severity', SEVERITIES),
    raised_by: fields.strings('raised_by'),
    verdict: fields.oneOf('verdict', VERDICTS),
  }
}
