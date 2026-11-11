import { parseIssueReference, type RepositoryName } from './spec.js'

/**
 * The build tracker's state file: the one source the page is rendered from.
 *
 * Version 1 holds the spec (an issue reference `owner/repo#number`, with the
 * host in front when it is not github.com, or `{ "path": "<file>" }` for a
 * local markdown spec under the repository root), a title and an optional
 * subtitle, optional milestones (a name and the ticket that closes it) and
 * one entry per ticket keyed by its issue number (a local ticket's file
 * number). A ticket may name blockers outside the spec in `external`, and a
 * local ticket its file in `file`. Keys this version does not name are kept
 * as they are, so a later version can add fields.
 */

export const STATE_VERSION = 1

/** A ticket's status in the state file. */
export const STATUSES = ['todo', 'running', 'review', 'merged', 'owner'] as const
export type Status = (typeof STATUSES)[number]

/** A blocker outside the spec: `owner/repo#number`, or a local ticket's name as written. */
export interface ExternalBlocker {
  readonly ref: string
  /** Its issue's state, when the issue tracker reported one. */
  readonly state?: 'open' | 'closed'
}

export interface Ticket {
  readonly number: number
  readonly title: string
  readonly blockedBy: readonly number[]
  readonly external: readonly ExternalBlocker[]
  /** A local ticket's file under the repository root; undefined for an issue. */
  readonly file: string | undefined
  readonly status: Status
  readonly note: string
  readonly live: boolean
}

export interface Milestone {
  readonly name: string
  readonly closes: number
}

/** The spec as the state names it: an issue, or a local spec's path under its repository root. */
export type SpecReference =
  | { readonly kind: 'github'; readonly repository: RepositoryName; readonly number: number }
  | { readonly kind: 'local'; readonly path: string }

export interface BuildTrackerState {
  readonly spec: SpecReference
  readonly title: string
  readonly subtitle: string
  readonly milestones: readonly Milestone[]
  /** Every ticket, in issue-number order. */
  readonly tickets: readonly Ticket[]
  /**
   * Sections the page shows in place of the milestones', each a name and its
   * tickets. Set only for a state built from a ticket file; a state file has
   * no such field.
   */
  readonly groups?: readonly TicketGroup[]
}

/** A named section of the page and the tickets in it, by issue number. */
export interface TicketGroup {
  readonly name: string
  readonly tickets: readonly number[]
}

export type BuildTrackerErrorCode =
  | 'missing-state'
  | 'unreadable-state'
  | 'invalid-assignment'
  | 'invalid-ticket'
  | 'unknown-ticket'
  | 'unknown-status'
  | 'state-exists'
  | 'invalid-spec'
  | 'invalid-milestone'
  | 'invalid-ticket-file'
  | 'invalid-tickets'
  | 'tracker-unreachable'
  | 'unsupported-ticket'
  | 'no-state-home'

export class BuildTrackerError extends Error {
  constructor(
    readonly code: BuildTrackerErrorCode,
    message: string,
  ) {
    super(message)
    this.name = 'BuildTrackerError'
  }
}

export type JsonObject = Record<string, unknown>

export function isStatus(value: unknown): value is Status {
  return typeof value === 'string' && (STATUSES as readonly string[]).includes(value)
}

/** Whether a ticket, or a raw ticket entry of the state file, is merged. */
export function isMerged(ticket: { readonly status?: unknown }): boolean {
  return ticket.status === 'merged'
}

const TICKET_KEY = /^[1-9][0-9]*$/

/** Reads parsed JSON as a version-1 state, or says what is wrong with it. */
export function parseState(raw: unknown): BuildTrackerState {
  if (!isObject(raw)) throw invalid('the state is not a JSON object')
  if (raw.version !== STATE_VERSION) throw invalid(`version must be ${String(STATE_VERSION)}, not ${raw.version === undefined ? 'missing' : JSON.stringify(raw.version)}`)
  const spec = parseSpec(raw.spec)
  const title = requireString(raw.title, 'title')
  const subtitle = raw.subtitle === undefined ? '' : requireString(raw.subtitle, 'subtitle')
  if (!isObject(raw.tickets)) throw invalid('tickets must be an object keyed by issue number')
  const tickets = Object.entries(raw.tickets).map(([key, value]) => parseTicket(key, value))
  const numbers = new Set(tickets.map((ticket) => ticket.number))
  for (const ticket of tickets) {
    const stray = ticket.blockedBy.find((blocker) => !numbers.has(blocker))
    if (stray !== undefined) throw invalid(`ticket ${String(ticket.number)} is blocked by ${String(stray)}, which is not in the state`)
  }
  const milestones = readMilestones(raw.milestones, numbers)
  return {
    milestones,
    spec,
    subtitle,
    tickets: tickets.sort((left, right) => left.number - right.number),
    title,
  }
}

function parseSpec(raw: unknown): SpecReference {
  if (isObject(raw) && typeof raw.path === 'string' && raw.path !== '') return { kind: 'local', path: raw.path }
  const issue = parseIssueReference(typeof raw === 'string' ? raw : '')
  if (issue === undefined) throw invalid('spec must be a reference like owner/repo#123, or { "path": "<file>" } for a local spec')
  return { kind: 'github', number: issue.number, repository: issue.repository }
}

function parseTicket(key: string, value: unknown): Ticket {
  const where = `ticket ${key}`
  if (!TICKET_KEY.test(key)) throw invalid(`${where}: a ticket key must be its issue number`)
  if (!isObject(value)) throw invalid(`${where} is not an object`)
  const blockedBy = value.blocked_by
  if (!Array.isArray(blockedBy) || !blockedBy.every(isIssueNumber)) throw invalid(`${where}: blocked_by must be a list of issue numbers`)
  if (!isStatus(value.status)) throw invalid(`${where}: status must be one of ${STATUSES.join(', ')}`)
  if (value.live !== undefined && typeof value.live !== 'boolean') throw invalid(`${where}: live must be true or false`)
  return {
    blockedBy,
    external: parseExternal(value.external, where),
    file: value.file === undefined ? undefined : requireString(value.file, `${where}: file`),
    live: value.live === true,
    note: value.note === undefined ? '' : requireString(value.note, `${where}: note`),
    number: Number(key),
    status: value.status,
    title: requireString(value.title, `${where}: title`),
  }
}

/** Each external blocker is `{ "ref": ..., "state"?: "open" | "closed" }`, or just its ref as a string. */
function parseExternal(raw: unknown, where: string): readonly ExternalBlocker[] {
  if (raw === undefined) return []
  const reason = `${where}: external must be a list of blocker references`
  if (!Array.isArray(raw)) throw invalid(reason)
  return raw.map((item: unknown): ExternalBlocker => {
    if (typeof item === 'string' && item !== '') return { ref: item }
    if (!isObject(item) || typeof item.ref !== 'string' || item.ref === '') throw invalid(reason)
    if (item.state === undefined) return { ref: item.ref }
    if (item.state !== 'open' && item.state !== 'closed') throw invalid(`${where}: an external blocker's state must be open or closed`)
    return { ref: item.ref, state: item.state }
  })
}

/** The state file's milestones; `parseMilestones` in `seed.ts` reads them from `init`'s arguments instead. */
function readMilestones(raw: unknown, numbers: ReadonlySet<number>): readonly Milestone[] {
  if (raw === undefined) return []
  if (!Array.isArray(raw)) throw invalid('milestones must be a list')
  return raw.map((item: unknown, index) => {
    const where = `milestone ${String(index + 1)}`
    if (!isObject(item)) throw invalid(`${where} is not an object`)
    const name = requireString(item.name, `${where}: name`)
    if (!isIssueNumber(item.closes) || !numbers.has(item.closes)) throw invalid(`${where}: closes must name a ticket in the state`)
    return { closes: item.closes, name }
  })
}

function requireString(value: unknown, field: string): string {
  if (typeof value !== 'string') throw invalid(`${field} must be a string`)
  return value
}

function isIssueNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value > 0
}

export function isObject(value: unknown): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function invalid(reason: string): BuildTrackerError {
  return new BuildTrackerError('unreadable-state', reason)
}
