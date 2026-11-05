import { fits, liveReservations, meteredCharges, round, usd } from './caps.js'
import { isObject, ModelGatewayError } from './config.js'
import type { LedgerEntry } from './ledger.js'
import { compactTimestamp } from './run.js'
import { WORKFLOW_TTL_MS, workflowEnded, type WorkflowRecord } from './workflows.js'

/**
 * The owner's permission for one named workflow to send delegations to a
 * metered-employer client, with a dollar cap.
 *
 * A grant is written by the agent after it asked the owner, and it records the
 * question and the answer so a grant written without asking stands out. It
 * ends when its workflow ends, after 24 hours, or when its cap is spent,
 * whichever comes first; the gateway never renews one and never writes one
 * itself. Before a metered-employer run starts, its estimate is reserved
 * against the grant's remaining cap, and the run's actual spend replaces the
 * reservation when it ends.
 */

/** The version the grants file is written as. */
export const GRANTS_VERSION = 1

/** How long a grant lives when its workflow outlives it. */
export const GRANT_TTL_MS = WORKFLOW_TTL_MS

/** One grant, as the machine's grants file holds it. */
export interface Grant {
  readonly id: string
  readonly workflow: string
  /** The metered-employer client the owner allowed, `codex` today. */
  readonly client: string
  readonly cap_usd: number
  /** The question the owner was asked. */
  readonly question: string
  /** What the owner answered. */
  readonly answer: string
  readonly written_at: string
  /** When it lapses: 24 hours after it was written, or when its workflow's own 24 hours run out first. */
  readonly expires_at: string
  /** When someone ended it, as an ISO timestamp; absent while it stands. */
  readonly ended_at?: string
}

/** What a change to the grants leaves behind: the grants to write, and the one it touched. */
export interface GrantChange {
  readonly grants: readonly Grant[]
  readonly grant: Grant
}

/** Why a grant no longer covers a run. */
export type GrantLapse = 'workflow-ended' | 'expired' | 'cap-spent'

/** One grant with what it holds for the runs that ask: its cap, what is spent and what is reserved. */
export interface GrantRoom {
  readonly grant: Grant
  readonly spent_usd: number
  readonly reserved_usd: number
  readonly remaining_usd: number
}

/** One grant as `grant list` reports it: what it allows, and what it has left. */
export interface GrantView extends Grant {
  readonly spent_usd: number
  readonly reserved_usd: number
  readonly remaining_usd: number
}

/** A grant that no longer covers a run, and why. */
export interface LapsedGrant extends GrantView {
  readonly reason: GrantLapse
}

/**
 * The grants a file holds. A file that is not JSON, of another version, or
 * whose entries are not grants is one error naming what is wrong, rather than
 * a surprise when a run asks whether a grant covers it.
 */
export function parseGrantsFile(text: string): readonly Grant[] {
  let raw: unknown
  try {
    raw = JSON.parse(text)
  } catch (error) {
    throw invalid(`not JSON: ${messageOf(error)}`)
  }
  if (!isObject(raw)) throw invalid('the grants file must be a JSON object')
  if (raw.version !== GRANTS_VERSION) throw invalid(`version must be ${String(GRANTS_VERSION)}`)
  if (!Array.isArray(raw.grants)) throw invalid('grants must be a list')
  const grants: readonly unknown[] = raw.grants
  return grants.map((value, index) => parseGrant(value, index))
}

function parseGrant(raw: unknown, index: number): Grant {
  const where = `grants[${String(index)}]`
  if (!isObject(raw)) throw invalid(`${where} must be an object`)
  const cap = raw.cap_usd
  if (typeof cap !== 'number' || !Number.isFinite(cap) || cap <= 0) throw invalid(`${where}.cap_usd must be a number above 0`)
  const grant: Grant = {
    answer: requireString(raw.answer, `${where}.answer`),
    cap_usd: cap,
    client: requireString(raw.client, `${where}.client`),
    expires_at: requireTimestamp(raw.expires_at, `${where}.expires_at`),
    id: requireString(raw.id, `${where}.id`),
    question: requireString(raw.question, `${where}.question`),
    workflow: requireString(raw.workflow, `${where}.workflow`),
    written_at: requireTimestamp(raw.written_at, `${where}.written_at`),
  }
  if (raw.ended_at === undefined) return grant
  return { ...grant, ended_at: requireTimestamp(raw.ended_at, `${where}.ended_at`) }
}

function requireString(value: unknown, where: string): string {
  if (typeof value !== 'string' || value === '') throw invalid(`${where} must be a non-empty string`)
  return value
}

const UTC_TIMESTAMP = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,3})?Z$/

/**
 * A full ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SS(.sss)Z`) that names a real
 * moment. Anything Date.parse would read in the machine's local time, or
 * guess at, is refused: a hand-edited file must not move a grant's lapse.
 */
function requireTimestamp(value: unknown, where: string): string {
  const fail = (): ModelGatewayError => invalid(`${where} must be an ISO-8601 UTC timestamp like 2026-11-04T10:52:15Z`)
  if (typeof value !== 'string') throw fail()
  const match = UTC_TIMESTAMP.exec(value)
  if (match === null) throw fail()
  const [year, month, day, hour, minute, second] = match.slice(1).map(Number) as [number, number, number, number, number, number]
  const moment = new Date(Date.UTC(year, month - 1, day, hour, minute, second))
  const real =
    moment.getUTCFullYear() === year &&
    moment.getUTCMonth() === month - 1 &&
    moment.getUTCDate() === day &&
    moment.getUTCHours() === hour &&
    moment.getUTCMinutes() === minute &&
    moment.getUTCSeconds() === second
  if (!real) throw fail()
  return value
}

function invalid(message: string): ModelGatewayError {
  return new ModelGatewayError('invalid-config', message)
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** A grant's id: the UTC moment it was written, and a short suffix that keeps two grants of one second apart. */
export function grantId(at: Date, suffix: string): string {
  return `grant-${compactTimestamp(at)}-${suffix}`
}

/**
 * A grant as the gateway writes it: the owner's answer, and the moment it
 * lapses. It lapses 24 hours after it was written, or when the workflow's own
 * 24 hours run out first, so a grant never outlives the work it was given for.
 */
export function newGrant(args: {
  readonly id: string
  readonly workflow: string
  readonly client: string
  readonly cap_usd: number
  readonly question: string
  readonly answer: string
  readonly written_at: Date
  readonly workflowRecord: WorkflowRecord | undefined
}): Grant {
  return {
    answer: args.answer,
    cap_usd: args.cap_usd,
    client: args.client,
    expires_at: expiryOf(args.written_at, args.workflowRecord),
    id: args.id,
    question: args.question,
    workflow: args.workflow,
    written_at: args.written_at.toISOString(),
  }
}

/**
 * When a grant written at `writtenAt` lapses: its own 24 hours, or its
 * workflow's, whichever ends first. A workflow that already ended, or whose 24
 * hours already ran out, would give a grant that is expired on arrival, so it
 * is refused instead.
 */
function expiryOf(writtenAt: Date, record: WorkflowRecord | undefined): string {
  if (record !== undefined && workflowEnded(record, writtenAt)) {
    throw new ModelGatewayError(
      'workflow-ended',
      `workflow ${record.name} ended at ${record.ended_at ?? new Date(Date.parse(record.started_at) + WORKFLOW_TTL_MS).toISOString()}: a grant for it would lapse on arrival, so start a new workflow`,
    )
  }
  const own = writtenAt.getTime() + GRANT_TTL_MS
  const workflow = record === undefined ? own : Date.parse(record.started_at) + WORKFLOW_TTL_MS
  return new Date(Math.min(own, workflow)).toISOString()
}

/**
 * What a grant holds right now: its cap, what its workflow has spent on
 * metered-employer runs since the grant was written, and what live
 * reservations of that workflow hold.
 */
export function grantRoom(entries: readonly LedgerEntry[], grant: Grant): GrantRoom {
  const since = Date.parse(grant.written_at)
  const spent = meteredCharges(entries, 'metered-employer')
    .filter(
      (charge) => charge.client === grant.client && charge.workflow === grant.workflow && Date.parse(charge.at) >= since,
    )
    .map((charge) => charge.usd)
  const reserved = liveReservations(entries)
    .filter(
      (reservation) =>
        reservation.cost_class === 'metered-employer' &&
        reservation.client === grant.client &&
        reservation.workflow === grant.workflow &&
        Date.parse(reservation.reserved_at) >= since,
    )
    .map((reservation) => reservation.estimate_usd)
  const spent_usd = round(sum(spent), 6)
  const reserved_usd = round(sum(reserved), 6)
  return { grant, remaining_usd: round(grant.cap_usd - spent_usd - reserved_usd, 6), reserved_usd, spent_usd }
}

/**
 * Why a grant no longer covers a run, or undefined while it does: its workflow
 * ended, its 24 hours ran out, or its cap is spent, in that order.
 */
export function grantLapse(
  grant: Grant,
  room: GrantRoom,
  record: WorkflowRecord | undefined,
  now: Date,
): GrantLapse | undefined {
  if (grant.ended_at !== undefined || (record !== undefined && workflowEnded(record, now))) return 'workflow-ended'
  if (now.getTime() >= Date.parse(grant.expires_at)) return 'expired'
  if (room.spent_usd >= grant.cap_usd) return 'cap-spent'
  return undefined
}

/**
 * The grant that covers a workflow now: the live one the owner wrote last for
 * it (the latest `written_at`, the later in the file when two share one), or undefined when every grant of it lapsed. A workflow this machine
 * never started has no record, and its grant stands until its own 24 hours
 * run out.
 */
export function liveGrant(
  grants: readonly Grant[],
  workflow: string,
  client: string,
  entries: readonly LedgerEntry[],
  record: WorkflowRecord | undefined,
  now: Date,
): Grant | undefined {
  let newest: Grant | undefined
  for (const grant of grants) {
    if (grant.workflow !== workflow || grant.client !== client) continue
    if (grantLapse(grant, grantRoom(entries, grant), record, now) !== undefined) continue
    if (newest === undefined || Date.parse(grant.written_at) >= Date.parse(newest.written_at)) newest = grant
  }
  return newest
}

/** Every grant, split into the ones that still cover a run and the ones that lapsed with why. */
export function grantViews(
  grants: readonly Grant[],
  entries: readonly LedgerEntry[],
  records: readonly WorkflowRecord[],
  now: Date,
): { readonly grants: readonly GrantView[]; readonly lapsed: readonly LapsedGrant[] } {
  const live: GrantView[] = []
  const lapsed: LapsedGrant[] = []
  for (const grant of grants) {
    const room = grantRoom(entries, grant)
    const view: GrantView = {
      ...grant,
      remaining_usd: room.remaining_usd,
      reserved_usd: room.reserved_usd,
      spent_usd: room.spent_usd,
    }
    const reason = grantLapse(grant, room, records.find((record) => record.name === grant.workflow), now)
    if (reason === undefined) live.push(view)
    else lapsed.push({ ...view, reason })
  }
  return { grants: live, lapsed }
}

/**
 * Why a run is refused for want of a grant: it names the workflow, says no
 * live grant covers it, and names the `grant write` call that would add one.
 */
export function noGrantMessage(workflow: string, client: string): string {
  const call = `{"workflow":${JSON.stringify(workflow)},"client":${JSON.stringify(client)},"cap_usd":5,"question":"<what you asked the owner>","answer":"<what the owner answered>"}`
  return `no live grant covers workflow ${workflow}, so ${client} may not run: ask the owner, then add one with model-gateway grant write --json ${shellQuoted(call)}`
}

/**
 * A value as one POSIX shell word: a single quote of its own ends the word,
 * escapes itself and reopens it, so no value can break out of the quotes.
 */
function shellQuoted(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`
}

/**
 * Why a run does not fit its grant, naming the grant, its cap, what is spent
 * and reserved, and the run's estimate; undefined when it fits.
 */
export function grantRefusal(
  entries: readonly LedgerEntry[],
  grant: Grant,
  estimateUsd: number,
  run: { readonly client: string; readonly model: string },
): string | undefined {
  const room = grantRoom(entries, grant)
  if (fits({ limit_usd: grant.cap_usd, reserved_usd: room.reserved_usd, spent_usd: room.spent_usd }, estimateUsd)) {
    return undefined
  }
  return `${run.client}:${run.model} does not fit grant ${grant.id} of ${usd(grant.cap_usd)} for workflow ${grant.workflow}: ${usd(room.spent_usd)} spent and ${usd(room.reserved_usd)} reserved, and its estimate is ${usd(estimateUsd)}`
}

function sum(values: readonly number[]): number {
  return values.reduce((total, value) => total + value, 0)
}
