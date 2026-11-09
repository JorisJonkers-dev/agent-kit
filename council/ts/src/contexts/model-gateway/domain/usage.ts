import { liveReservations } from './caps.js'
import { isObject, ModelGatewayError, type CostClass, type GatewayConfig, type PlanLimit, type PlanWindow } from './config.js'
import { runRecords, type LedgerEntry, type LedgerRecord } from './ledger.js'
import { sortedClients } from './models.js'
import type { RunTokens } from './run.js'
import { requireWorkflowName } from './workflows.js'

/**
 * What the gateway can say about a client before it plans: whether it can run
 * now, what it has spent, what its own plan has left, and how close it is to
 * the plan limit the owner set for it.
 *
 * Availability is found passively. The gateway asks PATH, the client's own
 * status command where it has one, and the ledger's recent outcomes; it never
 * sends a prompt to find out. A reading it cannot get is `unknown`, never a
 * zero that would read as an exhausted plan.
 */

/** How many days of usage a report covers when the caller names none. */
export const DEFAULT_USAGE_DAYS = 7

const DAY_MS = 24 * 60 * 60 * 1000

/** How far back each plan window reaches from now. */
const WINDOW_MS: Readonly<Record<PlanWindow, number>> = {
  '5h': 5 * 60 * 60 * 1000,
  day: DAY_MS,
  month: 30 * DAY_MS,
  week: 7 * DAY_MS,
}

/** What a `usage` call asks for: how many days, and optionally one workflow. */
export interface UsageInput {
  readonly days: number
  readonly workflow?: string
}

/** Reads a usage call's input, or says what is wrong with it. */
export function readUsageInput(raw: unknown): UsageInput {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
  const days = raw.days === undefined ? DEFAULT_USAGE_DAYS : raw.days
  if (typeof days !== 'number' || !Number.isInteger(days) || days <= 0) {
    throw new ModelGatewayError('invalid-input', 'days must be a whole number of days above 0')
  }
  const workflow = raw.workflow
  if (workflow === undefined) return { days }
  return { days, workflow: requireWorkflowName(workflow, 'workflow must be a non-empty string') }
}

/** What a client's own status command said about it, and which command said it. */
export interface ClientStatusReading {
  /** The command, as the owner would type it. */
  readonly command: string
  readonly signed_in: 'yes' | 'no' | 'unknown'
  /** One line saying what the reading rests on. */
  readonly detail: string
}

/**
 * A client model whose own answer said it cannot run, and why: the gateway
 * remembers it until the reset the answer named, or, when it named none, until
 * a run of the client model succeeded.
 */
export interface StandingRefusal {
  readonly client: string
  readonly model: string
  /** One line naming what the answer said, and when it said it ends. */
  readonly reason: string
}

/** What the gateway found out about one client before it read the ledger. */
export interface ClientReading {
  readonly client: string
  readonly installed: boolean
  /** What the client's own status command said; undefined when it has none. */
  readonly status: ClientStatusReading | undefined
}

/** Whether a client can run now, and what that rests on. */
export interface Availability {
  /** `unable_to_start` is a plan window at 100%; `unavailable` is anything else that stops it. */
  readonly status: 'available' | 'unavailable' | 'unable_to_start'
  readonly installed: boolean
  readonly signed_in: 'yes' | 'no' | 'unknown'
  /** One line saying what the reading rests on. */
  readonly reason: string
}

/** What a client itself says is left of its plan; `unknown` when it reports none. */
export interface Quota {
  readonly status: 'unknown'
  readonly reason: string
}

/** What a set of runs came to. */
export interface UsageTotals {
  readonly runs: number
  /**
   * The tokens the runs reported; absent when one of them reported none, since
   * the gateway cannot know the total. A run whose count is unknown is still
   * counted in `runs` and in `cost_usd`.
   */
  readonly tokens?: RunTokens
  readonly cost_usd: number
}

export interface DayUsage extends UsageTotals {
  /** The UTC day, as `YYYY-MM-DD`. */
  readonly day: string
}

export interface WorkflowUsage extends UsageTotals {
  readonly workflow: string
}

/** How a plan window stands: below the warning ratio, at it, or spent out. */
export type PlanState = 'ok' | 'warning' | 'exhausted'

/** One window of a client's plan, with what the ledger says is spent in it. */
export type PlanWindowUsage =
  | {
      readonly window: PlanWindow
      readonly limit_usd: number
      readonly spent_usd: number
      readonly ratio: number
      readonly state: PlanState
    }
  | {
      readonly window: PlanWindow
      readonly limit_runs: number
      readonly runs: number
      readonly ratio: number
      readonly state: PlanState
    }

export interface ClientUsage {
  readonly client: string
  readonly cost_class: CostClass
  readonly availability: Availability
  readonly quota: Quota
  readonly usage: UsageTotals
  readonly by_day: readonly DayUsage[]
  readonly by_workflow: readonly WorkflowUsage[]
  readonly plan_limits: readonly PlanWindowUsage[]
  /** What this client's live reservations hold against the caps now, and how many there are. */
  readonly reserved_usd: number
  readonly reserved_runs: number
  /** One line per plan window at or above the warning ratio. */
  readonly warnings: readonly string[]
}

export interface UsageResult {
  readonly status: 'ok'
  readonly days: number
  /** The workflow the usage was narrowed to, when the caller named one. */
  readonly workflow?: string
  readonly clients: readonly ClientUsage[]
}

/**
 * What every client on this machine can do now and what it has spent: the
 * usage of the last `days` days, by day and by workflow, and each client's
 * plan windows against the ledger.
 */
export function usageReport(
  input: UsageInput,
  config: GatewayConfig,
  readings: readonly ClientReading[],
  entries: readonly LedgerEntry[],
  now: Date,
): UsageResult {
  const records = runRecords(entries)
  const held = liveReservations(entries)
  const since = startOfDay(now) - (input.days - 1) * DAY_MS
  const counted = records.filter(
    (record) => Date.parse(record.at) >= since && (input.workflow === undefined || record.workflow === input.workflow),
  )
  const byClient = new Map(readings.map((reading) => [reading.client, reading]))
  const clients = sortedClients(config).map(([id, client]) => {
    const reading = byClient.get(id) ?? { client: id, installed: false, status: undefined }
    const mine = counted.filter((record) => record.client === id)
    const reservations = held.filter((reservation) => reservation.client === id)
    const windows = planWindows(id, client.plan_limits ?? [], records, now, config.settings.plan_warning_ratio)
    return {
      availability: availabilityOf(id, client.binary, reading, records, now, config.settings.availability_window_minutes, windows),
      by_day: byDay(mine),
      by_workflow: byWorkflow(mine),
      client: id,
      cost_class: client.cost_class,
      plan_limits: windows,
      quota: quotaOf(id, reading.status),
      reserved_runs: reservations.length,
      reserved_usd: round(
        reservations.reduce((total, reservation) => total + reservation.estimate_usd, 0),
        6,
      ),
      usage: totalsOf(mine),
      warnings: warningsOf(id, windows),
    }
  })
  return {
    clients,
    days: input.days,
    status: 'ok',
    ...(input.workflow === undefined ? {} : { workflow: input.workflow }),
  }
}

/**
 * Each window of a client's plan, with the spend the ledger holds in it. A
 * window reaches back from now, so a plan that allows $12 per 5 hours is
 * watched over the last five hours whatever the calendar says.
 */
export function planWindows(
  client: string,
  limits: readonly PlanLimit[],
  records: readonly LedgerRecord[],
  now: Date,
  warningRatio: number,
): readonly PlanWindowUsage[] {
  return limits.map((limit) => {
    const since = now.getTime() - WINDOW_MS[limit.window]
    const mine = records.filter((record) => record.client === client && Date.parse(record.at) >= since)
    if ('limit_usd' in limit) {
      const spent = round(mine.reduce((sum, record) => sum + record.cost_usd, 0), 6)
      const ratio = round(spent / limit.limit_usd, 4)
      return { limit_usd: limit.limit_usd, ratio, spent_usd: spent, state: stateOf(ratio, warningRatio), window: limit.window }
    }
    const ratio = round(mine.length / limit.limit_runs, 4)
    return { limit_runs: limit.limit_runs, ratio, runs: mine.length, state: stateOf(ratio, warningRatio), window: limit.window }
  })
}

/** One line per plan window at or above the warning ratio, for `usage` and for every run's result. */
export function warningsOf(client: string, windows: readonly PlanWindowUsage[]): readonly string[] {
  return windows
    .filter((window) => window.state !== 'ok')
    .map((window) => `${client} has used ${String(Math.round(window.ratio * 100))}% of ${describeWindow(window)}`)
}

/** A plan window as a sentence names it: its span, and what is spent of what it allows. */
export function describeWindow(window: PlanWindowUsage): string {
  return 'limit_usd' in window
    ? `its ${window.window} plan limit ($${window.spent_usd.toFixed(2)} of $${window.limit_usd.toFixed(2)})`
    : `its ${window.window} plan limit (${String(window.runs)} of ${String(window.limit_runs)} runs)`
}

function stateOf(ratio: number, warningRatio: number): PlanState {
  if (ratio >= 1) return 'exhausted'
  return ratio >= warningRatio ? 'warning' : 'ok'
}

/** One line naming what a run's own answer said: the refusal it reported, and the reset it named when it named one. */
export function refusalReason(record: LedgerRecord): string {
  const said = `${record.reason ?? 'failed'} at ${record.at}`
  return record.unavailable_until === undefined ? said : `${said}: unavailable until ${record.unavailable_until}`
}

/**
 * The client models whose own answers said they cannot run: a quota answer
 * with the reset it named on the run's line, and the answer for a model the
 * account cannot use, which names no moment. An answer stands until the
 * moment it named, and one that named none stands until a later run of the
 * client model succeeded; one whose moment has passed stands no more.
 */
export function standingRefusals(records: readonly LedgerRecord[], now: Date): readonly StandingRefusal[] {
  const standing = new Map<string, LedgerRecord>()
  for (const record of [...records].sort((left, right) => Date.parse(left.at) - Date.parse(right.at))) {
    const key = `${record.client}:${record.model}`
    if (record.outcome === 'ok') standing.delete(key)
    else if (record.unavailable_until !== undefined || record.reason === 'model-unavailable')
      standing.set(key, record)
  }
  return [...standing]
    .filter(
      ([, record]) =>
        record.unavailable_until === undefined ||
        Date.parse(record.unavailable_until) > now.getTime(),
    )
    .map(([, record]) => ({
      client: record.client,
      model: record.model,
      reason: refusalReason(record),
    }))
}

/**
 * Whether a client can run now. A client that is not installed, one whose own
 * status command says it is signed out, and one whose last run in the
 * availability window was rate-limited or refused for its credentials are all
 * unavailable; a client model whose own answer said it cannot run — its
 * account's quota, which named when it resets, or a model the account cannot
 * use — makes the client unavailable until that answer no longer stands; and
 * a plan window at 100% makes it unable to start. A status command the
 * gateway cannot run leaves the reading `unknown`, which does not stop a run:
 * silence is not a refusal.
 */
function availabilityOf(
  client: string,
  binary: string,
  reading: ClientReading,
  records: readonly LedgerRecord[],
  now: Date,
  windowMinutes: number,
  windows: readonly PlanWindowUsage[],
): Availability {
  if (!reading.installed) {
    return { installed: false, reason: `not installed: ${binary} is not on PATH`, signed_in: 'unknown', status: 'unavailable' }
  }
  const signedIn = reading.status?.signed_in ?? 'unknown'
  const reason =
    reading.status === undefined
      ? `${client} has no status command, so the gateway cannot tell whether it is signed in`
      : `${reading.status.command}: ${reading.status.detail}`
  if (signedIn === 'no') return { installed: true, reason, signed_in: signedIn, status: 'unavailable' }
  const exhausted = windows.find((window) => window.state === 'exhausted')
  if (exhausted !== undefined) {
    return { installed: true, reason: `at 100% of ${describeWindow(exhausted)}`, signed_in: signedIn, status: 'unable_to_start' }
  }
  const mine = records.filter((record) => record.client === client)
  const standing = standingRefusals(mine, now)
  if (standing.length > 0) {
    const latest = standing[standing.length - 1]
    return { installed: true, reason: latest.reason, signed_in: signedIn, status: 'unavailable' }
  }
  const error = recentError(mine, now, windowMinutes)
  if (error !== undefined) {
    return {
      installed: true,
      reason: `${error.reason ?? 'failed'} at ${error.at}, with no later run of it that succeeded`,
      signed_in: signedIn,
      status: 'unavailable',
    }
  }
  return { installed: true, reason, signed_in: signedIn, status: 'available' }
}

/**
 * The last rate-limit or auth error a client's runs left in the availability
 * window, unless a later run of it succeeded: one success clears the errors
 * before it, and an error after it stands.
 */
function recentError(records: readonly LedgerRecord[], now: Date, windowMinutes: number): LedgerRecord | undefined {
  const since = now.getTime() - windowMinutes * 60_000
  let error: LedgerRecord | undefined
  for (const record of [...records].sort((left, right) => Date.parse(left.at) - Date.parse(right.at))) {
    if (Date.parse(record.at) < since) continue
    if (record.outcome === 'ok') error = undefined
    else if (record.reason === 'rate-limited' || record.reason === 'auth') error = record
  }
  return error
}

/** What a client itself says is left of its plan. No client's status command reports one, so it is `unknown`. */
function quotaOf(client: string, status: ClientStatusReading | undefined): Quota {
  return {
    reason:
      status === undefined
        ? `${client} has no status command, so the gateway cannot read its quota`
        : `${status.command} reports no quota`,
    status: 'unknown',
  }
}

/**
 * What a set of runs came to. A run that reported no token count leaves the
 * total unknown rather than short: the gateway never reports a reading it
 * could not get as zero, so the tokens are left out and the run still counts.
 */
function totalsOf(records: readonly LedgerRecord[]): UsageTotals {
  const tokens = { cached: 0, input: 0, output: 0 }
  let cost = 0
  let known = true
  for (const record of records) {
    cost += record.cost_usd
    if (record.tokens === undefined) {
      known = false
      continue
    }
    tokens.cached += record.tokens.cached
    tokens.input += record.tokens.input
    tokens.output += record.tokens.output
  }
  return { cost_usd: round(cost, 6), runs: records.length, ...(known ? { tokens } : {}) }
}

function byDay(records: readonly LedgerRecord[]): readonly DayUsage[] {
  const days = new Map<string, LedgerRecord[]>()
  for (const record of records) {
    const day = record.at.slice(0, 10)
    days.set(day, [...(days.get(day) ?? []), record])
  }
  return [...days]
    .map(([day, mine]) => ({ day, ...totalsOf(mine) }))
    .sort((left, right) => (left.day < right.day ? -1 : left.day > right.day ? 1 : 0))
}

function byWorkflow(records: readonly LedgerRecord[]): readonly WorkflowUsage[] {
  const workflows = new Map<string, LedgerRecord[]>()
  for (const record of records) {
    workflows.set(record.workflow, [...(workflows.get(record.workflow) ?? []), record])
  }
  return [...workflows]
    .map(([workflow, mine]) => ({ workflow, ...totalsOf(mine) }))
    .sort((left, right) => (left.workflow < right.workflow ? -1 : left.workflow > right.workflow ? 1 : 0))
}

/** The UTC midnight that starts the day `now` falls in. */
function startOfDay(now: Date): number {
  return Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate())
}

/** A sum of dollars or a ratio, rounded so the JSON does not carry float noise. */
function round(value: number, places: number): number {
  const factor = 10 ** places
  return Math.round(value * factor) / factor
}
