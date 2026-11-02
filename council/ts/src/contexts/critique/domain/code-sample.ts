import type { ContextSlice } from './context-slices.js'
import type { LeftOutFile } from './diff-filter.js'
import { refusal, type Refusal } from './outcomes.js'
import { REPO_MAP_PATH, type RenderedRepoMap } from './repo-map.js'
import { FACTS_INDEX_PATH, type ExclusionKind, type PacketLimits, type TrackedFile } from './repository-packet.js'
import {
  admitPieces,
  bytes,
  candidateOf,
  holdsAll,
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

export interface CodeSampleRequest {
  readonly commit: string
  readonly limits: PacketLimits
  /** Every tracked path not excluded, sorted, with its text, or null for a binary file. */
  readonly tracked: readonly TrackedFile[]
  /** How many tracked paths each exclusion left out of the tree. */
  readonly excluded: Readonly<Record<ExclusionKind, number>>
  /** The files whose hunks the diff leaves out. */
  readonly leftOut: readonly LeftOutFile[]
  /** The bytes the diff, the tickets and the spec take. */
  readonly documentBytes: number
  readonly repoMap: RenderedRepoMap
  /** The repository paths the change touched that the tree holds. */
  readonly changed: readonly string[]
  /** The repository paths that import a changed file. */
  readonly importers: readonly string[]
  readonly slices: readonly ContextSlice[]
}

export interface CodeSample {
  readonly status: 'sampled'
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
 * diff, the tickets, the spec, the repo map and the facts index. Every drop
 * is recorded in the facts index. Refused when those alone pass the cap.
 */
export function sampleCodeTree(request: CodeSampleRequest): CodeSample | Refusal {
  const { limits } = request
  const candidates = codeCandidates(request)
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
  const fixed = request.documentBytes + bytes(request.repoMap.text) + bytes(renderIndex(request, candidates, slicePieces, none))
  if (fixed > limits.cap_bytes) {
    return refusal(
      'packet-over-cap',
      `the diff, the tickets, the spec, ${REPO_MAP_PATH} and ${FACTS_INDEX_PATH} come to ${String(fixed)} bytes, past the packet cap of ${String(limits.cap_bytes)}; review a smaller change or raise packet.cap_bytes in routing`,
    )
  }
  const admission = admitPieces(pieces, limits.cap_bytes - fixed, treeLine)
  const tree = candidates.flatMap((candidate) => {
    const file = sampledFile(candidate, admission.held.get(candidate) ?? [])
    return file === undefined ? [] : [file]
  })
  return {
    dropped: candidates.filter((candidate) => !holdsAll(candidate, admission.held.get(candidate) ?? [])).length,
    index: renderIndex(request, candidates, slicePieces, admission),
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
  return request.tracked
    .flatMap((file) => (file.text === null || `tree/${file.path}` === REPO_MAP_PATH ? [] : [candidateOf(file.path, tierOf(file.path), file.text)]))
    .sort((a, b) => CODE_TIERS.indexOf(a.tier) - CODE_TIERS.indexOf(b.tier) || (a.path < b.path ? -1 : 1))
}

/** Each tier's first windows before its second; a file that only holds a slice is the rest of the tree here. */
function windowOrder(candidates: readonly TreeCandidate<CodeTier>[], size: number): readonly TreePiece<CodeTier>[] {
  const rank = (tier: CodeTier) => Math.min(CODE_TIERS.indexOf(tier), CODE_TIERS.indexOf('related'))
  return candidates
    .flatMap((candidate, position) => windowsOf(candidate, size).map((piece, window) => ({ piece, position, window })))
    .sort((a, b) => rank(a.piece.candidate.tier) - rank(b.piece.candidate.tier) || a.window - b.window || a.position - b.position)
    .map((entry) => entry.piece)
}

/** A file's line in the facts index; a file of the rest of the tree has one only when the cap cut it. */
function treeLine(candidate: TreeCandidate<CodeTier>, held: readonly LineRange[]): string | undefined {
  if (candidate.tier === 'rest' && holdsAll(candidate, held)) return undefined
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

function renderIndex(
  request: CodeSampleRequest,
  candidates: readonly TreeCandidate<CodeTier>[],
  slicePieces: readonly TreePiece<CodeTier>[],
  admission: { readonly admitted: ReadonlySet<TreePiece<CodeTier>>; readonly held: ReadonlyMap<TreeCandidate<CodeTier>, readonly LineRange[]> },
): string {
  const { limits, repoMap } = request
  const { generated, lock, vendored } = request.excluded
  const binary = request.tracked.filter((file) => file.text === null).length
  const leftOut = request.leftOut.map((file) => `- \`${file.path}\`: ${file.reason}; ${String(file.changed_lines)} changed lines`)
  const slices = slicePieces.map((piece) => piece.note?.(admission.admitted.has(piece)) ?? '')
  const tree = candidates.flatMap((candidate) => treeLine(candidate, admission.held.get(candidate) ?? []) ?? [])
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
    '## Excluded',
    '',
    `Never sent under \`tree/\`: ${String(vendored)} vendored, ${String(generated)} generated and ${String(lock)} lock files, and ${String(binary)} binary files.`,
    ...(leftOut.length === 0 ? [] : ['', 'Left out of `diff/change.diff`, each with one line in its place:', '', ...leftOut]),
    '',
    '## Context slices',
    '',
    'The code around the change, at its own line numbers: each changed function or block, then the symbols it calls, then the functions that call it.',
    '',
    ...(slices.length === 0 ? ['The change touches no text file the tree holds.'] : slices),
    '',
    '## Tree',
    '',
    `Files under \`tree/\`, in the order the cap let them in: the slices above, then the files the change touched, then the files that import them, then the rest of the tree. A file goes in windows of ${String(limits.window_lines)} lines from its first line. A file cut short holds the lines listed here at their own numbers; a run of lines it does not hold is blank but for a marker on its first line. A file of the rest of the tree not listed here was sent in full.`,
    '',
    ...tree,
    '',
  ].join('\n')
}
