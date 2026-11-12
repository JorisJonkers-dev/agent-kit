import { capDrop, dropSummaryBound, possibleDrops, summariseDrops } from './drop-summary.js'
import { JsonFields } from './json.js'
import { lineCount } from './line-references.js'
import { refusal, type Refusal } from './outcomes.js'
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
import { matchesGlob } from './triage.js'

/** Why a tracked path is never sent: it is vendored, generated, or a lock file. */
export const EXCLUSION_KINDS = ['vendored', 'generated', 'lock'] as const
export type ExclusionKind = (typeof EXCLUSION_KINDS)[number]

/**
 * Routing's `packet`: how big a repository packet may be. The cap counts
 * every packet file; the facts cap counts the computed fact files, beside
 * `facts/INDEX.md`, which is always sent; a sampled file is sent in windows
 * of `window_lines` lines. `exclude` holds the globs of the paths never sent.
 * Past `manifest_cap_bytes`, `tree/MANIFEST.tsv` counts its least-ranked
 * paths per directory instead of listing them.
 */
export interface PacketLimits {
  readonly cap_bytes: number
  readonly facts_cap_bytes: number
  /** How big a repository packet's `tree/MANIFEST.tsv` may be before it counts paths per directory. */
  readonly manifest_cap_bytes: number
  /** How big a code packet's `tree/REPO-MAP.md` may be, inside the cap. */
  readonly repo_map_cap_bytes: number
  readonly window_lines: number
  readonly exclude: Readonly<Record<ExclusionKind, readonly string[]>>
}

/** The repo map's cap for a routing file that states none. */
const DEFAULT_REPO_MAP_CAP_BYTES = 20_480
/** The manifest's cap for a routing file that states none. */
const DEFAULT_MANIFEST_CAP_BYTES = 30_720

/** ADR-0004's starting limits, for a routing file that states none; the phase-0 live check sets the final ones in routing. */
export const DEFAULT_PACKET_LIMITS: PacketLimits = {
  cap_bytes: 153_600,
  exclude: { generated: [], lock: [], vendored: [] },
  facts_cap_bytes: 30_720,
  manifest_cap_bytes: DEFAULT_MANIFEST_CAP_BYTES,
  repo_map_cap_bytes: DEFAULT_REPO_MAP_CAP_BYTES,
  window_lines: 400,
}

export function readPacketLimits(fields: JsonFields): PacketLimits {
  const exclude = fields.object('exclude')
  return {
    cap_bytes: fields.positiveInteger('cap_bytes'),
    exclude: { generated: exclude.strings('generated'), lock: exclude.strings('lock'), vendored: exclude.strings('vendored') },
    facts_cap_bytes: fields.positiveInteger('facts_cap_bytes'),
    manifest_cap_bytes: fields.has('manifest_cap_bytes') ? fields.positiveInteger('manifest_cap_bytes') : DEFAULT_MANIFEST_CAP_BYTES,
    repo_map_cap_bytes: fields.has('repo_map_cap_bytes') ? fields.positiveInteger('repo_map_cap_bytes') : DEFAULT_REPO_MAP_CAP_BYTES,
    window_lines: fields.positiveInteger('window_lines'),
  }
}

/** Why a path is never sent: its kind, and the rule that says so. */
export interface Exclusion {
  readonly kind: ExclusionKind
  /** The routing glob it matches, or `marker` when its first lines say a tool generated it. */
  readonly rule: string
}

/** The kind of the first exclusion glob a path matches, or nothing when it may be sent. */
export function excludedByPath(path: string, limits: PacketLimits): ExclusionKind | undefined {
  return exclusionOf(path, undefined, limits)?.kind
}

/**
 * Why a path, with its text when it is known, is never sent: the first
 * exclusion glob it matches, else a generated-file marker in its first lines.
 */
export function exclusionOf(path: string, text: string | undefined, limits: PacketLimits): Exclusion | undefined {
  for (const kind of EXCLUSION_KINDS) {
    const glob = limits.exclude[kind].find((candidate) => matchesGlob(path, candidate))
    if (glob !== undefined) return { kind, rule: glob }
  }
  return text !== undefined && isGenerated(text) ? { kind: 'generated', rule: 'marker' } : undefined
}

/** What an exclusion says about its file, for a line of the diff or the facts index. */
export function exclusionReason(exclusion: Exclusion): string {
  return exclusion.rule === 'marker'
    ? 'a generated file: its first lines say a tool generated it'
    : `a ${exclusion.kind} file: it matches the routing glob \`${exclusion.rule}\``
}

/** How many leading lines may carry a generated-file marker. */
const MARKER_LINES = 5
const GENERATED_MARKER = /@generated|do not edit/i

/** Whether a file says in its first lines that a tool generated it (`@generated`, "Do not edit"). */
export function isGenerated(text: string): boolean {
  return GENERATED_MARKER.test(text.split('\n', MARKER_LINES).join('\n'))
}

/** One file the runner computed into `facts/`, and what it holds. */
export interface FactFile {
  readonly name: string
  readonly holds: string
  readonly text: string
}

/**
 * What prepare knows about the repository beyond its tree. The scans that
 * compute facts (#232, #233) fill this in; until a scan has run, its field
 * is undefined and the sample tier that needs it is skipped.
 */
export interface RepositoryFacts {
  readonly files: readonly FactFile[]
  /** Paths, hottest first. */
  readonly hotspots?: readonly string[]
  /** The files of each import cycle. */
  readonly cycles?: readonly (readonly string[])[]
  /** Each path's rank by churn, 1 the most changed. */
  readonly churn_ranks?: ReadonlyMap<string, number>
}

export const NO_FACTS: RepositoryFacts = { files: [] }

/** A tracked path that is not excluded, with its text, or null for a binary file. */
export interface TrackedFile {
  readonly path: string
  readonly text: string | null
}

export interface SampleRequest {
  readonly commit: string
  readonly limits: PacketLimits
  /** Every tracked path not excluded, sorted. */
  readonly tracked: readonly TrackedFile[]
  /** How many tracked paths each exclusion left out. */
  readonly excluded: Readonly<Record<ExclusionKind, number>>
  /** The repository paths already sent as documents, which the tree never repeats. */
  readonly documents: readonly string[]
  /** The bytes the documents take. */
  readonly documentBytes: number
  /** The text that names files: the artifact's. */
  readonly artifactText: string
  readonly facts: RepositoryFacts
  /**
   * Paths sampled after every other tier, in this order: for a stage with no
   * artifact to name files, the source the sample would otherwise miss.
   */
  readonly rest?: readonly string[]
  /** What code-nav's repository map did to the sample's order: a sentence for the facts index; absent when no map was asked for. */
  readonly rankingNote?: string
}


export type { SampledFile } from './tree-sample.js'

export interface RepositorySample {
  readonly status: 'sampled'
  readonly manifest: string
  readonly index: string
  readonly facts: readonly FactFile[]
  readonly tree: readonly SampledFile[]
  /** How many sampled files the cap left out in whole or in part. */
  readonly dropped: number
}

/** Why a file was sampled, in priority order. */
const TIERS = ['named', 'hotspot', 'entry', 'cycle', 'rest'] as const
type Tier = (typeof TIERS)[number]
/** The tiers whose files the facts index lists by name; it counts the others' drops per directory. */
const LISTED_TIERS: ReadonlySet<Tier> = new Set(['named', 'hotspot'])

/** The file names that make a file its module's entry or interface. */
const ENTRY_FILES = new Set([
  'index.ts',
  'index.tsx',
  'index.js',
  'index.jsx',
  'index.mjs',
  'index.cjs',
  'main.ts',
  '__init__.py',
  '__main__.py',
  'main.py',
  'mod.rs',
  'lib.rs',
  'main.rs',
  'main.go',
  'Main.kt',
  'main.kt',
  'Main.java',
])

/** The packet path of the tree's manifest. */
export const MANIFEST_PATH = 'tree/MANIFEST.tsv'
export const FACTS_INDEX_PATH = 'facts/INDEX.md'

/**
 * Chooses what of the repository's tree a packet sends, by rule and never by
 * a model: the files the artifact names, then the top hotspots, then each
 * module's entry file, then the files on each import cycle, each file once,
 * in its first tier. Files are cut into windows of the routing's window
 * size; the windows go in tier by tier, each tier's first windows before its
 * second, while they fit under the cap beside the documents, the manifest,
 * the facts and the facts index. A file's windows always run from its first
 * line, so each line keeps its number, and a window that does not fit drops
 * the rest of its file. Every drop is recorded in the facts index: by name
 * for a named file or a hotspot, counted per directory for an entry file or
 * a file on a cycle, under a share of the cap reserved for the most that
 * count can take. Refused when the documents, the manifest and the facts
 * alone pass the cap.
 */
export function sampleRepository(request: SampleRequest): RepositorySample | Refusal {
  const { limits } = request
  const facts = admitFacts(request.facts.files, limits.facts_cap_bytes)
  const candidates = sampleCandidates(request)
  const manifest = renderManifest(request, new Set(candidates.map((candidate) => candidate.path)))
  const counted = candidates.filter((candidate) => !LISTED_TIERS.has(candidate.tier))
  const reserved = dropSummaryBound(counted.flatMap((candidate) => possibleDrops(candidate, limits.window_lines)))
  const none = new Map<TreeCandidate<Tier>, readonly LineRange[]>()
  const fixed =
    request.documentBytes +
    bytes(manifest.text) +
    facts.reduce((sum, file) => sum + bytes(file.text), 0) +
    bytes(renderIndex(request, manifest, facts, candidates, none, [])) +
    reserved
  if (fixed > limits.cap_bytes) {
    return refusal(
      'packet-over-cap',
      `the documents, ${MANIFEST_PATH} and facts/ come to ${String(fixed)} bytes, past the packet cap of ${String(limits.cap_bytes)}; review fewer records or raise packet.cap_bytes in routing`,
    )
  }
  const { held } = admitPieces(windowOrder(candidates, limits.window_lines), limits.cap_bytes - fixed, (candidate, lines) =>
    LISTED_TIERS.has(candidate.tier) ? sampleLine(candidate, lines) : undefined,
  )
  const drops = summariseDrops(counted.flatMap((candidate) => capDrop(candidate, held.get(candidate) ?? [], limits.window_lines) ?? []))
  const tree = candidates.flatMap((candidate) => {
    const file = sampledFile(candidate, held.get(candidate) ?? [])
    return file === undefined ? [] : [file]
  })
  return {
    dropped: candidates.filter((candidate) => !holdsAll(candidate, held.get(candidate) ?? [])).length,
    facts,
    index: renderIndex(request, manifest, facts, candidates, held, drops),
    manifest: manifest.text,
    status: 'sampled',
    tree,
  }
}

/** Every candidate's windows, tier by tier, each tier's first windows before its second. */
function windowOrder(candidates: readonly TreeCandidate<Tier>[], size: number): readonly TreePiece<Tier>[] {
  const order = candidates.flatMap((candidate, position) => windowsOf(candidate, size).map((piece, window) => ({ piece, position, window })))
  return order
    .sort((a, b) => TIERS.indexOf(a.piece.candidate.tier) - TIERS.indexOf(b.piece.candidate.tier) || a.window - b.window || a.position - b.position)
    .map((entry) => entry.piece)
}

/** The fact files, in order, that fit under the facts cap. */
function admitFacts(files: readonly FactFile[], cap: number): readonly FactFile[] {
  let remaining = cap
  return files.filter((file) => {
    const size = bytes(file.text)
    if (size > remaining) return false
    remaining -= size
    return true
  })
}

/** The files each tier samples, each file once, in its first tier. */
function sampleCandidates(request: SampleRequest): readonly TreeCandidate<Tier>[] {
  const documents = new Set(request.documents)
  const texts = new Map(
    request.tracked.flatMap((file) =>
      file.text === null || documents.has(file.path) || file.path === 'MANIFEST.tsv' ? [] : [[file.path, file.text] as const],
    ),
  )
  const tiers: Readonly<Record<Tier, readonly string[]>> = {
    cycle: request.facts.cycles?.flat() ?? [],
    entry: [...texts.keys()].filter((path) => ENTRY_FILES.has(path.split('/').at(-1) ?? '')),
    hotspot: request.facts.hotspots ?? [],
    named: namedPaths(request.artifactText),
    rest: request.rest ?? [],
  }
  const seen = new Set<string>()
  return TIERS.flatMap((tier) =>
    tiers[tier].flatMap((path): TreeCandidate<Tier>[] => {
      const text = texts.get(path)
      if (text === undefined || seen.has(path)) return []
      seen.add(path)
      return [candidateOf(path, tier, text)]
    }),
  )
}

/** The path-like words of a text, in the order they first appear, without a `./` or `tree/` prefix or a sentence's full stop. */
export function namedPaths(text: string): readonly string[] {
  return (text.match(/[\w@+.\-/]+/g) ?? []).map((word) => word.replace(/^(?:\.\/|tree\/)/, '').replace(/\.+$/, ''))
}

const MANIFEST_HEADER = 'path\tlines\tchurn_rank'

/** The manifest's text, and how many of its least-ranked paths it counts per directory instead of listing. */
interface RenderedManifest {
  readonly text: string
  readonly collapsed: number
  /** Whether it counts the collapsed paths per directory; false when even the counts would pass its cap. */
  readonly counted: boolean
}

/**
 * Every tracked path with its line count and churn rank, in path order.
 * Past the manifest cap, its least-ranked paths are counted per directory
 * at its end instead, one at a time until it fits; when even every path
 * counted does not fit, the counts are left out too and the manifest says
 * so. The paths go in this order: a path the sample chose
 * last, then those without a churn rank before the least churned, then the
 * deepest, then the last in path order.
 */
function renderManifest(request: SampleRequest, sampled: ReadonlySet<string>): RenderedManifest {
  const churn = request.facts.churn_ranks
  const rows = new Map(
    request.tracked.map((file) => [file.path, `${file.path}\t${file.text === null ? '-' : String(lineCount(file.text))}\t${String(churn?.get(file.path) ?? '-')}`]),
  )
  const cap = request.limits.manifest_cap_bytes
  let rowBytes = lineBytes(MANIFEST_HEADER) + [...rows.values()].reduce((sum, row) => sum + lineBytes(row), 0)
  let countBytes = 0
  let collapsed = 0
  const counts = new Map<string, number>()
  const rank = (path: string) => [sampled.has(path) ? 1 : 0, -(churn?.get(path) ?? Infinity), -path.split('/').length] as const
  const order = [...rows.keys()].sort((a, b) => {
    const [x, y] = [rank(a), rank(b)]
    return x[0] - y[0] || x[1] - y[1] || x[2] - y[2] || (a < b ? 1 : -1)
  })
  for (const path of order) {
    if (rowBytes + countBytes + (collapsed === 0 ? 0 : lineBytes(collapsedLine(collapsed, cap))) <= cap) break
    const directory = path.includes('/') ? `${path.slice(0, path.lastIndexOf('/'))}/` : './'
    const count = counts.get(directory) ?? 0
    countBytes += lineBytes(countLine(directory, count + 1)) - (count === 0 ? 0 : lineBytes(countLine(directory, count)))
    rowBytes -= lineBytes(rows.get(path))
    counts.set(directory, count + 1)
    rows.delete(path)
    collapsed += 1
  }
  const counted = rowBytes + countBytes + (collapsed === 0 ? 0 : lineBytes(collapsedLine(collapsed, cap))) <= cap
  const tail =
    collapsed === 0
      ? []
      : counted
        ? [collapsedLine(collapsed, cap), ...[...counts].sort(([a], [b]) => (a < b ? -1 : 1)).map(([directory, count]) => countLine(directory, count))]
        : [uncountedLine(collapsed, cap)]
  return { collapsed, counted, text: `${[MANIFEST_HEADER, ...rows.values(), ...tail].join('\n')}\n` }
}

function collapsedLine(collapsed: number, cap: number): string {
  return `# collapsed: past the manifest cap of ${String(cap)} bytes, its ${String(collapsed)} least-ranked paths are counted per directory below, as directory and paths`
}

/** The collapsed line when even the per-directory counts would pass the cap, so they are left out too. */
function uncountedLine(collapsed: number, cap: number): string {
  return `# collapsed: past the manifest cap of ${String(cap)} bytes, its ${String(collapsed)} least-ranked paths are left out, and so are their per-directory counts, which would not fit`
}

function countLine(directory: string, count: number): string {
  return `# ${directory}\t${String(count)}`
}

/** What became of one sampled file, holding these lines. */
function sampleLine(candidate: TreeCandidate<Tier>, held: readonly LineRange[]): string {
  return `- \`${candidate.path}\` (${candidate.tier}, ${String(candidate.lineCount)} lines): ${sentStatus(candidate, held)}`
}

function renderIndex(
  request: SampleRequest,
  manifest: RenderedManifest,
  facts: readonly FactFile[],
  candidates: readonly TreeCandidate<Tier>[],
  held: ReadonlyMap<TreeCandidate<Tier>, readonly LineRange[]>,
  drops: readonly string[],
): string {
  const { limits } = request
  const admitted = new Set(facts)
  const factLines = request.facts.files.map(
    (file) => `- \`facts/${file.name}\`: ${file.holds}${admitted.has(file) ? '' : '; the facts cap dropped it'}`,
  )
  const { generated, lock, vendored } = request.excluded
  const skipped = [
    ...(request.facts.hotspots === undefined ? ['No hotspot facts yet, so no hotspot was sampled.'] : []),
    ...(request.facts.cycles === undefined ? ['No import facts yet, so no import cycle was sampled.'] : []),
  ]
  const samples = candidates.flatMap((candidate) => (LISTED_TIERS.has(candidate.tier) ? [sampleLine(candidate, held.get(candidate) ?? [])] : []))
  return [
    '# Facts',
    '',
    `What the runner computed from the repository at commit \`${request.commit}\`. Facts are settled: cite them, never recompute them. A quote from a fact is a fact, never a recorded decision.`,
    '',
    '## Files',
    '',
    `- \`${MANIFEST_PATH}\`: every tracked path but the excluded ones, with its line count (\`-\` for a binary file) and its churn rank (\`-\` while no churn facts exist). Columns: \`path\`, \`lines\`, \`churn_rank\`.${
      manifest.collapsed === 0
        ? ''
        : ` Past its cap of ${String(request.limits.manifest_cap_bytes)} bytes, it ${manifest.counted ? 'counts' : 'leaves out'} its ${String(manifest.collapsed)} least-ranked paths${manifest.counted ? ' per directory at its end instead of listing them' : ', and their per-directory counts, which would not fit'}.`
    }`,
    ...factLines,
    ...(factLines.length === 0 ? ['', 'No other facts are computed yet.'] : []),
    '',
    '## Limits',
    '',
    `Packet cap ${String(limits.cap_bytes)} bytes; facts cap ${String(limits.facts_cap_bytes)} bytes; windows of ${String(limits.window_lines)} lines.`,
    '',
    ...(request.rankingNote === undefined ? [] : ['## Ranking', '', request.rankingNote, '']),
    '## Excluded',
    '',
    `Never sent, and not in \`${MANIFEST_PATH}\`: ${String(vendored)} vendored, ${String(generated)} generated and ${String(lock)} lock files.`,
    '',
    '## Sample',
    '',
    `Files sent under \`tree/\`, in the order they were chosen: the files the artifact names, the top hotspots, each module's entry file, then the files on each import cycle. A longer file is sent in windows of ${String(limits.window_lines)} lines from its first line, at its own line numbers. The files the artifact names and the top hotspots are listed by name; the cap's drops of entry files and files on a cycle are counted per directory with the bytes left out (\`dir/**\` counts every file under it), and one neither listed nor counted was sent in full.`,
    '',
    ...skipped,
    ...(skipped.length === 0 ? [] : ['']),
    ...(candidates.length === 0 ? ['Nothing was sampled.'] : [...samples, ...drops]),
    '',
  ].join('\n')
}
