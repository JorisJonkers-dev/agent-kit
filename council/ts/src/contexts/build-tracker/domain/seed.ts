import { specText, type ResolvedSpec } from './spec.js'
import { BuildTrackerError, STATE_VERSION, type ExternalBlocker, type JsonObject } from './state.js'

/** One ticket of a spec as its issue tracker holds it. */
export interface SpecTicket {
  readonly number: number
  readonly title: string
  readonly closed: boolean
  /** Its blockers among the spec's tickets. */
  readonly blockedBy: readonly number[]
  /** Its blockers outside the spec. */
  readonly external: readonly ExternalBlocker[]
  /** A local ticket's file, under the repository root. */
  readonly file?: string
}

/** A spec's title and tickets, read once from its issue tracker. */
export interface SpecRead {
  readonly title: string
  readonly tickets: readonly SpecTicket[]
}

export interface MilestoneArgument {
  readonly name: string
  readonly closes: number
}

const MILESTONE = /^(.*\S)\s*=\s*#?([1-9]\d*)$/s

/** Reads each `<name>=<ticket>` in order; every ticket must be one of the spec's. */
export function parseMilestones(texts: readonly string[], read: SpecRead): readonly MilestoneArgument[] {
  const known = new Set(read.tickets.map((ticket) => ticket.number))
  return texts.map((text) => {
    const match = MILESTONE.exec(text.trim())
    if (match === null) throw new BuildTrackerError('invalid-milestone', `${text} is not <name>=<ticket>`)
    const closes = Number(match[2])
    if (!known.has(closes)) {
      throw new BuildTrackerError('unknown-ticket', `milestone ${String(match[1])} closes ticket ${String(closes)}, which is not one of the spec's tickets`)
    }
    return { closes, name: String(match[1]) }
  })
}

/** A new state for the spec: every open ticket `todo`, every closed one `merged`. */
export function seedState(spec: ResolvedSpec, read: SpecRead, milestones: readonly MilestoneArgument[]): JsonObject {
  const tickets: JsonObject = {}
  for (const ticket of read.tickets) tickets[String(ticket.number)] = trackerFields({ status: ticket.closed ? 'merged' : 'todo' }, ticket)
  return {
    ...(milestones.length > 0 ? { milestones: milestones.map((milestone) => ({ closes: milestone.closes, name: milestone.name })) } : {}),
    spec: spec.kind === 'github' ? specText(spec) : { path: spec.path },
    tickets,
    title: read.title,
    version: STATE_VERSION,
  }
}

export interface SyncOutcome {
  readonly raw: JsonObject
  /** Tickets the state did not have. */
  readonly added: readonly number[]
  /** Tickets whose issue closed since the state last saw them open. */
  readonly merged: readonly number[]
}

/**
 * The state brought up to date with the tracker. New tickets are added; a
 * known ticket gets its title, blockers and file from the tracker, and turns
 * `merged` once its issue is closed. Every status, note and other key the
 * agent set on an open ticket stays, and so do the milestones, the title and
 * tickets the tracker no longer lists.
 */
export function syncState(raw: JsonObject, read: SpecRead): SyncOutcome {
  const tickets: JsonObject = { ...(raw.tickets as JsonObject) }
  const added: number[] = []
  const merged: number[] = []
  for (const ticket of read.tickets) {
    const key = String(ticket.number)
    const known = tickets[key] as JsonObject | undefined
    if (known === undefined) added.push(ticket.number)
    else if (ticket.closed && known.status !== 'merged') merged.push(ticket.number)
    const status = ticket.closed ? 'merged' : (known?.status ?? 'todo')
    tickets[key] = trackerFields({ ...known, status }, ticket)
  }
  return { added, merged, raw: { ...raw, tickets } }
}

/** The entry with the fields the tracker owns replaced; an empty external list or a missing file leaves no key. */
function trackerFields(entry: JsonObject, ticket: SpecTicket): JsonObject {
  // Title and blockers lead a new entry; a known entry keeps its key order.
  const result: JsonObject = { title: ticket.title, blocked_by: [], ...entry }
  result.title = ticket.title
  result.blocked_by = [...ticket.blockedBy]
  delete result.external
  delete result.file
  if (ticket.external.length > 0) result.external = ticket.external.map((blocker) => ({ ...blocker }))
  if (ticket.file !== undefined) result.file = ticket.file
  return result
}
