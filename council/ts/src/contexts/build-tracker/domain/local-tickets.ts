import type { SpecTicket } from './seed.js'
import { BuildTrackerError, type ExternalBlocker } from './state.js'

/** A local ticket file: its path under the repository root and its text. */
export interface LocalTicketFile {
  readonly file: string
  readonly text: string
}

const CLOSED_STATUSES: readonly string[] = ['closed', 'done', 'resolved', 'wontfix', 'merged']

/**
 * Reads local ticket files. A ticket's number is the number its file name
 * starts with (`01-store.md` is ticket 1); a file without one is refused.
 * Its "Blocked by" names other tickets by number (`01`, `#1`, `01-store.md`)
 * or by title; a name that matches no ticket is a blocker outside the spec,
 * kept as written. A `Status:` of done, closed, resolved, merged or wontfix
 * closes it.
 */
export function localTickets(files: readonly LocalTicketFile[]): readonly SpecTicket[] {
  const parsed = files.map((file) => {
    const stem = (file.file.split('/').at(-1) ?? '').replace(/\.md$/i, '')
    const prefix = /^\d+/.exec(stem)?.[0]
    if (prefix === undefined || Number(prefix) === 0) {
      throw new BuildTrackerError(
        'invalid-ticket-file',
        `${file.file} has no ticket number in front of its name; name each ticket file like 01-${stem}.md`,
      )
    }
    const heading = firstHeading(file.text)
    const title = heading?.replace(/^#?\d+\s*[:.\-–]\s*/, '') ?? stem
    return { blockers: blockerNames(file.text), closed: isClosed(file.text), file: file.file, number: Number(prefix), title }
  })
  const seen = new Map<number, string>()
  for (const ticket of parsed) {
    const other = seen.get(ticket.number)
    if (other !== undefined) {
      throw new BuildTrackerError('invalid-ticket-file', `${other} and ${ticket.file} both have ticket number ${String(ticket.number)}`)
    }
    seen.set(ticket.number, ticket.file)
  }
  const resolveName = (name: string): number | undefined => {
    const number = /^#?(\d+)(?=$|[\s\-:.(])/.exec(name)?.[1]
    const match =
      number === undefined
        ? parsed.find((ticket) => ticket.title.toLowerCase() === name.toLowerCase())
        : parsed.find((ticket) => ticket.number === Number(number))
    return match?.number
  }
  return parsed
    .sort((left, right) => left.number - right.number)
    .map(({ blockers, ...ticket }) => {
      const blockedBy: number[] = []
      const external: ExternalBlocker[] = []
      for (const name of blockers) {
        const number = resolveName(name)
        if (number === undefined) external.push({ ref: name })
        else if (!blockedBy.includes(number)) blockedBy.push(number)
      }
      return { ...ticket, blockedBy, external }
    })
}

/** A local spec's title: its first `#` heading, or its file name. */
export function localSpecTitle(text: string, path: string): string {
  return firstHeading(text) ?? (path.split('/').at(-1) ?? path)
}

const HEADING = /^#[ \t]+(.+?)[ \t]*$/m

/** The text of the file's first top-level `#` heading. */
function firstHeading(text: string): string | undefined {
  return HEADING.exec(text)?.[1]
}

function isClosed(text: string): boolean {
  const status = /^[ \t>]*\**Status\**[ \t]*:\**[ \t]*(.*)$/im.exec(text)?.[1]
  const value = status?.replace(/[*_`]/g, '').trim().toLowerCase()
  return value !== undefined && CLOSED_STATUSES.includes(value)
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
