import type { RunMode, RunTokens } from './run.js'

/**
 * One line of the gateway's ledger: what one run was, what it cost and how it
 * ended. It is machine-wide, so a later ticket can count a workflow's spend
 * and a client's usage from it.
 */
export interface LedgerRecord {
  /** When the run ended, as an ISO timestamp. */
  readonly at: string
  readonly client: string
  readonly cost_usd: number
  /** How long the run took, in seconds. */
  readonly duration_s: number
  readonly family: string
  readonly mode: RunMode
  readonly model: string
  readonly outcome: 'ok' | 'failed'
  /** Why a failed run failed: `timeout`, `parse-error`, `exit-<n>` or `launch-failed`. */
  readonly reason?: string
  /** The repository the run worked in, keyed `<host>/<owner>/<repo>`; null for an inline run. */
  readonly repository: string | null
  readonly run_id: string
  readonly tokens: RunTokens
  readonly workflow: string
}

/** The record as the ledger holds it: one JSON object on one line. */
export function ledgerLine(record: LedgerRecord): string {
  return `${JSON.stringify(record)}\n`
}
