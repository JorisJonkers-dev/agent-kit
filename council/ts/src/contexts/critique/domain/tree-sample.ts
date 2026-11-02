import { lineCount } from './line-references.js'

/**
 * How a packet's `tree/` is filled under a cap, by rule and never by a
 * model. Each file is a candidate, sent as the pieces of it that fit: its
 * windows (runs of lines from its first) or, for a code packet, the slices
 * around a change. A file holds every line of the pieces that went in, at
 * its own line numbers; a run of lines it does not hold before its last
 * held line is blank but for a marker on its first line, and the file ends
 * at its last held line.
 */

/** Lines `first` to `last` of a file, both included, counted from 1. */
export interface LineRange {
  readonly first: number
  readonly last: number
}

/** A file that may go into the tree. */
export interface TreeCandidate<Tier extends string = string> {
  readonly path: string
  /** Why it was chosen: the first tier it was chosen in. */
  readonly tier: Tier
  readonly text: string
  readonly lineCount: number
}

/** Some lines of a candidate, offered to the cap in the order pieces are listed. */
export interface TreePiece<Tier extends string = string> {
  readonly candidate: TreeCandidate<Tier>
  readonly ranges: readonly LineRange[]
  /**
   * A window runs from where its file's last window stopped: one that does
   * not fit drops the rest of its file's windows. A slice stands alone.
   */
  readonly window: boolean
  /** The piece's own line in the facts index, by whether it went in. */
  readonly note?: (admitted: boolean) => string
}

/** What went in: each candidate's held lines, merged, and the pieces admitted. */
export interface TreeAdmission<Tier extends string = string> {
  readonly held: ReadonlyMap<TreeCandidate<Tier>, readonly LineRange[]>
  readonly admitted: ReadonlySet<TreePiece<Tier>>
}

/** A tree file as frozen, and only when it was cut short, which of its lines it holds. */
export interface SampledFile {
  readonly path: string
  readonly text: string
  /** Only when the file was cut short: it holds `ranges` (or lines 1 to `last`) of its `lines`. */
  readonly window?: { readonly last: number; readonly lines: number; readonly ranges?: readonly LineRange[] }
}

const encoder = new TextEncoder()

export function bytes(text: string): number {
  return encoder.encode(text).length
}

/** A candidate for a file's text. */
export function candidateOf<Tier extends string>(path: string, tier: Tier, text: string): TreeCandidate<Tier> {
  return { lineCount: lineCount(text), path, text, tier }
}

/** A candidate's windows of `size` lines, from its first line; an empty file is one empty window. */
export function windowsOf<Tier extends string>(candidate: TreeCandidate<Tier>, size: number): readonly TreePiece<Tier>[] {
  const count = Math.max(1, Math.ceil(candidate.lineCount / size))
  return Array.from({ length: count }, (_, i) => ({
    candidate,
    ranges: [{ first: i * size + 1, last: Math.max(i * size + 1, Math.min(candidate.lineCount, (i + 1) * size)) }],
    window: true,
  }))
}

/**
 * Offers each piece to the budget in order. A piece costs the bytes it adds
 * to its file, and the change it makes to the facts index: its file's line
 * there, as `describe` words it (none when the line is undefined), and the
 * piece's own note. With no `describe`, the facts index has a cap of its
 * own, so a piece costs only its bytes. A piece that does not fit is left
 * out, and a window that does not fit drops its file's later windows.
 */
export function admitPieces<Tier extends string>(
  pieces: readonly TreePiece<Tier>[],
  budget: number,
  describe: ((candidate: TreeCandidate<Tier>, held: readonly LineRange[]) => string | undefined) | undefined,
): TreeAdmission<Tier> {
  const held = new Map<TreeCandidate<Tier>, readonly LineRange[]>()
  const admitted = new Set<TreePiece<Tier>>()
  const blocked = new Set<TreeCandidate<Tier>>()
  let remaining = budget
  for (const piece of pieces) {
    const { candidate } = piece
    if (piece.window && blocked.has(candidate)) continue
    const before = held.get(candidate) ?? []
    const after = mergeRanges([...before, ...piece.ranges])
    const index =
      describe === undefined
        ? 0
        : lineBytes(describe(candidate, after)) - lineBytes(describe(candidate, before)) + lineBytes(piece.note?.(true)) - lineBytes(piece.note?.(false))
    const cost = bytes(heldText(candidate, after)) - bytes(heldText(candidate, before)) + index
    if (cost > remaining) {
      if (piece.window) blocked.add(candidate)
      continue
    }
    remaining -= cost
    held.set(candidate, after)
    admitted.add(piece)
  }
  return { admitted, held }
}

/** A line's bytes in the facts index, its newline included; none for no line. */
export function lineBytes(line: string | undefined): number {
  return line === undefined ? 0 : bytes(line) + 1
}

/** Sorted, with overlapping and touching ranges merged. */
export function mergeRanges(ranges: readonly LineRange[]): readonly LineRange[] {
  const sorted = [...ranges].sort((a, b) => a.first - b.first || a.last - b.last)
  const merged: LineRange[] = []
  for (const range of sorted) {
    const last = merged.at(-1)
    if (last !== undefined && range.first <= last.last + 1) merged[merged.length - 1] = { first: last.first, last: Math.max(last.last, range.last) }
    else merged.push(range)
  }
  return merged
}

/** Whether held ranges cover every line of a candidate. */
export function holdsAll(candidate: TreeCandidate, held: readonly LineRange[]): boolean {
  const [only] = held
  return held.length === 1 && only?.first === 1 && only.last >= candidate.lineCount
}

/** The marker on the first line of a run of lines a file does not hold. */
export function gapMarker(range: LineRange): string {
  return `… lines ${String(range.first)}-${String(range.last)} are not in the packet; facts/INDEX.md says why`
}

/**
 * The text a file holds: all of it once every line is held, nothing when no
 * line is, and otherwise its held lines at their own numbers up to its last
 * held line, each run between them blank but for a marker on its first line.
 */
export function heldText(candidate: TreeCandidate, held: readonly LineRange[]): string {
  if (held.length === 0) return ''
  if (holdsAll(candidate, held)) return candidate.text
  const lines = candidate.text.split('\n')
  const out: string[] = []
  let next = 1
  for (const range of held) {
    if (range.first > next) out.push(gapMarker({ first: next, last: range.first - 1 }), ...Array.from({ length: range.first - next - 1 }, () => ''))
    out.push(...lines.slice(range.first - 1, range.last))
    next = range.last + 1
  }
  return `${out.join('\n')}\n`
}

/** The frozen tree file for a candidate's held lines, or nothing when it holds none. */
export function sampledFile(candidate: TreeCandidate, held: readonly LineRange[]): SampledFile | undefined {
  if (held.length === 0) return undefined
  const text = heldText(candidate, held)
  if (holdsAll(candidate, held)) return { path: candidate.path, text }
  const last = held.at(-1)?.last ?? 0
  const prefix = held.length === 1 && held[0]?.first === 1
  return { path: candidate.path, text, window: { last, lines: candidate.lineCount, ...(prefix ? {} : { ranges: held }) } }
}

/** `1-400`, or `1-3, 10-20`. */
export function formatRanges(ranges: readonly LineRange[]): string {
  return ranges.map((range) => `${String(range.first)}-${String(range.last)}`).join(', ')
}

/** The lines of a candidate its held ranges leave out. */
export function missingRanges(candidate: TreeCandidate, held: readonly LineRange[]): readonly LineRange[] {
  const missing: LineRange[] = []
  let next = 1
  for (const range of held) {
    if (range.first > next) missing.push({ first: next, last: range.first - 1 })
    next = range.last + 1
  }
  if (next <= candidate.lineCount) missing.push({ first: next, last: candidate.lineCount })
  return missing
}

/** What became of a file: dropped, sent in full, or which lines were sent and which the cap dropped. */
export function sentStatus(candidate: TreeCandidate, held: readonly LineRange[]): string {
  if (held.length === 0) return 'the cap dropped it'
  if (holdsAll(candidate, held)) return 'sent in full'
  return `sent lines ${formatRanges(held)}; the cap dropped lines ${formatRanges(missingRanges(candidate, held))}`
}
