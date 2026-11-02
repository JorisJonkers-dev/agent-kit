import { countChangedLines } from './triage.js'

/** A file of a diff whose hunks were left out, and why. */
export interface LeftOutFile {
  readonly path: string
  /** Why it is not shown, as `exclusionReason` words it. */
  readonly reason: string
  /** How many added and removed lines it had. */
  readonly changed_lines: number
}

export interface FilteredDiff {
  readonly diff: string
  readonly left_out: readonly LeftOutFile[]
}

/** Where a hunk sits on one side of the change: `count` lines from `start`. */
export interface HunkSide {
  readonly start: number
  readonly count: number
}

/** One hunk: its `@@` line and the lines under it. */
export interface DiffHunk {
  readonly text: string
  readonly old: HunkSide
  readonly new: HunkSide
}

/** One part of a diff: a file's section, or a line that stands in for one. */
export interface DiffSection {
  /**
   * The file's path at the head, or at the base for a deleted file; none for
   * a line that stands in for a file's section, or text before the first one.
   */
  readonly path: string | undefined
  /** Its lines before the first hunk: `diff --git` and the lines git adds under it. */
  readonly header: string
  readonly hunks: readonly DiffHunk[]
}

/** The line that stands in a diff for a file whose hunks are left out. */
const LEFT_OUT_PREFIX = 'Left out of this diff: '
const HUNK_HEADER = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/

/**
 * A unified diff with the hunks of every file `leftOut` gives a reason for
 * replaced by one line that names the file and says why it is not shown.
 * The line starts with neither `+` nor `-`, so the changed-line count no
 * longer counts the file.
 */
export function filterDiff(diff: string, leftOut: (path: string) => string | undefined): FilteredDiff {
  const kept: string[] = []
  const left: LeftOutFile[] = []
  for (const section of parseDiff(diff)) {
    const reason = section.path === undefined ? undefined : leftOut(section.path)
    if (section.path === undefined || reason === undefined) {
      kept.push(sectionText(section))
      continue
    }
    const changed = countChangedLines(sectionText(section))
    left.push({ changed_lines: changed, path: section.path, reason })
    kept.push(`${LEFT_OUT_PREFIX}${section.path}, ${reason}; its ${String(changed)} changed lines are not shown.\n`)
  }
  return { diff: kept.join(''), left_out: left }
}

/**
 * A diff cut into sections at each `diff --git` line and at each line that
 * stands in for a file, each file's section cut into its header and hunks.
 * Inside a hunk every line starts with `+`, `-`, a space or `\`, so a line
 * that starts with `@@ -` there starts the next hunk.
 */
export function parseDiff(diff: string): readonly DiffSection[] {
  const sections: { path: string | undefined; header: string; hunks: DiffHunk[] }[] = []
  for (const line of diff.split(/(?<=\n)/)) {
    const bare = line.replace(/\n$/, '')
    const current = sections.at(-1)
    const hunk = HUNK_HEADER.exec(bare)
    if (bare.startsWith('diff --git ') || bare.startsWith(LEFT_OUT_PREFIX) || current === undefined) {
      sections.push({ header: line, hunks: [], path: undefined })
    } else if (hunk !== null) {
      current.hunks.push({ new: side(hunk[3], hunk[4]), old: side(hunk[1], hunk[2]), text: line })
    } else {
      const last = current.hunks.at(-1)
      if (last === undefined) current.header += line
      else current.hunks[current.hunks.length - 1] = { ...last, text: last.text + line }
    }
  }
  return sections.map((section) => ({ ...section, path: section.header.startsWith('diff --git ') ? headerPath(section.header) : undefined }))
}

/** A section as the diff prints it. */
export function sectionText(section: DiffSection): string {
  return section.header + section.hunks.map((hunk) => hunk.text).join('')
}

function side(start: string | undefined, count: string | undefined): HunkSide {
  return { count: count === undefined ? 1 : Number(count), start: Number(start) }
}

/**
 * The path a file's header names: its `+++` line, else its `rename to` or
 * `copy to` line, else its `---` line (a deleted file), else its `diff --git`
 * line when that names one path twice (a mode change or a binary file). The
 * `diff --git` line alone cannot say where its first path ends when a path
 * holds ` b/`, so it is read last.
 */
function headerPath(header: string): string | undefined {
  const lines = header.replace(/\n$/, '').split('\n')
  const field = (prefix: string, strip: string) => {
    const line = lines.find((entry) => entry.startsWith(prefix))
    if (line === undefined) return undefined
    const path = unquote(line.slice(prefix.length).replace(/\t$/, ''))
    return path === '/dev/null' || !path.startsWith(strip) ? undefined : path.slice(strip.length)
  }
  return field('+++ ', 'b/') ?? field('rename to ', '') ?? field('copy to ', '') ?? field('--- ', 'a/') ?? gitLinePath(lines[0] ?? '')
}

/** The path a `diff --git a/<path> b/<path>` line names twice, quoted or not. */
function gitLinePath(line: string): string | undefined {
  const rest = line.slice('diff --git '.length)
  if (rest.startsWith('"')) {
    const second = rest.slice(quotedLength(rest) + 1)
    const path = unquote(second)
    return path.startsWith('b/') ? path.slice(2) : undefined
  }
  const length = (rest.length - 5) / 2
  const path = rest.slice(2, 2 + length)
  return rest === `a/${path} b/${path}` ? path : undefined
}

/** How many characters a C-quoted string at the start of `text` takes, its quotes included. */
function quotedLength(text: string): number {
  let i = 1
  while (i < text.length && text[i] !== '"') i += text[i] === '\\' ? 2 : 1
  return i + 1
}

const ESCAPES: Readonly<Record<string, number>> = { '"': 0x22, '\\': 0x5c, a: 0x07, b: 0x08, f: 0x0c, n: 0x0a, r: 0x0d, t: 0x09, v: 0x0b }

/** A path as git prints it, without the C quoting git adds to a path holding a quote, a backslash or a control character. */
function unquote(text: string): string {
  if (!text.startsWith('"') || !text.endsWith('"') || text.length < 2) return text
  const bytes: number[] = []
  const body = text.slice(1, -1)
  for (let i = 0; i < body.length; i += 1) {
    const char = body[i] ?? ''
    if (char !== '\\') {
      bytes.push(...new TextEncoder().encode(char))
      continue
    }
    const octal = /^[0-7]{3}/.exec(body.slice(i + 1))
    if (octal !== null) {
      bytes.push(parseInt(octal[0], 8))
      i += 3
      continue
    }
    const next = body[i + 1] ?? ''
    bytes.push(ESCAPES[next] ?? next.charCodeAt(0))
    i += 1
  }
  return new TextDecoder().decode(new Uint8Array(bytes))
}
