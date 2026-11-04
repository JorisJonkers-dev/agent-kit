import type { CostClass } from './config.js'
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
  /**
   * The cost class the run was held to, which is the class of the provider it
   * ran on where the client names one. A line written before the caps carry
   * none, and counts against no cap.
   */
  readonly cost_class?: CostClass
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
  /** A retry's or a climb's place in its delegation (2 and up); the delegation's first run carries none. */
  readonly attempt?: number
  /** The run id of the delegation's first attempt, on every later one. */
  readonly attempt_of?: string
  /**
   * What the client reported the run used; absent when it reported none, which
   * a timed-out or failed run does. A reading the gateway cannot get is
   * unknown, never zero, so a reader counts it as unknown.
   */
  readonly tokens?: RunTokens
  readonly workflow: string
}

/**
 * A metered run's estimate, held against the caps while it runs. It is its own
 * line so that two gateway processes can see what the other holds, and so that
 * a reservation whose run died is settled on a later call.
 */
export interface ReservationRecord {
  readonly kind: 'reservation'
  readonly run_id: string
  readonly workflow: string
  readonly client: string
  /**
   * The cost class the run is charged under, which is its provider's where the
   * client names one. The caps count a reservation by this, never by the
   * client's own class: a subsidised client's metered provider is still held.
   */
  readonly cost_class: CostClass
  readonly model: string
  /** The dollars the run is estimated to cost, held until it settles. */
  readonly estimate_usd: number
  readonly reserved_at: string
  /** When the reservation lapses: the run's time limit plus a grace period. */
  readonly expires_at: string
  /** The run's own directory, where a settle finds the result the run left. */
  readonly run_dir: string
}

/**
 * A reservation released: what the run really cost, or its full estimate when
 * the run left nothing behind. A run's own line carries its spend too, so a
 * settlement only counts against the caps for a run that has no line.
 */
export interface SettledRecord {
  readonly kind: 'settled'
  readonly run_id: string
  readonly workflow: string
  readonly client: string
  /** The cost class the run was charged under, copied from its reservation. */
  readonly cost_class: CostClass
  readonly cost_usd: number
  /** The reservation's own moment, which is the UTC day the spend counts on. */
  readonly reserved_at: string
  readonly settled_at: string
  /**
   * Why the gateway settled a reservation no run settled itself: `expired`
   * when its run left no line of its own, `not-started` when the run was
   * released before it launched and cost nothing.
   */
  readonly reason?: 'expired' | 'not-started'
}

/** One line of the ledger: a run (the older lines carry no `kind`), a reservation, or a settlement. */
export type LedgerEntry = LedgerRecord | ReservationRecord | SettledRecord

/**
 * Whether a line is a run's own record: every line that is not a reservation
 * or a settlement. A run line may carry a `kind` of its own (critique's
 * records name the client kind), so only the two bookkeeping kinds are told apart.
 */
export function isRunRecord(entry: LedgerEntry): entry is LedgerRecord {
  return !isReservation(entry) && !isSettled(entry)
}

export function isReservation(entry: LedgerEntry): entry is ReservationRecord {
  return 'kind' in entry && entry.kind === 'reservation'
}

export function isSettled(entry: LedgerEntry): entry is SettledRecord {
  return 'kind' in entry && entry.kind === 'settled'
}

/** The runs the ledger holds, oldest first: every line that is not a reservation or a settlement. */
export function runRecords(entries: readonly LedgerEntry[]): readonly LedgerRecord[] {
  return entries.filter(isRunRecord)
}

/** The record as the ledger holds it: one JSON object on one line. */
export function ledgerLine(entry: LedgerEntry): string {
  return `${JSON.stringify(entry)}\n`
}
