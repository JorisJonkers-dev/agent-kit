import type { CostClass, MeteredPersonalCaps, ModelPrices, Tier } from './config.js'
import {
  isReservation,
  isRunRecord,
  isSettled,
  type LedgerEntry,
  type ReservationRecord,
  type SettledRecord,
} from './ledger.js'

/**
 * What a metered run costs before it runs, and what it may hold against the
 * caps while it does.
 *
 * A run's estimate is its prompt's tokens at the model's input price plus its
 * difficulty tier's output allowance at the model's output price. The estimate
 * is reserved against the per-workflow and per-day caps before the run starts,
 * and the run's actual spend replaces it when it ends. A reservation whose run
 * left no settlement lapses at the run's time limit plus a grace period, and
 * counts its full estimate as spent.
 */

/** How many characters of a prompt the gateway counts as one token. */
export const CHARS_PER_TOKEN = 3.5

/** How long a reservation outlives its run's time limit, so a settle that races the run still finds it. */
export const RESERVATION_GRACE_S = 60

/** What a metered run is estimated to cost before it starts. */
export interface Estimate {
  /** The prompt's tokens: its characters over 3.5, rounded up. */
  readonly prompt_tokens: number
  /** The run's difficulty tier's output allowance. */
  readonly output_tokens: number
  readonly usd: number
}

/** The prompt's tokens: its characters over 3.5, rounded up. */
export function promptTokens(prompt: string): number {
  return Math.ceil(prompt.length / CHARS_PER_TOKEN)
}

/**
 * What a run is estimated to cost: its prompt's tokens at the model's input
 * price, plus its tier's output allowance at the model's output price. A model
 * the catalogue prices nowhere, or gives no tier, cannot be estimated, and a
 * metered run on it is refused rather than run uncapped.
 */
export function estimateRun(
  prompt: string,
  tier: Tier | undefined,
  prices: ModelPrices | undefined,
  allowances: Readonly<Record<Tier, number>>,
): Estimate | undefined {
  if (tier === undefined || prices === undefined) return undefined
  const prompt_tokens = promptTokens(prompt)
  const output_tokens = allowances[tier]
  return {
    output_tokens,
    prompt_tokens,
    usd: roundUp((prompt_tokens * prices.input + output_tokens * prices.output) / 1_000_000),
  }
}

/** The caps a metered-personal run is held to, as the machine names them. */
export type CapName = 'per_workflow_usd' | 'per_day_usd'

/** One cap, and what it holds for the run that asks: its limit, what is spent and what is reserved. */
export interface CapRoom {
  readonly cap: CapName
  readonly limit_usd: number
  readonly spent_usd: number
  readonly reserved_usd: number
}

/** Whether a run fits the caps, and the cap it does not fit. */
export interface CapDecision {
  /** The cap the run does not fit, with what it holds; undefined when the run fits every cap. */
  readonly refused?: CapRoom
}

/** One settled charge against the caps: what a run cost, under which workflow, on which UTC day. */
interface Charge {
  readonly day: string
  readonly usd: number
  readonly workflow: string
}

/**
 * Whether a run's estimate fits both caps: what the ledger says is spent and
 * what live reservations hold, in the run's workflow and on the reservation's
 * UTC day, plus the estimate itself. The workflow cap is checked first, so a
 * run that fits neither is refused for its workflow.
 *
 * Every line counts by the cost class it was charged under, never by its
 * client's own class: a subsidised client's metered provider is held too.
 */
export function checkCaps(
  entries: readonly LedgerEntry[],
  workflow: string,
  estimateUsd: number,
  caps: MeteredPersonalCaps,
  now: Date,
): CapDecision {
  const day = utcDay(now.toISOString())
  const charges = chargesOf(entries)
  const held = liveReservations(entries).filter((reservation) => reservation.cost_class === 'metered-personal')
  const workflowRoom = room(
    'per_workflow_usd',
    caps.per_workflow_usd,
    sum(charges.filter((charge) => charge.workflow === workflow).map((charge) => charge.usd)),
    sum(held.filter((reservation) => reservation.workflow === workflow).map((reservation) => reservation.estimate_usd)),
  )
  if (!fits(workflowRoom, estimateUsd)) return { refused: workflowRoom }
  const dayRoom = room(
    'per_day_usd',
    caps.per_day_usd,
    sum(charges.filter((charge) => charge.day === day).map((charge) => charge.usd)),
    sum(held.filter((reservation) => utcDay(reservation.reserved_at) === day).map((reservation) => reservation.estimate_usd)),
  )
  return fits(dayRoom, estimateUsd) ? {} : { refused: dayRoom }
}

/** Why a run does not fit a cap, naming the cap, its limit, what is spent and reserved, and the estimate. */
export function capRefusalMessage(room: CapRoom, estimateUsd: number, workflow: string, now: Date): string {
  const [name, where] =
    room.cap === 'per_workflow_usd'
      ? ['per-workflow', `in workflow ${workflow}`]
      : ['per-day', `on ${utcDay(now.toISOString())}`]
  return `does not fit the ${name} cap of ${usd(room.limit_usd)}: ${usd(room.spent_usd)} spent and ${usd(room.reserved_usd)} reserved ${where}, and its estimate is ${usd(estimateUsd)}`
}

/** The reservation a metered run holds while it runs: its estimate, and when it lapses. */
export function reservationRecord(args: {
  readonly run_id: string
  readonly workflow: string
  readonly client: string
  readonly cost_class: CostClass
  readonly model: string
  readonly estimate_usd: number
  readonly reserved_at: Date
  readonly time_limit_s: number
  readonly run_dir: string
}): ReservationRecord {
  return {
    client: args.client,
    cost_class: args.cost_class,
    estimate_usd: args.estimate_usd,
    expires_at: new Date(args.reserved_at.getTime() + (args.time_limit_s + RESERVATION_GRACE_S) * 1000).toISOString(),
    kind: 'reservation',
    model: args.model,
    reserved_at: args.reserved_at.toISOString(),
    run_dir: args.run_dir,
    run_id: args.run_id,
    workflow: args.workflow,
  }
}

/** The settlement that releases a reservation: what the run really cost. */
export function settledRecord(reservation: ReservationRecord, costUsd: number, at: Date): SettledRecord {
  return {
    client: reservation.client,
    cost_class: reservation.cost_class,
    cost_usd: costUsd,
    kind: 'settled',
    reserved_at: reservation.reserved_at,
    run_id: reservation.run_id,
    settled_at: at.toISOString(),
    workflow: reservation.workflow,
  }
}

/** The reservations no settlement has released yet: what the caps hold right now. */
export function liveReservations(entries: readonly LedgerEntry[]): readonly ReservationRecord[] {
  const settled = new Set(entries.filter(isSettled).map((entry) => entry.run_id))
  return entries.filter(isReservation).filter((reservation) => !settled.has(reservation.run_id))
}

/**
 * The settlements a ledger needs: every reservation whose run left no settled
 * record and whose expiry has passed, settled at the cost its run directory
 * reported, or at its full estimate when the run left nothing behind.
 */
export function settleExpired(
  entries: readonly LedgerEntry[],
  now: Date,
  costs: ReadonlyMap<string, number>,
): readonly SettledRecord[] {
  return liveReservations(entries)
    .filter((reservation) => Date.parse(reservation.expires_at) <= now.getTime())
    .map((reservation) => ({
      client: reservation.client,
      cost_class: reservation.cost_class,
      cost_usd: costs.get(reservation.run_id) ?? reservation.estimate_usd,
      kind: 'settled',
      reason: 'expired',
      reserved_at: reservation.reserved_at,
      run_id: reservation.run_id,
      settled_at: now.toISOString(),
      workflow: reservation.workflow,
    }))
}

/**
 * What the ledger says metered runs have cost, by workflow and by UTC day. A
 * run's own line carries its spend; a settlement counts only for a run that
 * has no line, which is a reservation that expired. Both count by the cost
 * class they were charged under, so a metered provider of a subsidised client
 * is held too.
 */
function chargesOf(entries: readonly LedgerEntry[]): readonly Charge[] {
  const runs = entries.filter(isRunRecord).filter((run) => run.cost_class === 'metered-personal')
  const settled = new Map(entries.filter(isSettled).map((entry) => [entry.run_id, entry]))
  const charges: Charge[] = runs.map((run) => ({
    day: utcDay(settled.get(run.run_id)?.reserved_at ?? run.at),
    usd: run.cost_usd,
    workflow: run.workflow,
  }))
  for (const settle of settled.values()) {
    if (settle.reason !== 'expired' || settle.cost_class !== 'metered-personal') continue
    if (runs.some((run) => run.run_id === settle.run_id)) continue
    charges.push({ day: utcDay(settle.reserved_at), usd: settle.cost_usd, workflow: settle.workflow })
  }
  return charges
}

function room(cap: CapName, limit_usd: number, spent_usd: number, reserved_usd: number): CapRoom {
  return { cap, limit_usd, reserved_usd: round(reserved_usd, 6), spent_usd: round(spent_usd, 6) }
}

/** Whether the estimate still fits what a cap has left, to the cent the ledger counts in. */
function fits(room: CapRoom, estimateUsd: number): boolean {
  return round(room.spent_usd + room.reserved_usd + estimateUsd, 6) <= room.limit_usd
}

/** The UTC day a moment falls on, as `YYYY-MM-DD`. */
function utcDay(at: string): string {
  return at.slice(0, 10)
}

function sum(values: readonly number[]): number {
  return values.reduce((total, value) => total + value, 0)
}

/** Dollars as a message names them. */
function usd(value: number): string {
  return `$${value.toFixed(2)}`
}

/** A sum of dollars, rounded so the JSON does not carry float noise. */
function round(value: number, places: number): number {
  const factor = 10 ** places
  return Math.round(value * factor) / factor
}

/**
 * An estimate, rounded up so a hard spend limit is never below what the run may
 * cost. The epsilon keeps float noise (1.4 * 1000 landing a hair above 1400)
 * from rounding a whole micro-dollar up.
 */
function roundUp(value: number): number {
  return Math.ceil(value * 1_000_000 - 1e-6) / 1_000_000
}
