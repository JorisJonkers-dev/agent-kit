import { NEEDS_TREE, type DeclaredCheck } from './checks.js'
import type { Category, Evidence, FindingBody, HintCode, MechanicalRejection, PreVerdict } from './findings.js'
import type { JsonFields } from './json.js'
import { lineCount, lineReferences, packetCandidates, type LineReference } from './line-references.js'
import { diffShownLines } from './diff-window.js'
import { heldRanges, type PacketFile, type PacketWindow } from './manifest.js'
import { mergeRanges, type LineRange } from './tree-sample.js'
import { isCodeStage, type CritiqueStage } from './stage.js'
import { evidenceRule, type EvidenceRule } from './stage-table.js'
import { ticketTarget } from './tickets.js'

/** One packet file's text, as it was frozen before any critic ran. */
export interface PacketText {
  readonly path: string
  readonly kind: PacketFile['kind']
  readonly text: string
  /** Only for a tree file a repository packet cut short. */
  readonly window?: PacketWindow
}

/** The packet as the evidence check reads it, built once for every finding of a run. */
export interface PacketIndex {
  readonly files: readonly (PacketText & { readonly normalised: string })[]
  /** How many lines each packet file has, by packet path. */
  readonly lines: ReadonlyMap<string, number>
  /** The tree files a repository packet cut short, by packet path. */
  readonly windows: ReadonlyMap<string, PacketWindow>
  /**
   * The lines at the head the diff's hunks show, by the packet path under
   * `tree/` they are cited as: a windowed diff shows only the hunks it sent.
   */
  readonly shown: ReadonlyMap<string, readonly LineRange[]>
  /** Whether the packet holds a tree, whose file and line citations are then checked at any stage. */
  readonly tree: boolean
  /** At the tickets stage, the ids of the spec's tickets, which a finding's location must name. */
  readonly tickets: readonly string[]
}

export function indexPacket(packet: readonly PacketText[], tickets: readonly string[] = []): PacketIndex {
  return {
    files: packet.map((file) => ({ ...file, normalised: normalise(file.text) })),
    lines: new Map(packet.map((file) => [file.path, lineCount(file.text)])),
    shown: new Map(
      packet.flatMap((file) => (file.kind === 'diff' ? [...diffShownLines(file.text)].map(([path, ranges]) => [`${TREE_DIR}/${path}`, ranges] as const) : [])),
    ),
    tickets,
    tree: packet.some((file) => file.kind === 'tree'),
    windows: new Map(packet.flatMap((file) => (file.window === undefined ? [] : [[file.path, file.window] as const]))),
  }
}

/** The packet folder a code packet keeps its tree in. */
const TREE_DIR = 'tree'

/** What a finding's evidence holds, once each quote is found in the packet. */
export interface EvidenceFound {
  readonly items: number
  /** A quote found in the artifact. */
  readonly spec: boolean
  /** A quote found in a reference: the decision source. */
  readonly decision: boolean
  /** A quote found in `history/`: a commit message or an issue. */
  readonly history: boolean
  /** A quote found in a fact the runner computed, which is never spec text or a decision. */
  readonly fact: boolean
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
 * stage's evidence rule, or the one the role of the critic that raised the
 * finding takes at that stage, decides whether the finding has anything checkable;
 * if not, it is a hint. Returns nothing when the orchestrator must
 * adjudicate the finding itself.
 */
export function checkEvidence(finding: FindingBody, stage: CritiqueStage, packet: PacketIndex, role?: string): PreVerdict | undefined {
  const found = evidenceFound(finding, packet)
  if (typeof found === 'number') return fabricated(`evidence ${String(found + 1)} quotes text that is not in the packet`)
  if (isCodeStage(stage) || packet.tree) {
    const missing = missingLine(finding, packet)
    if (missing !== undefined) return fabricated(missing)
  }
  const rule = evidenceRule(stage, role)
  // A candidate's rejections are mechanical, not hints.
  if (rule === 'recovery') return recoveryVerdict(finding, found, packet)
  const hint = found.items === 0 ? 'no-evidence' : EVIDENCE_RULES[rule](finding, found, packet)
  return hint === undefined ? undefined : { reason: HINT_REASONS[hint], reason_code: hint, verdict: 'hint' }
}

/**
 * Where each quote of a finding was found: a quote from the artifact is spec
 * text, one from a reference a decision, and one from a fact the runner
 * computed a fact, never either of the others. Returns the index of the
 * first quote no packet file holds.
 */
export function evidenceFound(finding: FindingBody, packet: PacketIndex): EvidenceFound | number {
  const found = { decision: false, fact: false, history: false, items: finding.evidence.length, scenario: false, spec: false }
  for (const [index, evidence] of finding.evidence.entries()) {
    if (evidence.kind !== 'quote') {
      found.scenario = true
      continue
    }
    const quote = normalise(evidence.value)
    const holders = packet.files.filter((file) => file.normalised.includes(quote))
    if (holders.length === 0) return index
    found.spec ||= holders.some((file) => file.kind === 'artifact')
    found.decision ||= holders.some((file) => file.kind === 'reference')
    found.fact ||= holders.some((file) => file.kind === 'fact')
    found.history ||= holders.some((file) => file.kind === 'history')
  }
  return found
}

/** Each evidence rule, as the stage table names it: the hint a finding earns, or nothing when it has something checkable. */
const EVIDENCE_RULES: Readonly<Record<Exclude<EvidenceRule, 'recovery'>, (finding: FindingBody, found: EvidenceFound, packet: PacketIndex) => HintCode | undefined>> = {
  adrs: adrRule,
  code: (finding) => codeRule(finding.evidence),
  decisions: (finding, found) => decisionsRule(finding.category, found),
  // A record finding quotes the record it is about, which is among the decisions under review.
  records: (_finding, found) => (found.spec ? undefined : 'no-record-quote'),
  spec: (finding, found) => specRule(finding.category, found),
  tickets: (finding, _found, packet) => ticketRule(finding.location, packet.tickets),
}

function fabricated(reason: string): PreVerdict {
  return { reason, reason_code: 'fabricated-evidence', verdict: 'rejected' }
}

/**
 * Why the first file-and-line citation of the location, or of a trace step,
 * names a file the packet does not hold or a line its file does not have.
 * A line a hunk of the diff shows is held, whether or not the tree holds it;
 * a line only a hunk the cap left out of the diff shows is not.
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
      const path = packetCandidates(reference.path, TREE_DIR).find((candidate) => packet.lines.has(candidate) || packet.shown.has(candidate))
      if (path === undefined) return `${where} cites ${reference.path}, which is not in the packet`
      const lines = packet.lines.get(path)
      const window = packet.windows.get(path)
      const inTree = lines === undefined ? [] : window === undefined ? [{ first: 1, last: lines }] : heldRanges(window)
      const held = mergeRanges([...inTree, ...(packet.shown.get(path) ?? [])])
      const outside = firstUnheld(reference, held)
      if (outside === undefined) continue
      return lines !== undefined && window === undefined
        ? `${where} cites line ${String(outside)} of ${path}, which has ${String(lines)} lines`
        : `${where} cites line ${String(outside)} of ${path}, outside the lines the packet holds (${held.map((range) => `${String(range.first)}-${String(range.last)}`).join(', ')}${window === undefined ? '' : ` of ${String(window.lines)}`})`
    }
  }
  return undefined
}

/**
 * The first line a citation names that the file does not hold: its first
 * line, its last, or for a range across a run of lines left out, the first
 * line of that run. Held ranges are merged, so a range is held only when one
 * of them holds both its ends.
 */
function firstUnheld(reference: LineReference, held: readonly { readonly first: number; readonly last: number }[]): number | undefined {
  const holding = (line: number) => held.find((range) => range.first <= line && line <= range.last)
  const first = holding(reference.first)
  if (first === undefined) return reference.first
  const last = holding(reference.last)
  if (last === undefined) return reference.last
  return first === last ? undefined : first.last + 1
}

/** A candidate that gives its reason says in these words when no commit message or issue states one. */
const NO_RATIONALE = /\bno rationale found\b/i

/**
 * The decision-recovery stage's rule: every finding is a candidate, an
 * `undocumented-decision`, and is rejected mechanically when it cites no
 * file of `tree/`, cites only one (a single site may be an accident), or
 * gives a "because" with neither a quote of `history/` nor the plain
 * statement "no rationale found". A candidate that clears all three has
 * evidence for the owner to confirm; nothing here accepts it.
 */
function recoveryVerdict(finding: FindingBody, found: EvidenceFound, packet: PacketIndex): PreVerdict | undefined {
  if (finding.category !== 'undocumented-decision') return { reason: HINT_REASONS['not-a-candidate'], reason_code: 'not-a-candidate', verdict: 'hint' }
  const sites = traceSites(finding.evidence, packet)
  if (sites.size === 0) return rejected('no-code-trace', 'cites no file of tree/ in a trace: the decision is not traced to the code')
  if (sites.size === 1) return rejected('single-site', `traces to one site only: one site may be an accident, so it needs two`)
  if (!found.history && !NO_RATIONALE.test(finding.claim)) {
    return rejected('invented-rationale', 'gives no reason from history/ and does not say that no rationale was found')
  }
  return undefined
}

function rejected(reasonCode: MechanicalRejection, reason: string): PreVerdict {
  return { reason, reason_code: reasonCode, verdict: 'rejected' }
}

/** The distinct files of `tree/` the trace steps of a finding cite, by packet path. */
function traceSites(evidence: readonly Evidence[], packet: PacketIndex): ReadonlySet<string> {
  const sites = new Set<string>()
  for (const item of evidence) {
    if (item.kind !== 'trace') continue
    for (const reference of lineReferences(item.value)) {
      const path = packetCandidates(reference.path, TREE_DIR).find((candidate) => packet.lines.has(candidate))
      if (path?.startsWith(`${TREE_DIR}/`) === true) sites.add(path)
    }
  }
  return sites
}

/** #181's code-stage rule: a code finding needs a failing test, or a trace that cites a file and line. */
function codeRule(evidence: readonly Evidence[]): HintCode | undefined {
  const checkable = evidence.some(
    (item) => item.kind === 'failing-test' || (item.kind === 'trace' && lineReferences(item.value).length > 0),
  )
  return checkable ? undefined : 'no-test-or-trace'
}

/**
 * #181's tickets-stage rule: a finding names the rule it breaks and the
 * ticket it is about, so the orchestrator can re-check that rule on that
 * ticket. An uncovered requirement is about the spec, so it needs no ticket.
 */
function ticketRule(location: string, tickets: readonly string[]): HintCode | undefined {
  const target = ticketTarget(location, tickets)
  if (target.rule === undefined) return 'no-ticket-rule'
  return target.rule === 'uncovered-requirement' || target.ticket !== undefined ? undefined : 'no-ticket-id'
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

/** What a decisions-stage finding may be: a branch never opened, or two decisions that clash. */
const GAP_CATEGORIES: readonly Category[] = ['unasked-question', 'contradiction']

/**
 * #181's decisions-stage rule: a gap is an unasked question or a
 * contradiction, quotes the decisions under review or the decision docs
 * beside them (never the feature statement alone), and gives a concrete
 * scenario.
 */
function decisionsRule(category: Category, found: EvidenceFound): HintCode | undefined {
  if (!GAP_CATEGORIES.includes(category)) return 'not-a-decision-gap'
  if (!found.spec && !found.decision) return 'no-decision-quote'
  return found.scenario ? undefined : 'no-scenario'
}

/**
 * The categories an evidence rule takes only with code under `tree/` to
 * check them against, and what each checks: a packet with no tree skips
 * them, and the report says so.
 */
const CODE_CATEGORIES: Readonly<Record<EvidenceRule, readonly { readonly category: Category; readonly about: string }[]>> = {
  adrs: [
    { about: 'a decision that names code, config or a command, checked against the code', category: 'drift' },
    { about: 'an assumption, revisit condition or promised consequence, checked against the code', category: 'stale' },
  ],
  code: [],
  decisions: [],
  records: [],
  recovery: [],
  spec: [],
  tickets: [],
}

/**
 * The ADR review's rule (#249): every finding quotes a record under review;
 * a contradiction quotes two distinct files, each a record or a reference,
 * since a fact is never a recorded decision; wording with two readings gives
 * both as an input; and a drift or stale finding needs code under `tree/`,
 * so a documents-only packet skips it.
 */
function adrRule(finding: FindingBody, found: EvidenceFound, packet: PacketIndex): HintCode | undefined {
  if (!packet.tree && CODE_CATEGORIES.adrs.some((code) => code.category === finding.category)) return 'needs-tree'
  if (!found.spec) return 'no-record-quote'
  if (finding.category === 'contradiction' && !quotesTwoFiles(finding.evidence, packet)) return 'one-file-contradiction'
  return finding.category === 'unasked-question' && !found.scenario ? 'no-scenario' : undefined
}

/**
 * Whether two of the quotes can be told apart by file: some record or
 * reference holds the first and not the second, and another holds the
 * second and not the first. Two quotes from one record stay one file even
 * when another file repeats one of them.
 */
function quotesTwoFiles(evidence: readonly Evidence[], packet: PacketIndex): boolean {
  const documents = packet.files.filter((file) => file.kind === 'artifact' || file.kind === 'reference')
  const holders = evidence.flatMap((item) => {
    if (item.kind !== 'quote') return []
    const quote = normalise(item.value)
    return [documents.filter((file) => file.normalised.includes(quote)).map((file) => file.path)]
  })
  const apart = (first: readonly string[], second: readonly string[]) => first.some((path) => !second.includes(path))
  return holders.some((first, i) => holders.some((second, j) => i !== j && apart(first, second) && apart(second, first)))
}

/**
 * A check a run skipped because its packet holds no code under `tree/`: a
 * category of the stage's evidence rule, or a role rule or lens check marked
 * as needing `tree/`.
 */
export interface SkippedCheck {
  readonly check: string
  /** `category`, `role <name>` or `lens <name>`. */
  readonly from: string
  readonly about?: string
}

/**
 * Every check a run's critics could not run because the packet holds no
 * code under `tree/`: the categories the stage's evidence rule takes only
 * with code, then each check marked "Needs `tree/`" in the role and lens
 * files the critics' prompts declare, by the file that declares it
 * (`role <name>` or `lens <name>`), once each.
 */
export function skippedChecks(
  stage: CritiqueStage,
  packet: Pick<PacketIndex, 'tree'>,
  sources: readonly { readonly from: string; readonly checks: readonly DeclaredCheck[] }[],
): readonly SkippedCheck[] {
  if (packet.tree) return []
  const categories = CODE_CATEGORIES[evidenceRule(stage, undefined)]
  const skipped: SkippedCheck[] = categories.map(({ about, category }) => ({ about, check: category, from: 'category' }))
  for (const source of sources) {
    for (const check of source.checks) {
      const known = skipped.some((entry) => entry.check === check.id)
      if (check.needsTree === true && !known) skipped.push({ check: check.id, from: source.from })
    }
  }
  return skipped
}

export function readSkippedCheck(fields: JsonFields): SkippedCheck {
  const about = fields.optionalString('about')
  return { check: fields.string('check'), from: fields.string('from'), ...(about === undefined ? {} : { about }) }
}

const HINT_REASONS: Readonly<Record<HintCode, string>> = {
  'needs-tree': NEEDS_TREE.reason,
  'no-record-quote': 'no quote from a record under review',
  'one-file-contradiction': 'a contradiction quotes one file; it needs a quote from each of two files',
  'no-decision-or-scenario': 'no quote from a decision source and no concrete scenario',
  'no-decision-quote': 'no quote from a decision source',
  'no-evidence': 'no evidence to check',
  'no-scenario': 'no concrete scenario',
  'no-spec-quote': 'no quote from the spec',
  'no-test-or-trace': 'no failing test and no trace with a file and line',
  'no-ticket-id': 'the location names no ticket of the spec',
  'no-ticket-rule': 'the location names no ticket rule',
  'not-a-candidate': 'not an undocumented-decision candidate: this stage recovers decisions only',
  'not-a-decision-gap': 'neither an unasked question nor a contradiction',
}

/** Runs of white space count as one space, so a quote may cross a line break. */
function normalise(text: string): string {
  return text.replace(/\s+/g, ' ').trim()
}
