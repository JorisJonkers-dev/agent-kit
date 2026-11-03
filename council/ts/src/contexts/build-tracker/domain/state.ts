/**
 * The build tracker's state file: the one source the page is rendered from.
 *
 * Version 1 holds the spec reference (`owner/repo#number`), a title and an
 * optional subtitle, optional milestones (a name and the ticket that closes
 * it) and one entry per ticket keyed by its issue number. Keys this version
 * does not name are kept as they are, so a later version can add fields.
 */

export const STATE_VERSION = 1

/** A ticket's status in the state file. */
export const STATUSES = ['todo', 'running', 'review', 'merged', 'owner'] as const
export type Status = (typeof STATUSES)[number]

export interface Ticket {
  readonly number: number
  readonly title: string
  readonly blockedBy: readonly number[]
  readonly status: Status
  readonly note: string
  readonly live: boolean
}

export interface Milestone {
  readonly name: string
  readonly closes: number
}

export interface SpecReference {
  readonly owner: string
  readonly repo: string
  readonly number: number
}

export interface TrackerState {
  readonly spec: SpecReference
  readonly title: string
  readonly subtitle: string
  readonly milestones: readonly Milestone[]
  /** Every ticket, in issue-number order. */
  readonly tickets: readonly Ticket[]
}

export type BuildTrackerErrorCode = 'missing-state' | 'unreadable-state' | 'invalid-assignment' | 'unknown-ticket' | 'unknown-status'

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

const SPEC_PATTERN = /^([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+)#([1-9][0-9]*)$/
const TICKET_KEY = /^[1-9][0-9]*$/

/** Reads parsed JSON as a version-1 state, or says what is wrong with it. */
export function parseState(raw: unknown): TrackerState {
  if (!isObject(raw)) throw invalid('the state is not a JSON object')
  if (raw.version !== STATE_VERSION) throw invalid(`version must be ${String(STATE_VERSION)}, not ${raw.version === undefined ? 'missing' : JSON.stringify(raw.version)}`)
  const spec = SPEC_PATTERN.exec(typeof raw.spec === 'string' ? raw.spec : '')
  if (spec === null) throw invalid('spec must be a reference like owner/repo#123')
  const title = requireString(raw.title, 'title')
  const subtitle = raw.subtitle === undefined ? '' : requireString(raw.subtitle, 'subtitle')
  if (!isObject(raw.tickets)) throw invalid('tickets must be an object keyed by issue number')
  const tickets = Object.entries(raw.tickets).map(([key, value]) => parseTicket(key, value))
  const numbers = new Set(tickets.map((ticket) => ticket.number))
  for (const ticket of tickets) {
    const stray = ticket.blockedBy.find((blocker) => !numbers.has(blocker))
    if (stray !== undefined) throw invalid(`ticket ${String(ticket.number)} is blocked by ${String(stray)}, which is not in the state`)
  }
  const milestones = parseMilestones(raw.milestones, numbers)
  return {
    milestones,
    spec: { number: Number(spec[3]), owner: String(spec[1]), repo: String(spec[2]) },
    subtitle,
    tickets: tickets.sort((left, right) => left.number - right.number),
    title,
  }
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
    live: value.live === true,
    note: value.note === undefined ? '' : requireString(value.note, `${where}: note`),
    number: Number(key),
    status: value.status,
    title: requireString(value.title, `${where}: title`),
  }
}

function parseMilestones(raw: unknown, numbers: ReadonlySet<number>): readonly Milestone[] {
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
