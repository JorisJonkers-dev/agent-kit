import type { Category, FindingBody, HintCode, PreVerdict } from './findings.js'
import type { PacketFile } from './manifest.js'
import type { CritiqueStage } from './stage.js'

/** One packet file's text, as it was frozen before any critic ran. */
export interface PacketText {
  readonly path: string
  readonly kind: PacketFile['kind']
  readonly text: string
}

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
 * sees it. Every quote must occur in the packet, or the finding is rejected
 * as fabricated evidence. Then the stage's evidence rule decides whether the
 * finding has anything checkable; if not, it is a hint. Returns nothing when
 * the orchestrator must adjudicate the finding itself.
 */
export function checkEvidence(
  finding: FindingBody,
  stage: CritiqueStage,
  packet: readonly PacketText[],
): PreVerdict | undefined {
  const files = packet.map((file) => ({ ...file, text: normalise(file.text) }))
  const found = { decision: false, items: finding.evidence.length, scenario: false, spec: false }
  for (const [index, evidence] of finding.evidence.entries()) {
    if (evidence.kind !== 'quote') {
      found.scenario = true
      continue
    }
    const quote = normalise(evidence.value)
    const holders = files.filter((file) => file.text.includes(quote))
    if (holders.length === 0) {
      return {
        reason: `evidence ${String(index + 1)} quotes text that is not in the packet`,
        reason_code: 'fabricated-evidence',
        verdict: 'rejected',
      }
    }
    found.spec ||= holders.some((file) => file.kind === 'artifact')
    found.decision ||= holders.some((file) => file.kind === 'reference')
  }
  const hint = found.items === 0 ? 'no-evidence' : stage === 'spec' ? specRule(finding.category, found) : undefined
  return hint === undefined ? undefined : { reason: HINT_REASONS[hint], reason_code: hint, verdict: 'hint' }
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
}

/** Runs of white space count as one space, so a quote may cross a line break. */
function normalise(text: string): string {
  return text.replace(/\s+/g, ' ').trim()
}
