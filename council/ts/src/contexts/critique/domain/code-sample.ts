import type { ContextSlice } from './context-slices.js'
import type { LeftOutFile } from './diff-filter.js'
import { diffDropLine, windowDiff, type DiffDrop, type WindowedDiff } from './diff-window.js'
import { capDrop, dropSummaryBound, possibleDrops, summariseDrops } from './drop-summary.js'
import { refusal, type Refusal } from './outcomes.js'
import { rankIndex } from './ranking.js'
import { REPO_MAP_PATH, type RenderedRepoMap } from './repo-map.js'
import { FACTS_INDEX_PATH, namedPaths, type ExclusionKind, type PacketLimits, type TrackedFile } from './repository-packet.js'
import {
  admitPieces,
  bytes,
  candidateOf,
  holdsAll,
  lineBytes,
  sampledFile,
  sentStatus,
  windowsOf,
  type LineRange,
  type SampledFile,
  type TreeCandidate,
  type TreePiece,
} from './tree-sample.js'

/**
 * Why a file is in a code packet's tree, in priority order: the change
 * touched it, it imports a file the change touched, it holds a slice
 * (a caller or a callee), or it is the rest of the tree.
 */
const CODE_TIERS = ['changed', 'importer', 'related', 'rest'] as const
type CodeTier = (typeof CODE_TIERS)[number]

/** How much of the room the documents leave a windowed diff may take; the tree keeps the rest. */
const DIFF_SHARE = 0.5
/**
 * How much of the room the documents leave the facts index's lists of names
 * may take when, listing every drop it names, it would not fit: each list
 * then ends with a line that counts what it leaves unlisted. The room
 * reserved for the per-directory counts comes on top.
 */
const INDEX_SHARE = 0.25

const DIFF_WINDOW_NOTE =
  '`diff/change.diff` did not fit beside the rest of the packet, so it holds the hunks that fit, in windows by file: the changed source files, then the tests, then the docs, each group by changed lines, most first. Each hunk keeps its `@@` line, so a line it shows is cited at its own number as `tree/<path>:<line>`. A line only a left-out hunk shows can be read and cited only where `tree/` holds it, as the Tree section says. The cap left out these hunks, at their lines at the commit under review, or as `base lines` for a hunk that only removes lines:'

export interface CodeSampleRequest {
  readonly commit: string
  readonly limits: PacketLimits
  /** Every tracked path not excluded, sorted, with its text, or null for a binary file. */
  readonly tracked: readonly TrackedFile[]
  /** How many tracked paths each exclusion left out of the tree. */
  readonly excluded: Readonly<Record<ExclusionKind, number>>
  /** The files whose hunks the diff leaves out. */
  readonly leftOut: readonly LeftOutFile[]
  /** The diff, without those hunks, before the cap windows it. */
  readonly diff: string
  /** The bytes the tickets and the spec take. */
  readonly documentBytes: number
  readonly repoMap: RenderedRepoMap
  /** The repository paths the change touched that the tree holds. */
  readonly changed: readonly string[]
  /** The repository paths that import a changed file. */
  readonly importers: readonly string[]
  readonly slices: readonly ContextSlice[]
  /** The text of the tickets and the spec, which, with the diff's, names files. */
  readonly namedText: string
  /** The paths code-nav's repository map ranked, most referenced first; the importers and the rest of the tree go in that order. */
  readonly ranked?: readonly string[]
  /** What the map did to the sample's order, for the facts index; absent when no map was asked for. */
  readonly rankingNote?: string
}

export interface CodeSample {
  readonly status: 'sampled'
  /** The diff as sent: whole, or in windows when it alone does not fit. */
  readonly diff: string
  readonly index: string
  readonly tree: readonly SampledFile[]
  /** The slices the cap let in, in the order they were chosen. */
  readonly slices: readonly ContextSlice[]
  /** How many files of the tree the cap left out in whole or in part. */
  readonly dropped: number
}

/**
 * Chooses what of the tree a code packet sends, the way a repository packet
 * is capped: first every context slice, then the files the change touched,
 * then the files that import them, then the rest of the tree, in windows
 * from each file's first line, while they fit under the cap beside the
 * diff, the tickets, the spec, the repo map and the facts index. A diff
 * that does not fit beside the rest is sent in windows by file, taking at
 * most half the room the documents leave, so the tree keeps room for the
 * slices. Every drop is recorded in the facts index: by name for a file the
 * change touched, an importer, a file holding a slice, or a file the diff,
 * a ticket or the spec names; counted per directory for the rest of the
 * tree, under a share of the cap reserved for the most that count can take.
 * When listing every named drop would not fit, its lists of names take a
 * quarter of the room the documents and that reserve leave, and each counts
 * what it leaves unlisted.
 * Refused only when the tickets, the spec, the repo map and the facts
 * index's own text pass the cap.
 */
export function sampleCodeTree(request: CodeSampleRequest): CodeSample | Refusal {
  const { limits } = request
  const candidates = codeCandidates(request)
  const named = new Set(namedPaths(`${request.diff}\n${request.namedText}`))
  const counted = candidates.filter((candidate) => candidate.tier === 'rest' && !named.has(candidate.path))
  const countedSet = new Set(counted)
  const treeLine = (candidate: TreeCandidate<CodeTier>, held: readonly LineRange[]): string | undefined =>
    countedSet.has(candidate) || (candidate.tier === 'rest' && holdsAll(candidate, held)) ? undefined : fileLine(candidate, held)
  const reserved = dropSummaryBound(counted.flatMap((candidate) => possibleDrops(candidate, limits.window_lines)))
  const unsent = candidates.flatMap((candidate) => treeLine(candidate, []) ?? [])
  const byPath = new Map(candidates.map((candidate) => [candidate.path, candidate]))
  const sliced = request.slices.flatMap((slice) => {
    const candidate = byPath.get(slice.path)
    const piece: TreePiece<CodeTier> | undefined =
      candidate === undefined ? undefined : { candidate, note: (admitted) => sliceLine(slice, admitted), ranges: [slice], window: false }
    return piece === undefined ? [] : [{ piece, slice }]
  })
  const slicePieces = sliced.map((entry) => entry.piece)
  const pieces = [...slicePieces, ...windowOrder(candidates, limits.window_lines)]
  const none = { admitted: new Set<TreePiece<CodeTier>>(), held: new Map<TreeCandidate<CodeTier>, readonly LineRange[]>() }
  const documents = request.documentBytes + bytes(request.repoMap.text)
  const fixedWith = (diff: WindowedDiff) => documents + bytes(diff.diff) + bytes(renderIndex(request, unsent, [], slicePieces, none, diff.dropped)) + reserved
  const full: WindowedDiff = { diff: request.diff, dropped: [] }
  let diff = full
  // Set only when the facts index, listing every drop it names, would not fit: it is then capped on its own.
  let indexCap: number | undefined
  if (fixedWith(full) > limits.cap_bytes) {
    const empty = windowDiff(request.diff, 0)
    const fixedBefore = fixedWith(empty)
    if (fixedBefore <= limits.cap_bytes) {
      diff = windowDiff(request.diff, Math.floor((limits.cap_bytes - fixedBefore) * DIFF_SHARE))
    } else {
      // Every list collapsed, each count line naming a cap no shorter than the one the index gets.
      const collapsed = bytes(renderIndex(request, unsent, [], slicePieces, none, empty.dropped, limits.cap_bytes, false))
      const floor = documents + bytes(empty.diff) + collapsed + reserved
      if (floor > limits.cap_bytes) {
        return refusal(
          'packet-over-cap',
          `the tickets, the spec, ${REPO_MAP_PATH} and ${FACTS_INDEX_PATH} come to ${String(floor)} bytes before any hunk of the diff, past the packet cap of ${String(limits.cap_bytes)}; review a smaller change or raise packet.cap_bytes in routing`,
        )
      }
      // The lists get their share of the room beside the reserve for the counted drops.
      const room = limits.cap_bytes - documents - bytes(empty.diff) - reserved
      const listsCap = Math.max(collapsed, Math.floor(room * INDEX_SHARE))
      indexCap = listsCap + reserved
      const windowed = windowDiff(request.diff, Math.floor((room - listsCap) * DIFF_SHARE))
      diff = windowed.dropped.length === 0 ? full : windowed
    }
  }
  const admission =
    indexCap === undefined
      ? admitPieces(pieces, limits.cap_bytes - fixedWith(diff), treeLine)
      : admitPieces(pieces, limits.cap_bytes - documents - bytes(diff.diff) - indexCap, undefined)
  const heldOf = (candidate: TreeCandidate<CodeTier>) => admission.held.get(candidate) ?? []
  const listed = candidates.flatMap((candidate) => treeLine(candidate, heldOf(candidate)) ?? [])
  const counts = summariseDrops(counted.flatMap((candidate) => capDrop(candidate, heldOf(candidate), limits.window_lines) ?? []))
  const tree = candidates.flatMap((candidate) => {
    const file = sampledFile(candidate, admission.held.get(candidate) ?? [])
    return file === undefined ? [] : [file]
  })
  return {
    diff: diff.diff,
    dropped: candidates.filter((candidate) => !holdsAll(candidate, admission.held.get(candidate) ?? [])).length,
    index: renderIndex(request, listed, counts, slicePieces, admission, diff.dropped, indexCap),
    slices: sliced.filter((entry) => admission.admitted.has(entry.piece)).map((entry) => entry.slice),
    status: 'sampled',
    tree,
  }
}

/** Every text file of the tree, in tier order, each tier in path order. */
function codeCandidates(request: CodeSampleRequest): readonly TreeCandidate<CodeTier>[] {
  const changed = new Set(request.changed)
  const importers = new Set(request.importers)
  const related = new Set(request.slices.map((slice) => slice.path))
  const tierOf = (path: string): CodeTier =>
    changed.has(path) ? 'changed' : importers.has(path) ? 'importer' : related.has(path) ? 'related' : 'rest'
  const place = rankIndex(request.ranked ?? [])
  // The ranking orders the importers and the rest of the tree; the files the change touched keep path order.
  const rankOf = (candidate: TreeCandidate<CodeTier>) => (candidate.tier === 'importer' || candidate.tier === 'rest' ? place(candidate.path) : 0)
  return request.tracked
    .flatMap((file) => (file.text === null || `tree/${file.path}` === REPO_MAP_PATH ? [] : [candidateOf(file.path, tierOf(file.path), file.text)]))
    .sort((a, b) => CODE_TIERS.indexOf(a.tier) - CODE_TIERS.indexOf(b.tier) || rankOf(a) - rankOf(b) || (a.path < b.path ? -1 : 1))
}

/** Each tier's first windows before its second; a file that only holds a slice is the rest of the tree here. */
function windowOrder(candidates: readonly TreeCandidate<CodeTier>[], size: number): readonly TreePiece<CodeTier>[] {
  const rank = (tier: CodeTier) => Math.min(CODE_TIERS.indexOf(tier), CODE_TIERS.indexOf('related'))
  return candidates
    .flatMap((candidate, position) => windowsOf(candidate, size).map((piece, window) => ({ piece, position, window })))
    .sort((a, b) => rank(a.piece.candidate.tier) - rank(b.piece.candidate.tier) || a.window - b.window || a.position - b.position)
    .map((entry) => entry.piece)
}

/** A file's own line in the facts index. */
function fileLine(candidate: TreeCandidate<CodeTier>, held: readonly LineRange[]): string {
  return `- \`${candidate.path}\` (${candidate.tier}, ${String(candidate.lineCount)} lines): ${sentStatus(candidate, held)}`
}

/** What a slice holds, as its line in the prompt and the facts index say it. */
export function sliceReason(slice: ContextSlice): string {
  const symbol = slice.symbol === undefined ? undefined : `\`${slice.symbol}\``
  const of = `\`${slice.of ?? ''}\``
  if (slice.reason === 'callee') return `${symbol ?? ''}, which ${of} calls`
  if (slice.reason === 'caller') return `${symbol ?? ''}, which calls ${of}`
  return symbol === undefined ? 'lines the change touches' : `${symbol}, which the change touches`
}

/** The packet citation of a slice: `tree/<path>:<first>-<last>`. */
export function sliceCitation(slice: ContextSlice): string {
  return `tree/${slice.path}:${String(slice.first)}-${String(slice.last)}`
}

function sliceLine(slice: ContextSlice, admitted: boolean): string {
  return `- \`${sliceCitation(slice)}\`: ${sliceReason(slice)}: ${admitted ? 'sent' : 'the cap dropped it'}`
}

/**
 * The facts index. `tree` holds the tree's files listed by name, which a cap
 * on the index may collapse like its other lists; `counts` holds the
 * per-directory counts of the other drops, which it always keeps, as the
 * cap reserved room for them. With a cap and no `fill`, every list is
 * collapsed to its count line: the least the index can take.
 */
function renderIndex(
  request: CodeSampleRequest,
  tree: readonly string[],
  counts: readonly string[],
  slicePieces: readonly TreePiece<CodeTier>[],
  admission: { readonly admitted: ReadonlySet<TreePiece<CodeTier>>; readonly held: ReadonlyMap<TreeCandidate<CodeTier>, readonly LineRange[]> },
  diffDropped: readonly DiffDrop[],
  capBytes?: number,
  fill = true,
): string {
  const lists = [
    request.leftOut.map((file) => `- \`${file.path}\`: ${file.reason}; ${String(file.changed_lines)} changed lines`),
    diffDropped.map(diffDropLine),
    slicePieces.map((piece) => piece.note?.(admission.admitted.has(piece)) ?? ''),
    tree,
  ] as const
  const render = (shown: readonly (readonly string[])[]) => indexText(request, shown, counts)
  if (capBytes === undefined) return render(lists)
  const kept = lists.map(() => 0)
  const cut = () => lists.map((list, i) => collapse(list, kept[i] ?? 0, capBytes))
  let remaining = fill ? capBytes - bytes(render(cut())) : 0
  for (const [i, list] of lists.entries()) {
    for (const line of list) {
      if (lineBytes(line) > remaining) break
      remaining -= lineBytes(line)
      kept[i] = (kept[i] ?? 0) + 1
    }
  }
  return render(cut())
}

/** A list's first `kept` lines, then, when that leaves some out, one line that counts them. */
function collapse(list: readonly string[], kept: number, capBytes: number): readonly string[] {
  if (kept >= list.length) return list
  return [...list.slice(0, kept), `- … and ${String(list.length - kept)} more, which this index's cap of ${String(capBytes)} bytes leaves unlisted`]
}

/** The facts index around its four lists (the files left out of the diff, its hunks the cap left out, the slices and the tree's named files) and the tree's counted drops. */
function indexText(request: CodeSampleRequest, [leftOut = [], diffDropped = [], slices = [], tree = []]: readonly (readonly string[])[], counts: readonly string[]): string {
  const { limits, repoMap } = request
  const { generated, lock, vendored } = request.excluded
  const binary = request.tracked.filter((file) => file.text === null).length
  return [
    '# Facts',
    '',
    `What the runner computed from the repository at commit \`${request.commit}\`. Facts are settled: cite them, never recompute them. A quote from a fact is a fact, never a recorded decision.`,
    '',
    '## Files',
    '',
    `- \`${REPO_MAP_PATH}\`: every source file's top-level symbols and their signatures, ranked by how many other files reference them${repoMap.dropped === 0 ? '' : `; its cap left out the ${String(repoMap.dropped)} least referenced files`}.`,
    '',
    '## Limits',
    '',
    `Packet cap ${String(limits.cap_bytes)} bytes; repo map cap ${String(limits.repo_map_cap_bytes)} bytes; windows of ${String(limits.window_lines)} lines.`,
    '',
    ...(request.rankingNote === undefined ? [] : ['## Ranking', '', request.rankingNote, '']),
    '## Excluded',
    '',
    `Never sent under \`tree/\`: ${String(vendored)} vendored, ${String(generated)} generated and ${String(lock)} lock files, and ${String(binary)} binary files.`,
    ...(leftOut.length === 0 ? [] : ['', 'Left out of `diff/change.diff`, each with one line in its place:', '', ...leftOut]),
    ...(diffDropped.length === 0 ? [] : ['', '## Diff', '', DIFF_WINDOW_NOTE, '', ...diffDropped]),
    '',
    '## Context slices',
    '',
    'The code around the change, at its own line numbers: each changed function or block, then the symbols it calls, then the functions that call it.',
    '',
    ...(slices.length === 0 ? ['The change touches no text file the tree holds.'] : slices),
    '',
    '## Tree',
    '',
    `Files under \`tree/\`, in the order the cap let them in: the slices above, then the files the change touched, then the files that import them, then the rest of the tree. A file goes in windows of ${String(limits.window_lines)} lines from its first line. A file cut short holds the lines listed here at their own numbers; a run of lines it does not hold is blank but for a marker on its first line. Of the rest of the tree, a file the diff, a ticket or the spec names is listed by name, the cap's other drops are counted per directory with the bytes left out (\`dir/**\` counts every file under it), and a file neither listed nor counted was sent in full.`,
    '',
    ...tree,
    ...counts,
    '',
  ].join('\n')
}
