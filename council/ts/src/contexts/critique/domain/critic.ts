import { MODEL_FAMILIES, type ModelFamily } from './family.js'
import { FINDINGS_FAILURES, type FindingsFailure } from './findings.js'
import { isOneOf, type JsonFields } from './json.js'
import type { PromptHashes } from './prompt.js'

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
] as const

/** Why a critic's run counts as no review. `exit-<code>` is an exit code the client does not document. */
export type CriticFailure = FindingsFailure | (typeof RUN_FAILURES)[number] | `exit-${string}`

/** One critic's run as fan-out recorded it: who ran, how it went, and where its raw output is. */
export interface CriticRun extends CriticIdentity {
  readonly status: 'ok' | 'failed'
  readonly failure?: CriticFailure
  readonly detail?: string
  readonly no_findings_reason?: string
  readonly exit_code: number
  readonly cost_usd: number | null
  readonly duration_ms: number
  readonly finding_count: number
  readonly prompt_hashes: PromptHashes
  readonly lens_hashes: Readonly<Record<string, string>>
  readonly prompt_path: string
  readonly stdout_path: string
  readonly export_path: string
}

export function readCriticRun(fields: JsonFields): CriticRun {
  const failure = fields.optionalMatching('failure', isCriticFailure, 'a critic failure')
  const detail = fields.optionalString('detail')
  const noFindingsReason = fields.optionalString('no_findings_reason')
  const hashes = fields.object('prompt_hashes')
  return {
    client: fields.string('client'),
    cost_usd: fields.nullableNumber('cost_usd'),
    critic_id: fields.string('critic_id'),
    duration_ms: fields.number('duration_ms'),
    exit_code: fields.number('exit_code'),
    export_path: fields.string('export_path'),
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
    status: fields.oneOf('status', ['ok', 'failed']),
    stdout_path: fields.string('stdout_path'),
    ...(failure === undefined ? {} : { failure }),
    ...(detail === undefined ? {} : { detail }),
    ...(noFindingsReason === undefined ? {} : { no_findings_reason: noFindingsReason }),
  }
}

function isCriticFailure(value: string): value is CriticFailure {
  return isOneOf(RUN_FAILURES, value) || /^exit-\d+$/.test(value)
}
