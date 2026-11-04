import type { LoggedInvocation } from './budget.js'
import { CRITIC_STATUSES, type CriticFailure, type CriticStatus, type CriticTokens } from './critic.js'
import { JsonFields } from './json.js'
import type { LedgerCriticRun } from './ledger.js'
import type { PromptHashes } from './prompt.js'
import { CRITIQUE_STAGES, type CritiqueStage, type Round } from './stage.js'

/**
 * One critic run as the gateway's ledger holds it: the gateway's own fields
 * (client, model, family, mode, workflow, tokens, dollars, outcome) plus
 * everything critique's own line carries, so the gateway counts a critic run
 * the way it counts a delegation and critique's stats read it back.
 *
 * The gateway's `client` and `model` are the gateway's client id and model
 * alias; critique's own client name and model are `critic_client` and
 * `critic_model`, because a critic is pinned to one client model of a client
 * that offers several. A run that is not a review — the triage model's
 * question — carries no prompt hashes, role or lenses, so stats reads it as
 * spend only.
 */
export interface CritiqueLedgerRecord {
  /** When the run ended, as an ISO timestamp: the gateway's `at`, and critique's `recorded_at`. */
  readonly at: string
  readonly recorded_at: string
  readonly client: string
  readonly critic_client: string
  readonly model: string
  readonly critic_model: string
  readonly family: string
  readonly mode: 'read-only'
  readonly workflow: string
  readonly repository: string
  readonly run_id: string
  /**
   * What the client reported the run used; absent when it reported none, which
   * a timed-out or failed run does. The gateway counts a reading it cannot get
   * as unknown, never as zero.
   */
  readonly tokens?: CriticTokens
  /** What the gateway counts as the run's spend: what the run was charged, as for a delegation. */
  readonly cost_usd: number
  /** What the client reported the run cost; null when it reported nothing. */
  readonly critic_cost_usd: number | null
  readonly duration_s: number
  readonly outcome: 'ok' | 'failed'
  readonly reason?: string
  /** Critique's own line type, so a reader of the gateway's ledger knows whose record it is. */
  readonly type: 'invocation'
  readonly critic_id: string
  readonly stage: CritiqueStage
  readonly round: Round
  readonly kind: 'vibe' | 'agy'
  readonly charged_usd: number
  readonly status: CriticStatus
  readonly exit_code: number | null
  readonly finding_count?: number
  readonly role?: string
  readonly lenses?: readonly string[]
  readonly prompt_hashes?: PromptHashes
  readonly lens_hashes?: Readonly<Record<string, string>>
  readonly failure?: CriticFailure
  readonly retry_of?: string
  readonly spec?: string
  readonly vibe_cost_usd?: number | null
}

/** The critic client a run went through, as the gateway's record names it. */
export interface LedgerClient {
  readonly kind: 'vibe' | 'agy'
  /** The gateway client model the critic client is pinned to; without it the client runs its own model. */
  readonly gateway?: { readonly client: string; readonly model: string }
}

/** What one critic run says about itself, as the gateway's record carries it. */
export interface LedgerRun {
  readonly charged_usd: number
  readonly client: string
  readonly cost_usd: number | null
  readonly critic_id: string
  readonly exit_code: number | null
  readonly family: string
  readonly model: string
  readonly status: CriticStatus
  readonly duration_ms?: number
  readonly failure?: CriticFailure
  readonly finding_count?: number
  readonly lens_hashes?: Readonly<Record<string, string>>
  readonly lenses?: readonly string[]
  readonly prompt_hashes?: PromptHashes
  readonly retry_of?: string
  readonly role?: string
  readonly tokens?: CriticTokens
  readonly vibe_cost_usd?: number | null
}

/** What a critic run needs to become the gateway's record: the run it belongs to. */
export interface CritiqueLedgerContext {
  readonly flow: string
  readonly repository: string
  readonly round: Round
  readonly run_id: string
  readonly spec?: string
  readonly stage: CritiqueStage
}

/**
 * One critic run as the gateway's ledger line. The gateway's `reason` is its
 * own vocabulary, so only a failure it names is carried over; every other
 * failure stays in critique's `failure`.
 */
export function critiqueLedgerRecord(
  context: CritiqueLedgerContext,
  run: LedgerRun,
  client: LedgerClient,
  at: string,
): CritiqueLedgerRecord {
  const reason = gatewayReason(run)
  return {
    at,
    charged_usd: run.charged_usd,
    client: client.gateway?.client ?? client.kind,
    cost_usd: run.charged_usd,
    critic_client: run.client,
    critic_cost_usd: run.cost_usd,
    critic_id: run.critic_id,
    critic_model: run.model,
    duration_s: (run.duration_ms ?? 0) / 1000,
    exit_code: run.exit_code,
    family: run.family,
    ...(run.failure === undefined ? {} : { failure: run.failure }),
    ...(run.finding_count === undefined ? {} : { finding_count: run.finding_count }),
    kind: client.kind,
    ...(run.lens_hashes === undefined ? {} : { lens_hashes: run.lens_hashes }),
    ...(run.lenses === undefined ? {} : { lenses: run.lenses }),
    mode: 'read-only',
    model: client.gateway?.model ?? run.model,
    outcome: run.status === 'ok' ? 'ok' : 'failed',
    ...(run.prompt_hashes === undefined ? {} : { prompt_hashes: run.prompt_hashes }),
    ...(reason === undefined ? {} : { reason }),
    recorded_at: at,
    repository: context.repository,
    ...(run.retry_of === undefined ? {} : { retry_of: run.retry_of }),
    ...(run.role === undefined ? {} : { role: run.role }),
    round: context.round,
    run_id: context.run_id,
    ...(context.spec === undefined ? {} : { spec: context.spec }),
    stage: context.stage,
    status: run.status,
    ...(run.tokens === undefined ? {} : { tokens: run.tokens }),
    type: 'invocation',
    ...(run.vibe_cost_usd === undefined ? {} : { vibe_cost_usd: run.vibe_cost_usd }),
    workflow: context.flow,
  }
}

/** The gateway's own reason for a failed run, where critique's failure is one the gateway names. */
function gatewayReason(run: LedgerRun): string | undefined {
  if (run.status === 'ok' || run.failure === undefined) return undefined
  if (run.failure === 'timeout') return 'timeout'
  if (run.failure === 'unparseable-output' || run.failure === 'empty-output' || run.failure === 'missing-structured-output') {
    return 'parse-error'
  }
  return /^exit-\d+$/.test(run.failure) ? run.failure : undefined
}

/** What the gateway's ledger says about critique's runs, read back as critique's own lines. */
export interface GatewayLedgerLines {
  readonly runs: readonly LedgerCriticRun[]
  readonly invocations: readonly LoggedInvocation[]
}

/**
 * The line a stage's summary carries when the gateway's ledger would not take
 * a run: the critics already ran and spent money, so the stage goes on, and
 * the owner is told the run is in critique's own ledger only.
 */
export function gatewayLedgerWarning(message: string): string {
  return `the gateway ledger could not be written: ${message}; this run is recorded in critique's own ledger only`
}

/**
 * Reads the gateway's ledger for critique's records. A line that is not one
 * of them — a delegation's record, a line a later version wrote, a write cut
 * short — is skipped, so the gateway's own history never hides critique's.
 * A record without prompt hashes is a run that reviewed nothing, such as the
 * triage model's question: it counts as spend, never as a critic's cell.
 */
export function gatewayLedgerLines(text: string): GatewayLedgerLines {
  const runs: LedgerCriticRun[] = []
  const invocations: LoggedInvocation[] = []
  for (const line of text.split('\n')) {
    const record = critiqueRecord(line)
    if (record === undefined) continue
    invocations.push(record.invocation)
    if (record.run !== undefined) runs.push(record.run)
  }
  return { invocations, runs }
}

function critiqueRecord(
  line: string,
): { readonly run: LedgerCriticRun | undefined; readonly invocation: LoggedInvocation } | undefined {
  if (line.trim() === '') return undefined
  try {
    return readRecord(line)
  } catch {
    // A line a later version wrote, or a write cut short: the rest of the ledger still counts.
    return undefined
  }
}

function readRecord(
  line: string,
): { readonly run: LedgerCriticRun | undefined; readonly invocation: LoggedInvocation } | undefined {
  const fields = JsonFields.parse(line)
  if (!fields.has('type') || fields.optionalString('type') !== 'invocation') return undefined
  const context = {
    flow: fields.string('workflow'),
    recorded_at: fields.string('recorded_at'),
    repository: fields.string('repository'),
    round: fields.number('round'),
    run_id: fields.string('run_id'),
    spec: fields.has('spec') ? fields.string('spec') : undefined,
    stage: fields.oneOf('stage', CRITIQUE_STAGES),
  }
  const invocation: LoggedInvocation = {
    charged_usd: fields.number('charged_usd'),
    critic_id: fields.string('critic_id'),
    flow: context.flow,
    kind: fields.oneOf('kind', ['vibe', 'agy']),
    recorded_at: context.recorded_at,
    repository: context.repository,
    run_id: context.run_id,
  }
  if (!fields.has('prompt_hashes')) return { invocation, run: undefined }
  const hashes = fields.object('prompt_hashes')
  return {
    invocation,
    run: {
      ...context,
      client: fields.string('critic_client'),
      cost_usd: fields.nullableNumber('critic_cost_usd'),
      critic_id: fields.string('critic_id'),
      failure: fields.optionalString('failure') ?? null,
      // A run that leaves these undefined is written without them, so a record this code wrote reads back.
      finding_count: fields.has('finding_count') ? fields.number('finding_count') : 0,
      lens_hashes: fields.has('lens_hashes') ? fields.stringMap('lens_hashes') : {},
      lenses: fields.has('lenses') ? fields.strings('lenses') : [],
      model: fields.string('critic_model'),
      prompt_hashes: {
        contract: hashes.string('contract'),
        preamble: hashes.string('preamble'),
        role: hashes.string('role'),
        schema: hashes.string('schema'),
      },
      role: fields.optionalString('role') ?? '',
      status: fields.oneOf('status', CRITIC_STATUSES),
    },
  }
}
