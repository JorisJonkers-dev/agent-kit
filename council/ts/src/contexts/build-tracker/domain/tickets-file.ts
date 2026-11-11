import { BuildTrackerError, type BuildTrackerState, type Ticket } from './state.js'
import { isYamlObject, parseYamlSubset, type YamlValue } from './yaml-subset.js'

/** One branch of the ticket file: a group of tickets, with the issue that tracks it. */
export interface TicketBranch {
  readonly id: string
  readonly title: string
  readonly issue: number
}

/** One ticket of the ticket file: its key, its published issue, its branch and the keys it waits on. */
export interface TicketEntry {
  readonly key: string
  readonly issue: number
  readonly branch: string
  readonly title: string
  readonly blockedBy: readonly string[]
}

/** The ticket file of a spec (`docs/specs/<name>/tickets.yaml`): the source its GitHub issues are generated from. */
export interface TicketsFile {
  readonly spec: number
  readonly branches: readonly TicketBranch[]
  readonly tickets: readonly TicketEntry[]
}

/** Reads the ticket file's text, or says what is wrong with it. */
export function parseTicketsFile(text: string): TicketsFile {
  const root = parseYamlSubset(text)
  if (!isYamlObject(root)) throw invalid('the file is not a mapping')
  const spec = positiveInteger(root.spec, 'spec')
  if (!isYamlObject(root.branches)) throw invalid('branches must be a mapping of branch ids')
  const branches = Object.entries(root.branches).map(([id, value]): TicketBranch => {
    if (!isYamlObject(value)) throw invalid(`branch ${id} is not a mapping`)
    return { id, issue: positiveInteger(value.issue, `branch ${id}: issue`), title: requiredText(value.title, `branch ${id}: title`) }
  })
  if (!Array.isArray(root.tickets)) throw invalid('tickets must be a list')
  const tickets = (root.tickets as readonly YamlValue[]).map((value, index): TicketEntry => {
    if (!isYamlObject(value)) throw invalid(`ticket ${String(index + 1)} is not a mapping`)
    const key = requiredText(value.key, `ticket ${String(index + 1)}: key`)
    const where = `ticket ${key}`
    if (!Array.isArray(value.blocked_by) || !(value.blocked_by as readonly YamlValue[]).every((item) => typeof item === 'string')) {
      throw invalid(`${where}: blocked_by must be a list of ticket keys`)
    }
    return {
      blockedBy: value.blocked_by as readonly string[],
      branch: requiredText(value.branch, `${where}: branch`),
      issue: positiveInteger(value.issue, `${where}: issue`),
      key,
      title: requiredText(value.title, `${where}: title`),
    }
  })
  if (tickets.length === 0) throw invalid('the file has no tickets')
  checkGraph(branches, tickets)
  return { branches, spec, tickets }
}

function checkGraph(branches: readonly TicketBranch[], tickets: readonly TicketEntry[]): void {
  const keys = new Set<string>()
  const issues = new Set<number>()
  for (const ticket of tickets) {
    if (keys.has(ticket.key)) throw invalid(`the key ${ticket.key} appears twice`)
    if (issues.has(ticket.issue)) throw invalid(`the issue #${String(ticket.issue)} appears twice`)
    keys.add(ticket.key)
    issues.add(ticket.issue)
  }
  const branchIds = new Set(branches.map((branch) => branch.id))
  for (const ticket of tickets) {
    if (!branchIds.has(ticket.branch)) throw invalid(`ticket ${ticket.key} is on branch ${ticket.branch}, which the file does not define`)
    const stray = ticket.blockedBy.find((blocker) => !keys.has(blocker))
    if (stray !== undefined) throw invalid(`ticket ${ticket.key} is blocked by ${stray}, which is not a ticket of the file`)
  }
}

/** The state of one issue on the issue tracker, by number. */
export type IssueStates = ReadonlyMap<number, 'open' | 'closed'>

/**
 * The state a ticket file and the issue tracker make: every ticket of every
 * branch, merged when its issue is closed and todo when it is open, grouped
 * by branch in the file's order.
 */
export function stateFromTickets(
  file: TicketsFile,
  repository: { readonly host: string; readonly owner: string; readonly repo: string },
  title: string,
  states: IssueStates,
): BuildTrackerState {
  const issueOf = new Map(file.tickets.map((ticket) => [ticket.key, ticket.issue]))
  const tickets = file.tickets.map(
    (entry): Ticket => ({
      blockedBy: entry.blockedBy.map((key) => issueOf.get(key) ?? 0),
      external: [],
      file: undefined,
      live: /\(live\)/i.test(entry.title),
      note: '',
      number: entry.issue,
      status: states.get(entry.issue) === 'closed' ? 'merged' : 'todo',
      title: `${entry.key} ${entry.title}`,
    }),
  )
  const groups = file.branches.map((branch) => ({
    name: branch.title,
    tickets: file.tickets.filter((entry) => entry.branch === branch.id).map((entry) => entry.issue),
  }))
  return {
    groups,
    milestones: [],
    spec: { kind: 'github', number: file.spec, repository },
    subtitle: `${String(tickets.length)} tickets in ${String(groups.length)} branches`,
    tickets: tickets.sort((left, right) => left.number - right.number),
    title,
  }
}

function positiveInteger(value: YamlValue | undefined, field: string): number {
  if (typeof value !== 'number' || !Number.isInteger(value) || value <= 0) throw invalid(`${field} must be an issue number`)
  return value
}

function requiredText(value: YamlValue | undefined, field: string): string {
  if (typeof value !== 'string' || value === '') throw invalid(`${field} must be text`)
  return value
}

function invalid(reason: string): BuildTrackerError {
  return new BuildTrackerError('invalid-tickets', `the ticket file is invalid: ${reason}`)
}
