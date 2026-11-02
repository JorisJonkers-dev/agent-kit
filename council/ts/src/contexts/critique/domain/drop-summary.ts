import { bytes, heldText, holdsAll, lineBytes, type LineRange, type TreeCandidate } from './tree-sample.js'

/**
 * How the facts index records the files a packet leaves out that no critic
 * is likely to look for by name: counted per directory and reason, with the
 * bytes the packet does not hold, so the index stays small however many
 * files are left out. The files a critic is likely to look for (the ones
 * the artifact or the diff names, the top hotspots, the importers of the
 * changed files) keep a line of their own, which the samplers write.
 */

/** One file left out, in whole or in part. */
export interface Drop {
  readonly path: string
  /** What became of it, worded for many files: `the cap dropped them`. */
  readonly reason: string
  /** The bytes of it the packet does not hold. */
  readonly bytes: number
}

/** The most lines a summary takes; past it, directories are counted by a shorter prefix. */
export const DROP_SUMMARY_LINES = 24

export const CAP_DROPPED = 'the cap dropped them'

/** The reason of a file the cap cut short, which holds whole windows from its first line. */
export function capCutShort(windowLines: number): string {
  return `the cap cut them short: each holds only its first lines, in windows of ${String(windowLines)} lines`
}

/** What the cap left out of a candidate holding these lines, or nothing when it holds every line. */
export function capDrop(candidate: TreeCandidate, held: readonly LineRange[], windowLines: number): Drop | undefined {
  if (holdsAll(candidate, held)) return undefined
  return {
    bytes: bytes(candidate.text) - bytes(heldText(candidate, held)),
    path: candidate.path,
    reason: held.length === 0 ? CAP_DROPPED : capCutShort(windowLines),
  }
}

/** Every way the cap may leave a candidate out, at the most bytes each can take: dropped, or cut short when it has more than one window. */
export function possibleDrops(candidate: TreeCandidate, windowLines: number): readonly Drop[] {
  const whole = { bytes: bytes(candidate.text), path: candidate.path, reason: CAP_DROPPED }
  return candidate.lineCount > windowLines ? [whole, { ...whole, reason: capCutShort(windowLines) }] : [whole]
}

/**
 * The facts index lines for these drops: one per directory and reason, with
 * the count and the bytes, in directory order. When that passes
 * `DROP_SUMMARY_LINES`, a file more than `k` folders deep is counted under
 * its first `k` folders as `a/b/**`, for the largest `k` that fits.
 */
export function summariseDrops(drops: readonly Drop[]): readonly string[] {
  const deepest = Math.max(0, ...drops.map((drop) => foldersOf(drop.path).length))
  for (let k = deepest + 1; k > 0; k -= 1) {
    const lines = groupLines(drops, k)
    if (lines.length <= DROP_SUMMARY_LINES) return lines
  }
  return groupLines(drops, 0)
}

/**
 * The most bytes `summariseDrops` can take, newlines included, for any of
 * these possible drops: each file once, with one of the reasons it may get,
 * its bytes at most those given. A packet reserves it under its cap before
 * it knows what it drops.
 */
export function dropSummaryBound(possible: readonly Drop[]): number {
  const exact = new Map<string, { key: string; reason: string; paths: Set<string>; bytes: number }>()
  for (const drop of possible) {
    const key = keyOf(drop.path, Infinity)
    const id = `${key}\u0000${drop.reason}`
    const group = exact.get(id) ?? { bytes: 0, key, paths: new Set<string>(), reason: drop.reason }
    group.paths.add(drop.path)
    exact.set(id, { ...group, bytes: group.bytes + drop.bytes })
  }
  const groups = [...exact.values()]
  if (groups.length <= DROP_SUMMARY_LINES) return groups.reduce((sum, group) => sum + lineBytes(line(group.key, group.paths.size, group.bytes, group.reason)), 0)
  const most = new Map<string, number>()
  for (const drop of possible) most.set(drop.path, Math.max(most.get(drop.path) ?? 0, drop.bytes))
  const total = [...most.values()].reduce((sum, size) => sum + size, 0)
  const key = Math.max(...groups.map((group) => bytes(group.key) + '**'.length))
  const reasons = new Set(possible.map((drop) => drop.reason))
  const widest = lineBytes(line('k'.repeat(key), most.size, total, 'r'.repeat(Math.max(...[...reasons].map(bytes)))))
  return Math.max(DROP_SUMMARY_LINES, reasons.size) * widest
}

function foldersOf(path: string): readonly string[] {
  return path.split('/').slice(0, -1)
}

/** The directory a file is counted under when files more than `k` folders deep are counted by their first `k`. */
function keyOf(path: string, k: number): string {
  const folders = foldersOf(path)
  if (folders.length < k) return folders.length === 0 ? './' : `${folders.join('/')}/`
  return k === 0 ? '**' : `${folders.slice(0, k).join('/')}/**`
}

function groupLines(drops: readonly Drop[], k: number): readonly string[] {
  const groups = new Map<string, { key: string; reason: string; count: number; bytes: number }>()
  for (const drop of drops) {
    const key = keyOf(drop.path, k)
    const id = `${key}\u0000${drop.reason}`
    const group = groups.get(id) ?? { bytes: 0, count: 0, key, reason: drop.reason }
    groups.set(id, { ...group, bytes: group.bytes + drop.bytes, count: group.count + 1 })
  }
  return [...groups.values()]
    .sort((a, b) => (a.key === b.key ? (a.reason < b.reason ? -1 : 1) : a.key < b.key ? -1 : 1))
    .map((group) => line(group.key, group.count, group.bytes, group.reason))
}

function line(key: string, count: number, size: number, reason: string): string {
  return `- \`${key}\` (${String(count)} ${count === 1 ? 'file' : 'files'}, ${String(size)} bytes): ${reason}`
}
