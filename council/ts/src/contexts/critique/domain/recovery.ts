import type { TrackedFile } from './repository-packet.js'
import { lineCount } from './line-references.js'

/** One commit as the history reads it: what the git port's log gives. */
export interface HistoryEntry {
  readonly commit: string
  readonly date: string
  readonly subject: string
  readonly body: string
  readonly files: readonly string[]
}

/** The packet path of the commit log, and the folder of the issues it names. */
export const HISTORY_LOG_PATH = 'history/log.txt'
export const HISTORY_ISSUES_DIR = 'history/issues'

/** How many sampled files the log is asked about, and how many commits it holds at most. */
export const HISTORY_MAX_PATHS = 200
export const HISTORY_MAX_COMMITS = 200
/** How many bytes the log may take, and how many issues it fetches: history is evidence, not a second tree. */
export const HISTORY_CAP_BYTES = 40_960
export const HISTORY_MAX_ISSUES = 10
/** Each issue is cut to this many bytes, so the issues take 20 KiB at most. */
export const HISTORY_ISSUE_BYTES = 2048
/** What a recovery sample leaves of the packet cap for history, so the whole packet stays inside the cap. */
export const HISTORY_RESERVE_BYTES = HISTORY_CAP_BYTES + HISTORY_MAX_ISSUES * HISTORY_ISSUE_BYTES + 512

const SOURCE_EXTENSIONS = new Set([
  'c',
  'cc',
  'cjs',
  'clj',
  'cpp',
  'cs',
  'ex',
  'exs',
  'go',
  'h',
  'hpp',
  'java',
  'js',
  'jsx',
  'kt',
  'kts',
  'lua',
  'mjs',
  'nix',
  'php',
  'py',
  'rb',
  'rs',
  'scala',
  'sh',
  'swift',
  'ts',
  'tsx',
  'zig',
])

/** Whether a tracked path is source code, by its extension: documents, data and configuration are not. */
export function isSourcePath(path: string): boolean {
  const name = path.split('/').at(-1) ?? ''
  const dot = name.lastIndexOf('.')
  return dot > 0 && SOURCE_EXTENSIONS.has(name.slice(dot + 1).toLowerCase())
}

/** How many lines of source the tracked tree holds: the measure a recovery run's depth is chosen by. */
export function sourceLines(tracked: readonly TrackedFile[]): number {
  return tracked.reduce((sum, file) => sum + (file.text !== null && isSourcePath(file.path) ? lineCount(file.text) : 0), 0)
}

/** What `git log` gave, as the packet's `history/log.txt`: each commit's message and the sampled files it changed. */
export function renderHistoryLog(entries: readonly HistoryEntry[], commit: string): { readonly text: string; readonly left_out: number } {
  const header = [
    '# History of the sampled files',
    '',
    `Commits reachable from \`${commit}\` that changed a file under \`tree/\`, newest first. Each message is its author's own text: quote it for a rationale, and never take it for settled fact.`,
    '',
  ].join('\n')
  const note = (n: number): string => `${String(n)} older commits did not fit the history cap of ${String(HISTORY_CAP_BYTES)} bytes.\n`
  let text = header
  let held = 0
  for (const entry of entries) {
    const block = [`## ${entry.commit} ${entry.date}`, `Files: ${entry.files.join(', ')}`, '', entry.subject, ...(entry.body === '' ? [] : ['', entry.body]), '', ''].join('\n')
    if (Buffer.byteLength(text) + Buffer.byteLength(block) + Buffer.byteLength(note(entries.length)) > HISTORY_CAP_BYTES) break
    text += block
    held += 1
  }
  const left = entries.length - held
  return { left_out: left, text: left === 0 ? text : `${text}${note(left)}` }
}

/** The issue numbers the commit messages cite as `#123`, in the order they first appear, at most `HISTORY_MAX_ISSUES`. */
export function citedIssues(entries: readonly HistoryEntry[]): readonly number[] {
  const numbers: number[] = []
  for (const entry of entries) {
    for (const match of `${entry.subject}\n${entry.body}`.matchAll(/(?<![\w/#])#(\d+)\b/g)) {
      const number = Number(match[1])
      if (!numbers.includes(number)) numbers.push(number)
    }
  }
  return numbers.slice(0, HISTORY_MAX_ISSUES)
}

/**
 * The source files a recovery sample takes after the files the records name
 * and each module's entry file, so a repository whose records name no file is
 * still sampled: the shallowest first, since a module's top is where its
 * decisions show, then by path.
 */
export function sourceSample(tracked: readonly TrackedFile[]): readonly string[] {
  return tracked
    .filter((file) => file.text !== null && isSourcePath(file.path))
    .map((file) => file.path)
    .sort((a, b) => a.split('/').length - b.split('/').length || (a < b ? -1 : 1))
}
