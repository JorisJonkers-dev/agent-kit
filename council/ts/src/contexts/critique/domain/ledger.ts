import { CRITIC_STATUSES, type CriticStatus } from './critic.js'
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
    evidence_kinds: finding.evidence.map((e) => e.kind),
    finding_id: finding.id,
    /** Fan-out's mechanical pre-verdict, so the ledger shows how often a hint is overruled. */
    pre_verdict: finding.pre_verdict?.reason_code ?? null,
    raised_by: finding.raised_by,
    raised_by_families: familiesOf(run, finding),
    reason_code: finding.verdict.reason_code ?? null,
    severity: finding.severity,
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
  readonly raised_by: readonly string[]
  readonly verdict: VerdictKind
  readonly pre_verdict: string | null
}

/** The status of the run line a low-risk triage writes for the ticket review it skipped. */
export const SKIPPED_LOW_RISK = 'skipped-low-risk'

export interface Ledger {
  readonly runs: readonly LedgerCriticRun[]
  readonly findings: readonly LedgerFinding[]
}

/**
 * Reads a ledger back, field by field. A blank line is skipped, and so is a
 * line of a type this version does not know, so a later ticket can add one.
 * Any other line without the shape record writes is a JsonShapeError naming
 * its line number.
 */
export function parseLedger(text: string): Ledger {
  const runs: LedgerCriticRun[] = []
  const findings: LedgerFinding[] = []
  text.split('\n').forEach((line, index) => {
    if (line.trim() === '') return
    const fields = JsonFields.of(parseLine(line, index + 1), `line ${String(index + 1)}`)
    const type = fields.string('type')
    // A low-risk triage writes a run line for the review it skipped, which no critic ran.
    if (type === 'run' && fields.optionalString('status') === SKIPPED_LOW_RISK) return
    if (type === 'run') runs.push(readCriticRunLine(fields))
    if (type === 'finding') findings.push(readFindingLine(fields))
  })
  return { findings, runs }
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
    pre_verdict: fields.nullableString('pre_verdict'),
    raised_by: fields.strings('raised_by'),
    verdict: fields.oneOf('verdict', VERDICTS),
  }
}
