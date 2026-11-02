import { JsonFields } from './json.js'
import { lineCount } from './line-references.js'
import { refusal, type Refusal } from './outcomes.js'
import { matchesGlob } from './triage.js'

/** Why a tracked path is never sent: it is vendored, generated, or a lock file. */
export const EXCLUSION_KINDS = ['vendored', 'generated', 'lock'] as const
export type ExclusionKind = (typeof EXCLUSION_KINDS)[number]

/**
 * Routing's `packet`: how big a repository packet may be. The cap counts
 * every packet file; the facts cap counts the computed fact files, beside
 * `facts/INDEX.md`, which is always sent; a sampled file is sent in windows
 * of `window_lines` lines. `exclude` holds the globs of the paths never sent.
 */
export interface PacketLimits {
  readonly cap_bytes: number
  readonly facts_cap_bytes: number
  readonly window_lines: number
  readonly exclude: Readonly<Record<ExclusionKind, readonly string[]>>
}

/** ADR-0004's starting limits, for a routing file that states none; the phase-0 live check sets the final ones in routing. */
export const DEFAULT_PACKET_LIMITS: PacketLimits = {
  cap_bytes: 153_600,
  exclude: { generated: [], lock: [], vendored: [] },
  facts_cap_bytes: 30_720,
  window_lines: 400,
}

export function readPacketLimits(fields: JsonFields): PacketLimits {
  const exclude = fields.object('exclude')
  return {
    cap_bytes: fields.positiveInteger('cap_bytes'),
    exclude: { generated: exclude.strings('generated'), lock: exclude.strings('lock'), vendored: exclude.strings('vendored') },
    facts_cap_bytes: fields.positiveInteger('facts_cap_bytes'),
    window_lines: fields.positiveInteger('window_lines'),
  }
}

/** The kind of the first exclusion glob a path matches, or nothing when it may be sent. */
export function excludedByPath(path: string, limits: PacketLimits): ExclusionKind | undefined {
  return EXCLUSION_KINDS.find((kind) => limits.exclude[kind].some((glob) => matchesGlob(path, glob)))
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
}

/** A tree file, as frozen: all of it, or its first lines when the cap cut it short. */
export interface SampledFile {
  readonly path: string
  readonly text: string
  /** Only when the file was cut short: it holds lines 1 to `last` of its `lines`. */
  readonly window?: { readonly last: number; readonly lines: number }
}

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
const TIERS = ['named', 'hotspot', 'entry', 'cycle'] as const
type Tier = (typeof TIERS)[number]

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

interface Candidate {
  readonly path: string
  readonly tier: Tier
  readonly lines: readonly string[]
  readonly lineCount: number
  readonly text: string
  /** How many windows the file is cut into; an empty file is one empty window. */
  readonly windows: number
}

const encoder = new TextEncoder()

function bytes(text: string): number {
  return encoder.encode(text).length
}

/**
 * Chooses what of the repository's tree a packet sends, by rule and never by
 * a model: the files the artifact names, then the top hotspots, then each
 * module's entry file, then the files on each import cycle, each file once,
 * in its first tier. Files are cut into windows of the routing's window
 * size; the windows go in tier by tier, each tier's first windows before its
 * second, while they fit under the cap beside the documents, the manifest,
 * the facts and the facts index. A file's windows always run from its first
 * line, so each line keeps its number, and a window that does not fit drops
 * the rest of its file. Every drop is recorded in the facts index. Refused
 * when the documents, the manifest and the facts alone pass the cap.
 */
export function sampleRepository(request: SampleRequest): RepositorySample | Refusal {
  const { limits } = request
  const manifest = renderManifest(request)
  const facts = admitFacts(request.facts.files, limits.facts_cap_bytes)
  const candidates = sampleCandidates(request)
  const sent = new Map<Candidate, number>(candidates.map((candidate) => [candidate, 0]))
  const index = (): string => renderIndex(request, facts, candidates, sent)
  const fixed = request.documentBytes + bytes(manifest) + facts.reduce((sum, file) => sum + bytes(file.text), 0) + bytes(index())
  if (fixed > limits.cap_bytes) {
    return refusal(
      'packet-over-cap',
      `the documents, ${MANIFEST_PATH} and facts/ come to ${String(fixed)} bytes, past the packet cap of ${String(limits.cap_bytes)}; review fewer records or raise packet.cap_bytes in routing`,
    )
  }
  let remaining = limits.cap_bytes - fixed
  const blocked = new Set<Candidate>()
  for (const { candidate, window } of windowOrder(candidates)) {
    if (blocked.has(candidate)) continue
    const cost =
      bytes(prefix(candidate, window, limits)) -
      bytes(prefix(candidate, window - 1, limits)) +
      bytes(sampleLine(candidate, window, limits)) -
      bytes(sampleLine(candidate, window - 1, limits))
    if (cost > remaining) {
      blocked.add(candidate)
      continue
    }
    remaining -= cost
    sent.set(candidate, window)
  }
  const tree = candidates.flatMap((candidate): SampledFile[] => {
    const windows = sent.get(candidate) ?? 0
    if (windows === 0) return []
    const text = prefix(candidate, windows, limits)
    const cut = windows < candidate.windows ? { window: { last: windows * limits.window_lines, lines: candidate.lineCount } } : {}
    return [{ path: candidate.path, text, ...cut }]
  })
  return {
    dropped: candidates.filter((candidate) => (sent.get(candidate) ?? 0) < candidate.windows).length,
    facts,
    index: index(),
    manifest,
    status: 'sampled',
    tree,
  }
}

/** Every candidate's windows, tier by tier, each tier's first windows before its second. */
function windowOrder(candidates: readonly Candidate[]): readonly { readonly candidate: Candidate; readonly window: number }[] {
  const order = candidates.flatMap((candidate, position) =>
    Array.from({ length: candidate.windows }, (_, i) => ({ candidate, position, window: i + 1 })),
  )
  return order.sort(
    (a, b) => TIERS.indexOf(a.candidate.tier) - TIERS.indexOf(b.candidate.tier) || a.window - b.window || a.position - b.position,
  )
}

/** The file's first `windows` windows, as frozen: the whole text once every window is in. */
function prefix(candidate: Candidate, windows: number, limits: PacketLimits): string {
  if (windows === 0) return ''
  if (windows === candidate.windows) return candidate.text
  return `${candidate.lines.slice(0, windows * limits.window_lines).join('\n')}\n`
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
function sampleCandidates(request: SampleRequest): readonly Candidate[] {
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
  }
  const seen = new Set<string>()
  return TIERS.flatMap((tier) =>
    tiers[tier].flatMap((path): Candidate[] => {
      const text = texts.get(path)
      if (text === undefined || seen.has(path)) return []
      seen.add(path)
      const count = lineCount(text)
      return [{ lineCount: count, lines: text.split('\n'), path, text, tier, windows: Math.max(1, Math.ceil(count / request.limits.window_lines)) }]
    }),
  )
}

/** The path-like words of a text, in the order they first appear, without a `./` or `tree/` prefix or a sentence's full stop. */
function namedPaths(text: string): readonly string[] {
  return (text.match(/[\w@+.\-/]+/g) ?? []).map((word) => word.replace(/^(?:\.\/|tree\/)/, '').replace(/\.+$/, ''))
}

function renderManifest(request: SampleRequest): string {
  const rank = (path: string) => String(request.facts.churn_ranks?.get(path) ?? '-')
  const rows = request.tracked.map((file) => `${file.path}\t${file.text === null ? '-' : String(lineCount(file.text))}\t${rank(file.path)}`)
  return `${['path\tlines\tchurn_rank', ...rows].join('\n')}\n`
}

/** What became of one sampled file, after `windows` of its windows went in. */
function sampleLine(candidate: Candidate, windows: number, limits: PacketLimits): string {
  const head = `- \`${candidate.path}\` (${candidate.tier}, ${String(candidate.lineCount)} lines): `
  if (windows === 0) return `${head}the cap dropped it`
  if (windows === candidate.windows) return `${head}sent in full`
  const last = windows * limits.window_lines
  return `${head}sent lines 1-${String(last)}; the cap dropped lines ${String(last + 1)}-${String(candidate.lineCount)}`
}

function renderIndex(
  request: SampleRequest,
  facts: readonly FactFile[],
  candidates: readonly Candidate[],
  sent: ReadonlyMap<Candidate, number>,
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
  const samples = candidates.map((candidate) => sampleLine(candidate, sent.get(candidate) ?? 0, limits))
  return [
    '# Facts',
    '',
    `What the runner computed from the repository at commit \`${request.commit}\`. Facts are settled: cite them, never recompute them. A quote from a fact is a fact, never a recorded decision.`,
    '',
    '## Files',
    '',
    `- \`${MANIFEST_PATH}\`: every tracked path but the excluded ones, with its line count (\`-\` for a binary file) and its churn rank (\`-\` while no churn facts exist). Columns: \`path\`, \`lines\`, \`churn_rank\`.`,
    ...factLines,
    ...(factLines.length === 0 ? ['', 'No other facts are computed yet.'] : []),
    '',
    '## Limits',
    '',
    `Packet cap ${String(limits.cap_bytes)} bytes; facts cap ${String(limits.facts_cap_bytes)} bytes; windows of ${String(limits.window_lines)} lines.`,
    '',
    '## Excluded',
    '',
    `Never sent, and not in \`${MANIFEST_PATH}\`: ${String(vendored)} vendored, ${String(generated)} generated and ${String(lock)} lock files.`,
    '',
    '## Sample',
    '',
    `Files sent under \`tree/\`, in the order they were chosen: the files the artifact names, the top hotspots, each module's entry file, then the files on each import cycle. A longer file is sent in windows of ${String(limits.window_lines)} lines from its first line, at its own line numbers. What the cap left out is listed here.`,
    '',
    ...skipped,
    ...(skipped.length === 0 ? [] : ['']),
    ...(samples.length === 0 ? ['Nothing was sampled.'] : samples),
    '',
  ].join('\n')
}
