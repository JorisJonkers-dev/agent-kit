import type { Category, Evidence, FindingBody, HintCode, PreVerdict } from './findings.js'
import { lineCount, lineReferences, packetCandidates } from './line-references.js'
import type { PacketFile } from './manifest.js'
import { isCodeStage, type CritiqueStage } from './stage.js'

/** One packet file's text, as it was frozen before any critic ran. */
export interface PacketText {
  readonly path: string
  readonly kind: PacketFile['kind']
  readonly text: string
}

/** The packet as the evidence check reads it, built once for every finding of a run. */
export interface PacketIndex {
  readonly files: readonly (PacketText & { readonly normalised: string })[]
  /** How many lines each packet file has, by packet path. */
  readonly lines: ReadonlyMap<string, number>
}

export function indexPacket(packet: readonly PacketText[]): PacketIndex {
  return {
    files: packet.map((file) => ({ ...file, normalised: normalise(file.text) })),
    lines: new Map(packet.map((file) => [file.path, lineCount(file.text)])),
  }
}

/** The packet folder a code packet keeps its tree in. */
const TREE_DIR = 'tree'

/** What a finding's evidence holds, once each quote is found in the packet. */
interface EvidenceFound {
  readonly items: number
  /** A quote found in the artifact. */
  readonly spec: boolean
  /** A quote found in a reference: the decision source. */
  readonly decision: boolean
  /** An input, failing test or trace: a concrete scenario. */
  readonly scenario: boolean
}

/** Categories that claim the spec and its decisions disagree about what exists. */
const DECISION_CATEGORIES: readonly Category[] = ['omission', 'invention']

/**
 * The mechanical check fan-out runs on every finding before the orchestrator
 * sees it. Every quote must occur in the packet, and at a code stage every
 * file and line the location or a trace step cites must be in the packet's
 * frozen copy, or the finding is rejected as fabricated evidence. Then the
 * stage's evidence rule decides whether the finding has anything checkable;
 * if not, it is a hint. Returns nothing when the orchestrator must
 * adjudicate the finding itself.
 */
export function checkEvidence(finding: FindingBody, stage: CritiqueStage, packet: PacketIndex): PreVerdict | undefined {
  const found = { decision: false, items: finding.evidence.length, scenario: false, spec: false }
  for (const [index, evidence] of finding.evidence.entries()) {
    if (evidence.kind !== 'quote') {
      found.scenario = true
      continue
    }
    const quote = normalise(evidence.value)
    const holders = packet.files.filter((file) => file.normalised.includes(quote))
    if (holders.length === 0) return fabricated(`evidence ${String(index + 1)} quotes text that is not in the packet`)
    found.spec ||= holders.some((file) => file.kind === 'artifact')
    found.decision ||= holders.some((file) => file.kind === 'reference')
  }
  if (isCodeStage(stage)) {
    const missing = missingLine(finding, packet)
    if (missing !== undefined) return fabricated(missing)
  }
  const hint =
    found.items === 0
      ? 'no-evidence'
      : stage === 'spec'
        ? specRule(finding.category, found)
        : isCodeStage(stage)
          ? codeRule(finding.evidence)
          : undefined
  return hint === undefined ? undefined : { reason: HINT_REASONS[hint], reason_code: hint, verdict: 'hint' }
}

function fabricated(reason: string): PreVerdict {
  return { reason, reason_code: 'fabricated-evidence', verdict: 'rejected' }
}

/**
 * Why the first file-and-line citation of the location, or of a trace step,
 * names a file the packet does not hold or a line its file does not have.
 */
function missingLine(finding: FindingBody, packet: PacketIndex): string | undefined {
  const sources = [
    { text: finding.location, where: 'the location' },
    ...finding.evidence.flatMap((evidence, index) =>
      evidence.kind === 'trace' ? [{ text: evidence.value, where: `evidence ${String(index + 1)}` }] : [],
    ),
  ]
  for (const { text, where } of sources) {
    for (const reference of lineReferences(text)) {
      const path = packetCandidates(reference.path, TREE_DIR).find((candidate) => packet.lines.has(candidate))
      if (path === undefined) return `${where} cites ${reference.path}, which is not in the packet`
      const lines = packet.lines.get(path) ?? 0
      const outside = [reference.first, reference.last].find((line) => line < 1 || line > lines)
      if (outside !== undefined) return `${where} cites line ${String(outside)} of ${path}, which has ${String(lines)} lines`
    }
  }
  return undefined
}

/** #181's code-stage rule: a code finding needs a failing test, or a trace that cites a file and line. */
function codeRule(evidence: readonly Evidence[]): HintCode | undefined {
  const checkable = evidence.some(
    (item) => item.kind === 'failing-test' || (item.kind === 'trace' && lineReferences(item.value).length > 0),
  )
  return checkable ? undefined : 'no-test-or-trace'
}

/**
 * #167's spec-stage rule: an omission or invention needs a quote from the
 * spec and one from the decision source; any other finding needs a spec
 * quote and either a decision quote or a concrete scenario.
 */
function specRule(category: Category, found: EvidenceFound): HintCode | undefined {
  if (!found.spec) return 'no-spec-quote'
  if (DECISION_CATEGORIES.includes(category)) return found.decision ? undefined : 'no-decision-quote'
  return found.decision || found.scenario ? undefined : 'no-decision-or-scenario'
}

const HINT_REASONS: Readonly<Record<HintCode, string>> = {
  'no-decision-or-scenario': 'no quote from a decision source and no concrete scenario',
  'no-decision-quote': 'no quote from a decision source',
  'no-evidence': 'no evidence to check',
  'no-spec-quote': 'no quote from the spec',
  'no-test-or-trace': 'no failing test and no trace with a file and line',
}

/** Runs of white space count as one space, so a quote may cross a line break. */
function normalise(text: string): string {
  return text.replace(/\s+/g, ' ').trim()
}
