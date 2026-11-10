import { posix } from 'node:path'

import { inForce, recordStatus, withoutFences } from './decision-docs.js'

/** Where an ADR review's packet holds the checker's report, settled before any critic reads the records. */
export const ADR_CHECK_PATH = 'check/adrs.json'

/** One record of the decision log, at its repository path. */
export interface AdrSource {
  readonly path: string
  readonly text: string
}

/** What the checker reads off one record. */
export interface AdrEntry {
  readonly path: string
  /** The number its file name starts with; null for a file such as a README with none. */
  readonly number: number | null
  readonly title: string
  /** The status the record states, lowercased; null when it states none, which counts as accepted. */
  readonly status: string | null
  /** False when its status takes it out of force, or a record in force names it on a Supersedes or Replaces line. */
  readonly in_force: boolean
}

export interface AdrNumbering {
  /** Numbers missing between the lowest and the highest the log uses. */
  readonly gaps: readonly number[]
  readonly duplicates: readonly { readonly number: number; readonly records: readonly string[] }[]
  /** Files whose name starts with no number; reported, never an error. */
  readonly unnumbered: readonly string[]
}

export interface AdrLink {
  readonly record: string
  /** The target as the record writes it: a link's path, or an ADR number such as `ADR-0004`. */
  readonly target: string
  /** `supersede` for a target on a Supersedes, Superseded by, Amends or Replaces line; `link` for any other. */
  readonly kind: 'supersede' | 'link'
}

export interface AdrCheck {
  readonly records: readonly AdrEntry[]
  readonly numbering: AdrNumbering
  readonly unknown_statuses: readonly { readonly record: string; readonly status: string }[]
  readonly unresolved_links: readonly AdrLink[]
  readonly cites_superseded: readonly { readonly record: string; readonly cites: string }[]
}

/** `failed` while the log holds any structural error; unnumbered files alone never fail it. */
export type AdrCheckStatus = 'passed' | 'failed'

/** The status values a record may state, as Nygard and MADR name them. */
const STATUSES: readonly string[] = ['proposed', 'accepted', 'rejected', 'deprecated', 'superseded']

/** The number a record's file name starts with: up to five digits, never the year of a date such as `2024-01-15` or `2024-1-5`. */
const RECORD_NUMBER = /^(?!\d{4}[-_]\d{1,2}[-_]\d{1,2}(?!\d))(\d{1,5})[-_ ]/

/** A line that links a record to the one it replaces or amends, or that replaces or amends it. */
const SUPERSEDE_LINE =
  /\b(?:supersedes|superseded[ \t]+by|amends|amended[ \t]+by|replaces|replaced[ \t]+by)\b/i

/** A line on which a record says it replaces the records it names. */
const REPLACING_LINE = /\b(?:supersedes|replaces)\b/i

/** An inline link's target, a reference definition's target (never a footnote's `[^1]:`), or an ADR number. */
const REFERENCE =
  /\]\(<?([^)\s>]+)>?(?:[ \t]+"[^"]*")?\)|^[ \t]*\[(?!\^)[^\]\n]+\]:[ \t]*<?([^\s>]+)>?|\bADR[- ]?(\d{1,5})\b/gim

/** A bare URL, such as `https://example.com/adr/ADR-0004.md`: nothing in it names a record of this log. */
const BARE_URL = /\b[a-z][a-z0-9+.-]*:\/\/\S+/gi

/** A target with a scheme, such as `https:` or `mailto:`, or only an anchor, names no repository file. */
const NOT_A_FILE = /^(?:[a-z][a-z0-9+.-]*:|#)/i

/** One target a record names, with what it points at. */
interface Reference extends AdrLink {
  /** The repository path a link names, or null when it climbs out of the repository; only for a link to a file. */
  readonly path?: string | null
  /** The number an ADR number names; only for an ADR number. */
  readonly number?: number
  /** True on a line where the record says it replaces what it names. */
  readonly replacing: boolean
}

/**
 * The repository paths the log's links name, so the caller can say which
 * exist before the log is checked.
 */
export function linkedPaths(sources: readonly AdrSource[]): readonly string[] {
  const paths = sources.flatMap((source) =>
    references(source).flatMap((reference) =>
      typeof reference.path === 'string' ? [reference.path] : [],
    ),
  )
  return [...new Set(paths)]
}

/**
 * Checks the decision log's structure: numbering, the status values records
 * state, whether each link and ADR number resolves, and whether a record in
 * force still cites one that is superseded. `existing` holds the linked
 * repository paths that exist.
 */
export function checkAdrLog(
  sources: readonly AdrSource[],
  existing: ReadonlySet<string>,
): AdrCheck {
  const stated = sources.map(entry)
  const named = sources.map((source) => references(source))
  const numbers = new Set(
    stated.flatMap((record) => (record.number === null ? [] : [record.number])),
  )
  const resolves = (reference: Reference): boolean =>
    reference.number === undefined
      ? typeof reference.path === 'string' && existing.has(reference.path)
      : numbers.has(reference.number)
  const recordsOf = (reference: Reference): readonly string[] =>
    stated
      .filter((record) =>
        reference.number === undefined
          ? record.path === reference.path
          : record.number === reference.number,
      )
      .map((record) => record.path)
  const replaced = new Set(
    stated.flatMap((record, n) =>
      record.in_force
        ? (named[n] ?? []).filter((reference) => reference.replacing).flatMap(recordsOf)
        : [],
    ),
  )
  const records = stated.map((record) =>
    replaced.has(record.path) ? { ...record, in_force: false } : record,
  )
  const superseded = new Set([
    ...records.filter((record) => record.status === 'superseded').map((record) => record.path),
    ...replaced,
  ])
  return {
    cites_superseded: records.flatMap((record, n) => {
      if (!record.in_force || record.number === null) return []
      const cited = (named[n] ?? [])
        .filter((reference) => reference.kind === 'link')
        .flatMap(recordsOf)
      return [...new Set(cited)]
        .filter((path) => superseded.has(path) && path !== record.path)
        .map((cites) => ({ cites, record: record.path }))
    }),
    numbering: numbering(records),
    records,
    unknown_statuses: records.flatMap((record) =>
      record.status === null || STATUSES.includes(record.status)
        ? []
        : [{ record: record.path, status: record.status }],
    ),
    unresolved_links: named.flatMap((found) =>
      found
        .filter((reference) => !resolves(reference))
        .map(({ kind, record, target }) => ({ kind, record, target })),
    ),
  }
}

export function adrCheckStatus(check: AdrCheck): AdrCheckStatus {
  const { numbering } = check
  const errors = [
    numbering.gaps,
    numbering.duplicates,
    check.unknown_statuses,
    check.unresolved_links,
    check.cites_superseded,
  ]
  return errors.some((list) => list.length > 0) ? 'failed' : 'passed'
}

function entry(source: AdrSource): AdrEntry {
  const number = RECORD_NUMBER.exec(posix.basename(source.path))?.[1]
  return {
    in_force: inForce(source.text),
    number: number === undefined ? null : Number(number),
    path: source.path,
    status: recordStatus(source.text) ?? null,
    title: /^#[ \t]+(.+?)[ \t]*$/m.exec(source.text)?.[1] ?? '',
  }
}

function numbering(records: readonly AdrEntry[]): AdrNumbering {
  const byNumber = new Map<number, string[]>()
  for (const record of records) {
    if (record.number !== null)
      byNumber.set(record.number, [...(byNumber.get(record.number) ?? []), record.path])
  }
  const used = [...byNumber.keys()].sort((a, b) => a - b)
  const gaps: number[] = []
  for (let n = used[0] ?? 0; n <= (used.at(-1) ?? -1); n += 1) if (!byNumber.has(n)) gaps.push(n)
  return {
    duplicates: used.flatMap((number) => {
      const paths = byNumber.get(number) ?? []
      return paths.length > 1 ? [{ number, records: [...paths].sort() }] : []
    }),
    gaps,
    unnumbered: records.filter((record) => record.number === null).map((record) => record.path),
  }
}

/** Every target a record names outside fenced blocks, in text order. */
function references(source: AdrSource): readonly Reference[] {
  return withoutFences(source.text)
    .split('\n')
    .flatMap((line) => {
      const kind = SUPERSEDE_LINE.test(line) ? 'supersede' : 'link'
      const replacing = REPLACING_LINE.test(line)
      return [...line.replace(BARE_URL, ' ').matchAll(REFERENCE)].flatMap((match): Reference[] => {
        const [whole, inline, definition, number] = match
        const record = source.path
        if (number !== undefined)
          return [{ kind, number: Number(number), record, replacing, target: whole }]
        const target = inline ?? definition ?? ''
        return NOT_A_FILE.test(target)
          ? []
          : [{ kind, path: repositoryPath(source.path, target), record, replacing, target }]
      })
    })
}

/** The repository path a link in a record names, without its anchor; null when it climbs out of the repository. */
function repositoryPath(record: string, target: string): string | null {
  const file = target.replace(/#.*$/, '')
  const path = file.startsWith('/')
    ? posix.normalize(file.slice(1))
    : posix.normalize(posix.join(posix.dirname(record), file))
  return path === '..' || path.startsWith('../') ? null : path
}
