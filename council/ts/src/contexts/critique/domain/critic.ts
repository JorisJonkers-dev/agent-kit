import { MODEL_FAMILIES, type ModelFamily } from './family.js'
import { FINDINGS_FAILURES, type CriticFinding, type FindingsFailure } from './findings.js'
import { isOneOf, type JsonFields } from './json.js'
import type { PromptHashes } from './prompt.js'
import type { RosterCritic } from './routing.js'

/** Who a critic is in one run: its critic client, family and model, its role and its lenses. */
export interface CriticIdentity {
  readonly critic_id: string
  readonly client: string
  readonly family: ModelFamily
  readonly model: string
  readonly role: string
  readonly lenses: readonly string[]
}

const RUN_FAILURES = [
  ...FINDINGS_FAILURES,
  'empty-output',
  'unparseable-output',
  'missing-export',
  'unsafe-config',
  'config-error',
  'infrastructure-failure',
  'stopped-early',
  'print-timeout',
  'denied-action',
  'missing-structured-output',
  'unsafe-tool',
  /** The CLI outlived its outer timeout and was killed. */
  'timeout',
] as const

/** Why a critic's run counts as no review. `exit-<code>` is an exit code the client does not document. */
export type CriticFailure = FindingsFailure | (typeof RUN_FAILURES)[number] | `exit-${string}`

/** What a critic client makes of one critic's run: a review, or the reason it counts as none. */
export type CriticOutcome =
  | {
      readonly status: 'ok'
      readonly findings: readonly CriticFinding[]
      readonly no_findings_reason?: string
      readonly cost_usd: number | null
    }
  | { readonly status: 'failed'; readonly failure: CriticFailure; readonly detail: string; readonly cost_usd: number | null }

/** One critic's run as fan-out recorded it: who ran, how it went, and where its raw output is. */
export const CRITIC_STATUSES = ['ok', 'failed', 'violated-readonly', 'budget-skip'] as const

/**
 * `failed`: the critic gave no usable review. `violated-readonly`: it changed
 * its packet or the working tree, so its findings were discarded.
 * `budget-skip`: the flow had spent its Vibe cap, so it was never launched.
 */
export type CriticStatus = (typeof CRITIC_STATUSES)[number]

/** What one run used, as its client reported it; cached input is prompt the provider served from its cache. */
export interface CriticTokens {
  readonly cached: number
  readonly input: number
  readonly output: number
}

export interface CriticRun extends CriticIdentity {
  /** The critic id of the failed critic this one is the fallback retry for. */
  readonly retry_of?: string
  readonly status: CriticStatus
  readonly failure?: CriticFailure
  readonly detail?: string
  readonly no_findings_reason?: string
  /** Null for a critic that was never launched. */
  readonly exit_code: number | null
  /**
   * What the critic cost: for Vibe, computed from its tokens at routing's
   * prices; otherwise what its client reported. Null when nothing is known.
   */
  readonly cost_usd: number | null
  /** What Vibe itself reported, kept for comparison; absent for any other client. */
  readonly vibe_cost_usd?: number | null
  /** What the run used, as its client reported it; absent when the client reports none. */
  readonly tokens?: CriticTokens
  /** What the launch counts against the flow's Vibe cap. */
  readonly charged_usd: number
  readonly duration_ms: number
  readonly finding_count: number
  readonly prompt_hashes: PromptHashes
  readonly lens_hashes: Readonly<Record<string, string>>
  readonly prompt_path: string
  /** Absent for a critic that was never launched. */
  readonly stdout_path?: string
  readonly export_path?: string
  /** The tools the critic had, as its prompt named them; absent for a critic that was never launched. */
  readonly tools?: readonly string[]
  /** Why the critic could not search, or why its searches failed during the run (ADR-0006). */
  readonly search_failure?: string
}

/** The identity a roster critic carries into fan-out, record and the ledger. */
export function criticIdentity(critic: RosterCritic): CriticIdentity {
  return {
    client: critic.client,
    critic_id: critic.critic_id,
    family: critic.client_config.family,
    lenses: critic.lenses,
    model: critic.client_config.model,
    role: critic.role,
  }
}

export function readCriticRun(fields: JsonFields): CriticRun {
  const failure = fields.optionalMatching('failure', isCriticFailure, 'a critic failure')
  const detail = fields.optionalString('detail')
  const noFindingsReason = fields.optionalString('no_findings_reason')
  const retryOf = fields.optionalString('retry_of')
  const stdoutPath = fields.optionalString('stdout_path')
  const exportPath = fields.optionalString('export_path')
  const vibeCost = fields.has('vibe_cost_usd') ? { vibe_cost_usd: fields.nullableNumber('vibe_cost_usd') } : {}
  const tokens = fields.has('tokens') ? { tokens: readTokens(fields.object('tokens')) } : {}
  const searchFailure = fields.optionalString('search_failure')
  const hashes = fields.object('prompt_hashes')
  return {
    charged_usd: fields.number('charged_usd'),
    client: fields.string('client'),
    cost_usd: fields.nullableNumber('cost_usd'),
    critic_id: fields.string('critic_id'),
    duration_ms: fields.number('duration_ms'),
    exit_code: fields.nullableNumber('exit_code'),
    family: fields.oneOf('family', MODEL_FAMILIES),
    finding_count: fields.number('finding_count'),
    lens_hashes: fields.stringMap('lens_hashes'),
    lenses: fields.strings('lenses'),
    model: fields.string('model'),
    prompt_hashes: {
      contract: hashes.string('contract'),
      preamble: hashes.string('preamble'),
      role: hashes.string('role'),
      schema: hashes.string('schema'),
    },
    prompt_path: fields.string('prompt_path'),
    role: fields.string('role'),
    status: fields.oneOf('status', CRITIC_STATUSES),
    ...(failure === undefined ? {} : { failure }),
    ...(detail === undefined ? {} : { detail }),
    ...(noFindingsReason === undefined ? {} : { no_findings_reason: noFindingsReason }),
    ...(retryOf === undefined ? {} : { retry_of: retryOf }),
    ...(stdoutPath === undefined ? {} : { stdout_path: stdoutPath }),
    ...(exportPath === undefined ? {} : { export_path: exportPath }),
    ...vibeCost,
    ...tokens,
    ...(fields.has('tools') ? { tools: fields.strings('tools') } : {}),
    ...(searchFailure === undefined ? {} : { search_failure: searchFailure }),
  }
}

function isCriticFailure(value: string): value is CriticFailure {
  return isOneOf(RUN_FAILURES, value) || /^exit-\d+$/.test(value)
}

function readTokens(fields: JsonFields): CriticTokens {
  return { cached: fields.number('cached'), input: fields.number('input'), output: fields.number('output') }
}
