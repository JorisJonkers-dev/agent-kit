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

const FILE_HEADER = /^diff --git a\/(.+) b\/(.+)$/

/**
 * A unified diff with the hunks of every file `leftOut` gives a reason for
 * replaced by one line that names the file and says why it is not shown.
 * The line starts with neither `+` nor `-`, so the changed-line count no
 * longer counts the file.
 */
export function filterDiff(diff: string, leftOut: (path: string) => string | undefined): FilteredDiff {
  const kept: string[] = []
  const left: LeftOutFile[] = []
  for (const section of fileSections(diff)) {
    const reason = section.path === undefined ? undefined : leftOut(section.path)
    if (section.path === undefined || reason === undefined) {
      kept.push(section.text)
      continue
    }
    const changed = countChangedLines(section.text)
    left.push({ changed_lines: changed, path: section.path, reason })
    kept.push(`Left out of this diff: ${section.path}, ${reason}; its ${String(changed)} changed lines are not shown.\n`)
  }
  return { diff: kept.join(''), left_out: left }
}

/** The diff cut at each `diff --git` line, each part with the (new) path its header names. */
function fileSections(diff: string): readonly { readonly path: string | undefined; readonly text: string }[] {
  const sections: { path: string | undefined; text: string }[] = []
  for (const line of diff.split(/(?<=\n)/)) {
    const header = FILE_HEADER.exec(line.replace(/\n$/, ''))
    const current = sections.at(-1)
    if (header !== null || current === undefined) sections.push({ path: header?.[2], text: line })
    else current.text += line
  }
  return sections
}
