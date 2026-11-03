import { specText, type ResolvedSpec } from './spec.js'
import { BuildTrackerError, isMerged, STATE_VERSION, type ExternalBlocker, type JsonObject, type Milestone } from './state.js'

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

/** A spec's title and tickets as its issue tracker holds them now, read once. */
export interface IssueTrackerSnapshot {
  readonly title: string
  readonly tickets: readonly SpecTicket[]
}

const MILESTONE = /^(.*\S)\s*=\s*#?([1-9]\d*)$/s

/** Reads each `<name>=<ticket>` argument of `init` in order; every ticket must be one of the spec's. */
export function parseMilestones(texts: readonly string[], snapshot: IssueTrackerSnapshot): readonly Milestone[] {
  const known = new Set(snapshot.tickets.map((ticket) => ticket.number))
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
export function seedState(spec: ResolvedSpec, snapshot: IssueTrackerSnapshot, milestones: readonly Milestone[]): JsonObject {
  const tickets: JsonObject = {}
  for (const ticket of snapshot.tickets) tickets[String(ticket.number)] = issueTrackerFields({ status: ticket.closed ? 'merged' : 'todo' }, ticket)
  return {
    ...(milestones.length > 0 ? { milestones: milestones.map((milestone) => ({ closes: milestone.closes, name: milestone.name })) } : {}),
    spec: spec.kind === 'github' ? specText(spec) : { path: spec.path },
    tickets,
    title: snapshot.title,
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
 * The state brought up to date with the issue tracker. New tickets are
 * added; a known ticket gets its title, blockers and file from the issue
 * tracker, and turns `merged` once its issue is closed. Every status, note
 * and other key the agent set on an open ticket stays, and so do the
 * milestones, the title and tickets the issue tracker no longer lists.
 */
export function syncState(raw: JsonObject, snapshot: IssueTrackerSnapshot): SyncOutcome {
  const tickets: JsonObject = { ...(raw.tickets as JsonObject) }
  const added: number[] = []
  const merged: number[] = []
  for (const ticket of snapshot.tickets) {
    const key = String(ticket.number)
    const known = tickets[key] as JsonObject | undefined
    if (known === undefined) added.push(ticket.number)
    else if (ticket.closed && !isMerged(known)) merged.push(ticket.number)
    const status = ticket.closed ? 'merged' : (known?.status ?? 'todo')
    tickets[key] = issueTrackerFields({ ...known, status }, ticket)
  }
  return { added, merged, raw: { ...raw, tickets } }
}

/** The entry with the fields the issue tracker owns replaced; an empty external list or a missing file leaves no key. */
function issueTrackerFields(entry: JsonObject, ticket: SpecTicket): JsonObject {
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
