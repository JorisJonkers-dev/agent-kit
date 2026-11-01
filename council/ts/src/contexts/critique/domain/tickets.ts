/**
 * A spec's tickets, and what a script can judge about them: dependency
 * cycles, blockers that name no ticket of the spec, acceptance-criterion
 * counts and user-story coverage. No model is asked any of it.
 */

/** One ticket of a spec, as its tracker holds it. */
export interface Ticket {
  /** `#184` for a GitHub issue of the spec's repository, the `NN` of its file name for a local ticket. */
  readonly id: string
  /** Where it lives: `host/owner/repo#<number>` for an issue, its repository path for a file. */
  readonly reference: string
  readonly title: string
  readonly text: string
  /**
   * The ids of the tickets that must be done first. A blocker that names no
   * ticket of the spec keeps the form it was written in, so it shows as
   * dangling.
   */
  readonly blockedBy: readonly string[]
}

/**
 * Every elementary dependency cycle, each as ticket ids where each ticket is
 * blocked by the next and the last by the first. A cycle starts at its
 * earliest ticket in tracker order, and cycles come in the order of their
 * first tickets. A ticket that blocks itself is a cycle of one. Johnson's
 * circuit search, so a dense knot of tickets is still finite work per cycle.
 */
export function dependencyCycles(tickets: readonly Ticket[]): readonly (readonly string[])[] {
  const index = new Map(tickets.map((ticket, i) => [ticket.id, i]))
  const edges = tickets.map((ticket) => [
    ...new Set(ticket.blockedBy.flatMap((blocker) => (index.has(blocker) ? [index.get(blocker) ?? 0] : []))),
  ])
  const cycles: string[][] = []
  for (let start = 0; start < tickets.length; start += 1) {
    const blocked = new Set<number>()
    const blockedBy = new Map<number, Set<number>>()
    const path: number[] = []
    const unblock = (node: number): void => {
      blocked.delete(node)
      const waiting = blockedBy.get(node) ?? new Set<number>()
      blockedBy.delete(node)
      for (const other of waiting) if (blocked.has(other)) unblock(other)
    }
    const circuit = (node: number): boolean => {
      let found = false
      path.push(node)
      blocked.add(node)
      const next = (edges[node] ?? []).filter((other) => other >= start)
      for (const other of next) {
        if (other === start) {
          cycles.push(path.map((i) => tickets[i]?.id ?? ''))
          found = true
        } else if (!blocked.has(other) && circuit(other)) {
          found = true
        }
      }
      if (found) unblock(node)
      else for (const other of next) blockedBy.set(other, (blockedBy.get(other) ?? new Set<number>()).add(node))
      path.pop()
      return found
    }
    circuit(start)
  }
  return cycles
}

/** One ticket file as `/to-tickets` writes it locally: `.scratch/<feature>/issues/<NN>-<slug>.md`. */
export interface LocalTicketFile {
  /** Its repository path. */
  readonly reference: string
  readonly text: string
}

/**
 * Reads local ticket files. A ticket's id is the number its file name starts
 * with, as written (`01`), or the file name without `.md` when there is none.
 * Its "Blocked by" names other tickets by number (`01`, `#1`, `01-schema.md`)
 * or by title; a name that matches no ticket is kept as written.
 */
export function localTickets(files: readonly LocalTicketFile[]): readonly Ticket[] {
  const parsed = files.map((file) => {
    const stem = (file.reference.split('/').at(-1) ?? '').replace(/\.md$/i, '')
    const id = /^\d+/.exec(stem)?.[0] ?? stem
    const heading = /^#[ \t]+(.+?)[ \t]*$/m.exec(file.text)?.[1]
    const title = heading?.replace(/^#?\d+\s*[:.\-–]\s*/, '') ?? stem
    return { blockers: blockerNames(file.text), id, reference: file.reference, text: file.text, title }
  })
  const resolve = (name: string): string => {
    const number = /^#?(\d+)(?=$|[\s\-:.(])/.exec(name)?.[1]
    const match =
      number === undefined
        ? parsed.find((ticket) => ticket.title.toLowerCase() === name.toLowerCase())
        : parsed.find((ticket) => /^\d+$/.test(ticket.id) && Number(ticket.id) === Number(number))
    return match?.id ?? name
  }
  return parsed.map(({ blockers, ...ticket }) => ({ ...ticket, blockedBy: blockers.map(resolve) }))
}

const NO_BLOCKER = /^(none|n\/a|nothing|-+)\b/i
const EMPHASIS = /[*_`"']+/g

/**
 * The names a ticket's "Blocked by" lists: on the line itself
 * (`**Blocked by:** 01, 02`), or as list items under a `Blocked by` line or
 * heading. "None (can start immediately)" lists nothing.
 */
function blockerNames(text: string): readonly string[] {
  const lines = text.split(/\r?\n/)
  const at = lines.findIndex((line) => /^[\s>*_#-]*blocked by\b/i.test(line))
  if (at === -1) return []
  const rest = (lines[at] ?? '').replace(/^[\s>*_#-]*blocked by\b[\s*_:]*/i, '')
  const items: string[] = []
  if (rest.trim().length > 0) items.push(rest)
  else {
    for (const line of lines.slice(at + 1)) {
      const item = /^\s*(?:[-*+]|\d+[.)])\s+(.*)$/.exec(line)
      if (item !== null) items.push(item[1] ?? '')
      else if (line.trim().length > 0 || items.length > 0) break
    }
  }
  return items
    .flatMap((item) => item.split(/[,;]/))
    .flatMap((item) => {
      const name = item.replace(EMPHASIS, '').trim()
      if (name.length === 0 || NO_BLOCKER.test(name)) return []
      return /^(?:#?\d+\s*(?:and|&|\s)\s*)*#?\d+$/i.test(name) ? (name.match(/\d+/g) ?? []) : [name]
    })
}

/** What the checker reports about one ticket. */
export interface TicketReport {
  readonly id: string
  readonly reference: string
  readonly title: string
  /** Its acceptance criteria: the task-list items under its "Acceptance criteria" heading, or anywhere before its comments when it has none. */
  readonly criteria: number
  readonly blocked_by: readonly string[]
  /** The user story numbers it cites. */
  readonly stories: readonly number[]
}

export interface DanglingBlocker {
  readonly ticket: string
  /** The blocker as the ticket names it, which is no ticket of the spec. */
  readonly blocker: string
}

/**
 * Which spec user stories no ticket cites. Coverage is only checked when
 * some ticket cites a story; otherwise nothing shows whether the tickets
 * cover the spec, and it is not checkable rather than covered.
 */
export type StoryCoverage =
  | { readonly status: 'checked'; readonly stories: readonly number[]; readonly uncited: readonly number[] }
  | { readonly status: 'not-checkable'; readonly reason: 'no ticket cites a user story' | 'the spec has no numbered user stories' }

export interface TicketCheck {
  readonly tickets: readonly TicketReport[]
  readonly cycles: readonly (readonly string[])[]
  readonly dangling_blockers: readonly DanglingBlocker[]
  readonly coverage: StoryCoverage
}

/** Checks a spec's tickets against each other and against the spec. */
export function checkTickets(spec: string, tickets: readonly Ticket[]): TicketCheck {
  const ids = new Set(tickets.map((ticket) => ticket.id))
  const reports = tickets.map(
    (ticket): TicketReport => ({
      blocked_by: ticket.blockedBy,
      criteria: criteriaCount(ticket.text),
      id: ticket.id,
      reference: ticket.reference,
      stories: citedStories(ticket.text),
      title: ticket.title,
    }),
  )
  return {
    coverage: storyCoverage(userStories(spec), reports),
    cycles: dependencyCycles(tickets),
    dangling_blockers: tickets.flatMap((ticket) =>
      ticket.blockedBy.filter((blocker) => !ids.has(blocker)).map((blocker) => ({ blocker, ticket: ticket.id })),
    ),
    tickets: reports,
  }
}

function storyCoverage(stories: readonly number[], reports: readonly TicketReport[]): StoryCoverage {
  if (stories.length === 0) return { reason: 'the spec has no numbered user stories', status: 'not-checkable' }
  const cited = new Set(reports.flatMap((report) => report.stories))
  if (cited.size === 0) return { reason: 'no ticket cites a user story', status: 'not-checkable' }
  return { status: 'checked', stories, uncited: stories.filter((story) => !cited.has(story)) }
}

/** The numbers of the top-level numbered items under the spec's "User Stories" heading, subheadings included. */
export function userStories(spec: string): readonly number[] {
  const lines = section(prose(spec), /^user stories$/i) ?? []
  const numbers = lines.flatMap((line) => {
    const item = /^(\d+)[.)]\s/.exec(line)?.[1]
    return item === undefined ? [] : [Number(item)]
  })
  return [...new Set(numbers)]
}

const TASK_ITEM = /^\s*[-*+]\s+\[[ xX]\]\s/

function criteriaCount(text: string): number {
  const lines = prose(text)
  const scope = section(lines, /^acceptance criteria$/i) ?? beforeComments(lines)
  return scope.filter((line) => TASK_ITEM.test(line)).length
}

const STORY_NUMBER = String.raw`#?\d+(?:\s*[-–]\s*#?\d+)?`
const CITATION = new RegExp(
  String.raw`\buser\s+stor(?:y|ies)\b[\s:*_]*(${STORY_NUMBER}(?:\s*(?:,\s*(?:and\s+|or\s+)?|and\s+|or\s+|&\s*)${STORY_NUMBER})*)`,
  'gi',
)
/** The longest range a citation such as "user stories 3–9" is expanded over. */
const MAX_RANGE = 1000

/**
 * The user story numbers a ticket cites: "user story 3", "user stories 3, 5
 * and 7", "User stories: 3–5" or "user story #3", outside code blocks.
 */
function citedStories(text: string): readonly number[] {
  const cited = new Set<number>()
  for (const match of prose(text).join('\n').matchAll(CITATION)) {
    for (const part of (match[1] ?? '').split(/,|\band\b|\bor\b|&/)) {
      const [low, high = low] = (part.match(/\d+/g) ?? []).map(Number)
      if (low === undefined || high === undefined) continue
      const span = high >= low && high - low <= MAX_RANGE ? high - low : 0
      for (let story = low; story <= low + span; story += 1) cited.add(story)
      cited.add(high)
    }
  }
  return [...cited].sort((a, b) => a - b)
}

/** The lines of a markdown text outside fenced code blocks. */
function prose(text: string): readonly string[] {
  let fenced = false
  return text.split(/\r?\n/).filter((line) => {
    if (/^\s*(?:```|~~~)/.test(line)) {
      fenced = !fenced
      return false
    }
    return !fenced
  })
}

function headingOf(line: string): { readonly level: number; readonly title: string } | undefined {
  const heading = /^(#{1,6})[ \t]+(.*?)[ \t#]*$/.exec(line)
  return heading === null ? undefined : { level: (heading[1] ?? '').length, title: heading[2] ?? '' }
}

/** The lines under the first heading whose title matches, up to the next heading of its level or above. */
function section(lines: readonly string[], title: RegExp): readonly string[] | undefined {
  const start = lines.findIndex((line) => title.test(headingOf(line)?.title ?? ''))
  if (start === -1) return undefined
  const level = headingOf(lines[start] ?? '')?.level ?? 1
  const rest = lines.slice(start + 1)
  const end = rest.findIndex((line) => (headingOf(line)?.level ?? Infinity) <= level)
  return end === -1 ? rest : rest.slice(0, end)
}

/** The lines before a ticket's `## Comments`, where conversation is appended. */
function beforeComments(lines: readonly string[]): readonly string[] {
  const end = lines.findIndex((line) => /^comments$/i.test(headingOf(line)?.title ?? ''))
  return end === -1 ? lines : lines.slice(0, end)
}
