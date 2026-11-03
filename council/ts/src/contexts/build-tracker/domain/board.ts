import { isMerged, type BuildTrackerState, type Status, type Ticket } from './state.js'

/**
 * What the page shows for a ticket. A `todo` ticket is Ready once every
 * blocker is merged and no blocker outside the spec is known to be open, and
 * Blocked before that; every other status shows as itself. An outside
 * blocker whose state is unknown changes nothing.
 */
export type Shown = Exclude<Status, 'todo'> | 'ready' | 'blocked'

/** The label each shown status carries on the page, in the summary strip's order. */
export const SHOWN_LABELS: Readonly<Record<Shown, string>> = {
  merged: 'Merged',
  review: 'Merging',
  running: 'Building',
  ready: 'Ready',
  owner: 'Needs you',
  blocked: 'Blocked',
}

export const SHOWN_ORDER = Object.keys(SHOWN_LABELS) as readonly Shown[]

export interface BoardTicket extends Ticket {
  readonly shown: Shown
}

export interface BoardSection {
  readonly name: string
  readonly tickets: readonly BoardTicket[]
  readonly merged: number
}

/** A count per shown status, for the statuses at least one ticket shows. */
export type Counts = Readonly<Partial<Record<Shown, number>>>

export interface Board {
  readonly tickets: readonly BoardTicket[]
  readonly sections: readonly BoardSection[]
  readonly counts: Counts
}

export const OTHER_SECTION = 'Other'
export const SINGLE_SECTION = 'Tickets'

export function buildBoard(state: BuildTrackerState): Board {
  const merged = new Set(state.tickets.filter(isMerged).map((ticket) => ticket.number))
  const tickets = state.tickets.map((ticket): BoardTicket => ({ ...ticket, shown: shownStatus(ticket, merged) }))
  const counts: Partial<Record<Shown, number>> = {}
  for (const shown of SHOWN_ORDER) {
    const count = tickets.filter((ticket) => ticket.shown === shown).length
    if (count > 0) counts[shown] = count
  }
  return { counts, sections: sections(state, tickets), tickets }
}

function shownStatus(ticket: Ticket, merged: ReadonlySet<number>): Shown {
  if (ticket.status !== 'todo') return ticket.status
  const waiting = ticket.blockedBy.some((blocker) => !merged.has(blocker)) || ticket.external.some((blocker) => blocker.state === 'open')
  return waiting ? 'blocked' : 'ready'
}

/**
 * One section per milestone, in the state's order. A ticket sits in the
 * first milestone whose closing ticket depends on it, directly or through
 * other tickets; the rest go in Other. A state without milestones has one
 * section.
 */
function sections(state: BuildTrackerState, tickets: readonly BoardTicket[]): readonly BoardSection[] {
  if (state.milestones.length === 0) return [section(SINGLE_SECTION, tickets)]
  const byNumber = new Map(tickets.map((ticket) => [ticket.number, ticket]))
  const milestoneOf = new Map<number, number>()
  state.milestones.forEach((milestone, index) => {
    for (const number of [milestone.closes, ...dependencies(milestone.closes, byNumber)]) {
      if (!milestoneOf.has(number)) milestoneOf.set(number, index)
    }
  })
  const result = state.milestones.map((milestone, index) =>
    section(
      milestone.name,
      tickets.filter((ticket) => milestoneOf.get(ticket.number) === index),
    ),
  )
  const other = tickets.filter((ticket) => !milestoneOf.has(ticket.number))
  return other.length > 0 ? [...result, section(OTHER_SECTION, other)] : result
}

/** Every ticket `number` depends on, directly or through other tickets. */
function dependencies(number: number, byNumber: ReadonlyMap<number, BoardTicket>): ReadonlySet<number> {
  const seen = new Set<number>()
  const pending = [number]
  for (let next = pending.pop(); next !== undefined; next = pending.pop()) {
    for (const blocker of byNumber.get(next)?.blockedBy ?? []) {
      if (!seen.has(blocker)) {
        seen.add(blocker)
        pending.push(blocker)
      }
    }
  }
  return seen
}

function section(name: string, tickets: readonly BoardTicket[]): BoardSection {
  return { merged: tickets.filter(isMerged).length, name, tickets }
}
