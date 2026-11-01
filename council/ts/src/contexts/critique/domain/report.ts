import type { CriticRun } from './critic.js'
import { signalledDepth, type DepthSignal } from './depth.js'
import type { RunFinding } from './findings.js'
import type { DroppedCritic } from './routing.js'
import { countVerdicts, familiesOf, isDissent, openCriticalQuestions, VERDICTS, type AdjudicatedFinding, type AdjudicatedRun, type Verdict } from './verdicts.js'

/** About 1.5k tokens: the most a summary may add to the planning conversation. */
export const SUMMARY_MAX_CHARS = 6000
const CLAIM_MAX_CHARS = 160
/** What a critic that found nothing says it checked; the report keeps all of it. */
const CHECKED_MAX_CHARS = 300
/** One critic's minor findings, by id; the report keeps each in full. */
const MINOR_MAX_CHARS = 300

/**
 * The compact summary, always in the same shape: header, critics, accepted
 * findings, questions for the owner, minor findings, dissent, and the report
 * path. Minor findings never take a line of their own: each critic gets one
 * line naming its minor findings by verdict (#167 US33). When it
 * would exceed its budget, the longest list loses items from its end and says
 * how many more the report holds. Text outside the lists is clipped, so
 * emptying the lists always brings it within budget.
 */
export function renderSummary(run: AdjudicatedRun, reportPath: string): string {
  const serious = run.findings.filter((f) => f.severity !== 'minor')
  const lists = {
    accepted: serious.filter((f) => f.verdict.verdict === 'accepted').map((f) => findingLine(run, f)),
    dissent: run.findings.filter((f) => isDissent(run, f)).map((f) => findingLine(run, f, `Rejected: ${f.verdict.reason ?? ''}`)),
    questions: serious
      .filter((f) => f.verdict.verdict === 'needs-human')
      .map((f) => findingLine(run, f, questionTail(f.verdict))),
  }
  const hidden = { accepted: 0, dissent: 0, questions: 0 }
  const render = (): string =>
    [
      ...violationLines(run),
      `# Critique: ${run.stage}, run ${run.run_id}`,
      '',
      ...degradedLines(run),
      depthLine(run),
      ...roundLines(run),
      verdictLine(run),
      ...toTicketsLines(run),
      '',
      '## Critics',
      '',
      ...run.critics.map((critic) => `- ${criticLine(run, critic, CHECKED_MAX_CHARS)}`),
      ...run.dropped.map(droppedLine),
      '',
      '## Accepted',
      '',
      ...bullets(lists.accepted, hidden.accepted),
      '',
      '## Questions for you',
      '',
      ...bullets(lists.questions, hidden.questions),
      '',
      '## Minor',
      '',
      ...bullets(minorLines(run), 0),
      '',
      '## Dissent',
      '',
      ...bullets(lists.dissent, hidden.dissent),
      '',
      '## Report',
      '',
      reportPath,
      '',
    ].join('\n')
  const trimmable = () => (Object.keys(lists) as (keyof typeof lists)[]).filter((key) => lists[key].length > 0)
  let summary = render()
  while (summary.length > SUMMARY_MAX_CHARS && trimmable().length > 0) {
    const longest = trimmable().reduce((a, b) => (lists[b].length > lists[a].length ? b : a))
    lists[longest] = lists[longest].slice(0, -1)
    hidden[longest] += 1
    summary = render()
  }
  return summary
}

/** The full report: every critic, every finding with its evidence and verdict, and dissent. */
export function renderReport(run: AdjudicatedRun): string {
  const dissent = run.findings.filter((f) => isDissent(run, f))
  return [
    ...violationLines(run),
    `# Critique report: ${run.stage}, run ${run.run_id}`,
    '',
    ...degradedLines(run),
    `Repository: ${run.repository}`,
    depthLine(run),
    verdictLine(run),
    '',
    '## Critics',
    '',
    ...run.critics.flatMap((critic) => [
      `### ${critic.critic_id}`,
      '',
      `- ${criticLine(run, critic)}`,
      `- Role ${critic.role}, lenses ${critic.lenses.join(', ')}`,
      `- Exit code ${String(critic.exit_code)}, ${String(critic.duration_ms)} ms`,
      ...(critic.detail === undefined ? [] : [`- Detail: ${critic.detail}`]),
      `- Prompt hashes: ${hashList({ ...critic.prompt_hashes })}`,
      `- Lens hashes: ${hashList(critic.lens_hashes)}`,
      '',
    ]),
    ...run.dropped.map(droppedLine),
    '## Findings',
    '',
    ...(run.findings.length === 0 ? ['none', ''] : run.findings.flatMap((f) => findingSection(run, f))),
    '## Dissent',
    '',
    ...(dissent.length === 0 ? ['none'] : dissent.map((f) => `- ${f.id}: ${f.verdict.reason ?? ''}`)),
    '',
  ].join('\n')
}

/**
 * #167 US22: a critic that changed its packet or the working tree makes its
 * critic client unsafe, so the summary and the report open with that, one
 * line per such critic, before anything else.
 */
function violationLines(run: AdjudicatedRun): readonly string[] {
  const violators = run.critics.filter((critic) => critic.status === 'violated-readonly')
  if (violators.length === 0) return []
  return [
    ...violators.map(
      (critic) =>
        `> **Warning: read-only violated** by critic client ${critic.client} (${critic.critic_id}). ` +
        'Its findings were discarded and the packet restored. Do not rely on this client until its read-only settings are fixed.',
    ),
    '',
  ]
}

/** #167 US29: a stage where no critic reviewed anything says so first. */
function degradedLines(run: AdjudicatedRun): readonly string[] {
  return run.degraded ? ['degraded: no critique ran', ''] : []
}

function roundLines(run: AdjudicatedRun): readonly string[] {
  return run.round === 2 ? [`Round 2, after run ${run.previous_run_id ?? ''}`] : []
}

/** #167 US40: tickets are never cut from a spec with an open critical gap. */
function toTicketsLines(run: AdjudicatedRun): readonly string[] {
  const open = openCriticalQuestions(run).map((f) => f.id)
  if (open.length === 0) return []
  const which = open.length === 1 ? `critical question ${open.join('')} is` : `critical questions ${open.join(', ')} are`
  return [`Do not start /to-tickets: ${which} unanswered.`]
}

function questionTail(verdict: Verdict): string | undefined {
  const parts = [
    ...(verdict.reason === undefined ? [] : [`Question: ${verdict.reason}`]),
    ...(verdict.answered === true ? ['(answered)'] : []),
  ]
  return parts.length === 0 ? undefined : parts.join(' ')
}

/**
 * #167 US44: the depth, whether the caller or the signals chose it, and the
 * signals that fired (or, when none did, what was measured), so the owner can
 * tell whether the automatic choice is sound.
 */
function depthLine(run: AdjudicatedRun): string {
  const choice = run.depth_choice
  const measured = Object.entries(choice.measures).map(([measure, { threshold, value }]) => `${measure} ${String(value)} of ${String(threshold)}`)
  const signals =
    choice.signals.length > 0 ? choice.signals.map(signalText).join('; ') : ['none fired', ...(measured.length === 0 ? [] : [measured.join(', ')])].join(': ')
  return choice.chosen_by === 'argument'
    ? `Depth: ${run.depth}, chosen by argument; the signals alone would choose ${signalledDepth(choice)} (${signals})`
    : `Depth: ${run.depth}, chosen by signals: ${signals}`
}

function signalText(signal: DepthSignal): string {
  if (signal.kind === 'threshold') return `${signal.measure} ${String(signal.value)} > ${String(signal.threshold)}`
  return signal.kind === 'risk-term' ? `risk term "${signal.term}"` : `label ${signal.label}`
}

function droppedLine(dropped: DroppedCritic): string {
  const detail = dropped.detail === undefined ? '' : ` (${dropped.detail})`
  const slot =
    dropped.replaced_by !== undefined
      ? `replaced by ${dropped.replaced_by}`
      : dropped.deep === true
        ? 'deep critic left out'
        : 'no fallback left, so the stage runs with fewer critics'
  return `- Not run: ${dropped.client} (${dropped.family}): ${dropped.reason}${detail}; ${slot}`
}

function hashList(hashes: Readonly<Record<string, string>>): string {
  return Object.entries(hashes)
    .map(([name, hash]) => `${name} ${hash}`)
    .join(', ')
}

function findingSection(run: AdjudicatedRun, finding: AdjudicatedFinding): readonly string[] {
  return [
    `### ${finding.id} [${finding.severity} ${finding.category}] ${finding.location}`,
    '',
    `Raised by: ${finding.raised_by.join(', ')} (${familiesOf(run, finding).join(', ')})`,
    `Verdict: ${verdictText(finding.verdict)}`,
    ...(finding.pre_verdict === undefined || finding.pre_verdict.verdict === finding.verdict.verdict
      ? []
      : [`Pre-verdict: ${finding.pre_verdict.verdict} (${finding.pre_verdict.reason_code}): ${finding.pre_verdict.reason}`]),
    '',
    finding.claim,
    '',
    'Evidence:',
    '',
    ...(finding.evidence.length === 0
      ? ['- (no evidence)']
      : finding.evidence.map((e) => `- ${e.kind}${e.file === undefined ? '' : ` (${e.file})`}: ${e.value}`)),
    ...(finding.suggestion === undefined ? [] : ['', `Suggestion: ${finding.suggestion}`]),
    '',
  ]
}

function verdictText(verdict: Verdict): string {
  const code = verdict.reason_code === undefined ? '' : ` (${verdict.reason_code})`
  const answered = verdict.answered === true ? ', answered' : ''
  const reason = verdict.reason === undefined ? '' : `: ${verdict.reason}`
  return `${verdict.verdict}${answered}${code}${reason}`
}

function verdictLine(run: AdjudicatedRun): string {
  const counts = countVerdicts(run)
  return `Verdicts: accepted ${String(counts.accepted)}, rejected ${String(counts.rejected)}, needs-human ${String(counts['needs-human'])}, hint ${String(counts.hint)}`
}

function criticLine(run: AdjudicatedRun, critic: CriticRun, checkedMaxChars = Infinity): string {
  const who = `${critic.client} (${critic.model}, ${critic.family})`
  if (critic.status === 'budget-skip') return `${who}: budget-skip: ${critic.detail ?? ''}${retryText(run, critic)}`
  const cost = critic.cost_usd === null ? 'cost unknown' : `$${critic.cost_usd.toFixed(4)}`
  const status =
    critic.status === 'ok' ? 'ok' : critic.status === 'violated-readonly' ? 'violated read-only' : `failed (${critic.failure ?? 'unknown'})`
  const checked =
    critic.no_findings_reason === undefined ? '' : ` (checked: ${clip(critic.no_findings_reason, checkedMaxChars)})`
  return `${critic.client} (${critic.model}, ${critic.family}): ${status}, ${String(critic.finding_count)} finding${critic.finding_count === 1 ? '' : 's'}, ${cost}${retryText(run, critic)}${checked}`
}

/** A fallback retry names the critic it stands in for, and why that one failed. */
function retryText(run: AdjudicatedRun, critic: CriticRun): string {
  if (critic.retry_of === undefined) return ''
  const failed = run.critics.find((other) => other.critic_id === critic.retry_of)
  return `, fallback for ${critic.retry_of} (${failed?.failure ?? 'failed'})`
}

function clip(text: string, maxChars: number): string {
  return text.length > maxChars ? `${text.slice(0, maxChars - 1)}…` : text
}

/** One finding in the summary; agreement across families is named, so it stays visible. */
function findingLine(run: AdjudicatedRun, finding: RunFinding, tail?: string): string {
  const claim = clip(finding.claim, CLAIM_MAX_CHARS)
  const families = familiesOf(run, finding)
  const agreement = families.length < 2 ? '' : ` (raised by ${families.join(' and ')})`
  return `- ${finding.id} [${finding.severity} ${finding.category}] ${finding.location}: ${claim}${agreement}${tail === undefined ? '' : ` ${tail}`}`
}

function minorLines(run: AdjudicatedRun): readonly string[] {
  return run.critics.flatMap((critic) => {
    const minor = run.findings.filter((f) => f.severity === 'minor' && f.raised_by.includes(critic.critic_id))
    if (minor.length === 0) return []
    const byVerdict = VERDICTS.flatMap((verdict) => {
      const ids = minor.filter((f) => f.verdict.verdict === verdict).map((f) => f.id)
      return ids.length === 0 ? [] : [`${verdict} ${ids.join(', ')}`]
    })
    return [clip(`- ${critic.critic_id}: ${String(minor.length)} minor: ${byVerdict.join('; ')}`, MINOR_MAX_CHARS)]
  })
}

function bullets(items: readonly string[], hidden: number): readonly string[] {
  if (items.length === 0 && hidden === 0) return ['none']
  return [...items, ...(hidden === 0 ? [] : [`- … ${String(hidden)} more in the report`])]
}
