import { BuildTrackerError, isStatus, STATUSES, type JsonObject, type Status, type BuildTrackerState } from './state.js'

/** One `<ticket>=<status>[:note]` argument of `set`. */
export interface Assignment {
  readonly ticket: number
  readonly status: Status
  /** Absent keeps the ticket's note; an empty note removes it. */
  readonly note?: string
}

const ASSIGNMENT = /^#?([1-9][0-9]*)=([^:]*)(?::(.*))?$/s

/**
 * Reads every assignment against the state, so one bad ticket or status
 * refuses them all before anything is written.
 */
export function parseAssignments(texts: readonly string[], state: BuildTrackerState): readonly Assignment[] {
  const known = new Set(state.tickets.map((ticket) => ticket.number))
  return texts.map((text) => {
    const match = ASSIGNMENT.exec(text)
    if (match === null) throw new BuildTrackerError('invalid-assignment', `${text} is not <ticket>=<status>[:note]`)
    const ticket = Number(match[1])
    if (!known.has(ticket)) throw new BuildTrackerError('unknown-ticket', `ticket ${String(ticket)} is not in the state`)
    const status = match[2]
    if (!isStatus(status)) {
      throw new BuildTrackerError('unknown-status', `${String(status)} is not a status; use one of ${STATUSES.join(', ')}`)
    }
    const note = match[3]
    return note === undefined ? { status, ticket } : { note, status, ticket }
  })
}

const LANDED = /^#?0*([1-9][0-9]*)$/

/**
 * Each ticket named as landed (`12`, `#12`, or a local ticket's `01`) as a
 * `merged` assignment that keeps its note. Every name is read against the
 * state first, so one bad ticket refuses them all.
 */
export function landedAssignments(texts: readonly string[], state: BuildTrackerState): readonly Assignment[] {
  const known = new Set(state.tickets.map((ticket) => ticket.number))
  return texts.map((text) => {
    const match = LANDED.exec(text.trim())
    if (match === null) throw new BuildTrackerError('invalid-ticket', `${text} is not a ticket number`)
    const ticket = Number(match[1])
    if (!known.has(ticket)) throw new BuildTrackerError('unknown-ticket', `ticket ${String(ticket)} is not one of the spec's tickets`)
    return { status: 'merged', ticket }
  })
}

/** The raw state with each assignment applied in order, every other key kept as it was. */
export function applyAssignments(raw: JsonObject, assignments: readonly Assignment[]): JsonObject {
  const tickets: JsonObject = { ...(raw.tickets as JsonObject) }
  for (const assignment of assignments) {
    const key = String(assignment.ticket)
    const entry: JsonObject = { ...(tickets[key] as JsonObject), status: assignment.status }
    if (assignment.note === '') delete entry.note
    else if (assignment.note !== undefined) entry.note = assignment.note
    tickets[key] = entry
  }
  return { ...raw, tickets }
}
