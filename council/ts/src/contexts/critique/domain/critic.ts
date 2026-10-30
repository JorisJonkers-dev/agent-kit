import type { ModelFamily } from './family.js'
import type { FindingsFailure } from './findings.js'
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

/** Why a critic's run counts as no review. */
export type CriticFailure =
  | FindingsFailure
  | 'empty-output'
  | 'unparseable-output'
  | 'missing-export'
  | 'unsafe-config'
  | 'config-error'
  | 'infrastructure-failure'
  | 'stopped-early'
  | `exit-${string}`

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
