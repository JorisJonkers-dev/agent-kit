import { parseDiff, sectionText, type DiffHunk, type DiffSection } from './diff-filter.js'
import { bytes, mergeRanges, type LineRange } from './tree-sample.js'
import { countChangedLines } from './triage.js'

/**
 * How a diff that does not fit a code packet's cap is sent: in windows by
 * file, by rule and never by a model. Files go in this order: the changed
 * source files, then the tests, then the docs, each group by changed lines,
 * most first; each file's hunks in their own order. A hunk goes in with its
 * file's header and its own `@@` line, so a line it shows keeps its number;
 * a hunk that does not fit is left out and listed by file and line, and a
 * smaller one after it may still go in.
 */

/** What a changed file is, in the order its hunks are offered to the cap. */
const FILE_KINDS = ['source', 'test', 'doc'] as const
type FileKind = (typeof FILE_KINDS)[number]

const TEST_PATH = /(^|\/)(tests?|__tests__|specs?)\/|[._-](test|spec)\.[^/]+$|(^|\/)test_[^/]+\.py$/
const DOC_PATH = /(^|\/)docs?\/|\.(md|mdx|markdown|rst|adoc|txt)$/i

export function fileKind(path: string): FileKind {
  if (TEST_PATH.test(path)) return 'test'
  return DOC_PATH.test(path) ? 'doc' : 'source'
}

/** The first line of a windowed diff. */
export const DIFF_WINDOW_NOTICE = 'The packet cap left hunks out of this diff; facts/INDEX.md lists each one by file and line.\n'

/** A file's hunks the cap left out of the diff. */
export interface DiffDrop {
  readonly path: string
  /** Its hunks that were left out; none for a file with no hunks, whose whole section was. */
  readonly hunks: readonly DiffHunk[]
}

export interface WindowedDiff {
  readonly diff: string
  readonly dropped: readonly DiffDrop[]
}

/**
 * The diff with only the hunks that fit in `budget` bytes, each costing its
 * bytes and, for its file's first, the file's header. The lines that stand
 * in for left-out files always go in, after the windowed files. With a
 * budget of 0 every file's hunks are left out: the most the facts index
 * can list.
 */
export function windowDiff(diff: string, budget: number): WindowedDiff {
  const sections = parseDiff(diff)
  const files = rankFiles(sections.filter((section): section is DiffSection & { readonly path: string } => section.path !== undefined))
  const standing = sections.filter((section) => section.path === undefined).map(sectionText)
  let remaining = budget
  const kept: string[] = []
  const dropped: DiffDrop[] = []
  for (const file of files) {
    const pieces = file.hunks.length === 0 ? [undefined] : file.hunks
    const sent: DiffHunk[] = []
    const left: DiffHunk[] = []
    for (const hunk of pieces) {
      const cost = bytes(hunk?.text ?? '') + (sent.length === 0 ? bytes(file.header) : 0)
      const fits = cost <= remaining
      if (fits) remaining -= cost
      if (hunk === undefined) {
        if (fits) kept.push(file.header)
        else dropped.push({ hunks: [], path: file.path })
        continue
      }
      ;(fits ? sent : left).push(hunk)
    }
    if (sent.length > 0) kept.push(file.header + sent.map((hunk) => hunk.text).join(''))
    if (left.length > 0) dropped.push({ hunks: left, path: file.path })
  }
  return { diff: [DIFF_WINDOW_NOTICE, ...kept, ...standing].join(''), dropped }
}

/** Source files, then tests, then docs; each by changed lines, most first, then by path. */
function rankFiles<File extends DiffSection & { readonly path: string }>(files: readonly File[]): readonly File[] {
  const changed = (file: File) => file.hunks.reduce((sum, hunk) => sum + countChangedLines(hunk.text), 0)
  return [...files].sort(
    (a, b) => FILE_KINDS.indexOf(fileKind(a.path)) - FILE_KINDS.indexOf(fileKind(b.path)) || changed(b) - changed(a) || (a.path < b.path ? -1 : 1),
  )
}

/** A dropped file's line in the facts index: its hunks at their lines at the head, a hunk that only removes lines by its lines at the base. */
export function diffDropLine(drop: DiffDrop): string {
  if (drop.hunks.length === 0) return `- \`${drop.path}\`: its whole section, which has no hunks`
  const head = drop.hunks.filter((hunk) => hunk.new.count > 0).map((hunk) => span(hunk.new.start, hunk.new.count))
  const base = drop.hunks.filter((hunk) => hunk.new.count === 0).map((hunk) => span(hunk.old.start, hunk.old.count))
  const parts = [...(head.length === 0 ? [] : [`lines ${head.join(', ')}`]), ...(base.length === 0 ? [] : [`base lines ${base.join(', ')}`])]
  return `- \`${drop.path}\`: ${parts.join('; ')}`
}

function span(start: number, count: number): string {
  return count === 1 ? String(start) : `${String(start)}-${String(start + count - 1)}`
}

/**
 * The lines at the head each file's hunks in a diff show, by repository
 * path, so a citation of one resolves although the tree does not hold it.
 * A file with no lines at the head (deleted or emptied, `+0,0`) has none to
 * clash with, so its hunks show their lines at the base. Any other hunk
 * that only removes lines shows none: its base numbers name other lines at
 * the head.
 */
export function diffShownLines(diff: string): ReadonlyMap<string, readonly LineRange[]> {
  const shown = new Map<string, LineRange[]>()
  for (const section of parseDiff(diff)) {
    if (section.path === undefined) continue
    const ranges = section.hunks.flatMap((hunk) => {
      const side = hunk.new.count > 0 ? hunk.new : hunk.new.start === 0 ? hunk.old : undefined
      return side === undefined || side.count === 0 ? [] : [{ first: side.start, last: side.start + side.count - 1 }]
    })
    shown.set(section.path, [...(shown.get(section.path) ?? []), ...ranges])
  }
  return new Map([...shown].map(([path, ranges]) => [path, mergeRanges(ranges)]))
}
