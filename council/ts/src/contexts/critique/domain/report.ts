import { countVerdicts, isDissent, type AdjudicatedRun, type RecordedCritic, type RecordedFinding, type Verdict } from './verdicts.js'

/** About 1.5k tokens: the most a summary may add to the planning conversation. */
export const SUMMARY_MAX_CHARS = 6000
const CLAIM_MAX_CHARS = 160

type Adjudicated = AdjudicatedRun['findings'][number]

/**
 * The compact summary, always in the same shape: header, critics, accepted
 * findings, questions for the owner, dissent, and the report path. When it
 * would exceed its budget, the longest list loses items from its end and says
 * how many more the report holds.
 */
export function renderSummary(run: AdjudicatedRun, reportPath: string): string {
  const lists = {
    accepted: run.findings.filter((f) => f.verdict.verdict === 'accepted').map((f) => line(f)),
    dissent: run.findings.filter((f) => isDissent(run, f)).map((f) => line(f, `Rejected: ${f.verdict.reason ?? ''}`)),
    questions: run.findings
      .filter((f) => f.verdict.verdict === 'needs-human')
      .map((f) => line(f, f.verdict.reason === undefined ? undefined : `Question: ${f.verdict.reason}`)),
  }
  const hidden = { accepted: 0, dissent: 0, questions: 0 }
  const render = (): string =>
    [
      `# Critique: ${run.stage}, run ${run.run_id}`,
      '',
      `Depth: ${run.depth}`,
      verdictLine(run),
      '',
      '## Critics',
      '',
      ...run.critics.map((critic) => `- ${criticLine(critic)}`),
      ...run.dropped.map((d) => `- Not run: ${d.client} (${d.family}): ${d.reason}`),
      '',
      '## Accepted',
      '',
      ...bullets(lists.accepted, hidden.accepted),
      '',
      '## Questions for you',
      '',
      ...bullets(lists.questions, hidden.questions),
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
  let summary = render()
  while (summary.length > SUMMARY_MAX_CHARS) {
    const longest = (Object.keys(lists) as (keyof typeof lists)[]).reduce((a, b) =>
      lists[b].length > lists[a].length ? b : a,
    )
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
    `# Critique report: ${run.stage}, run ${run.run_id}`,
    '',
    `Repository: ${run.repository}`,
    `Depth: ${run.depth}`,
    verdictLine(run),
    '',
    '## Critics',
    '',
    ...run.critics.flatMap((critic) => [
      `### ${critic.critic_id}`,
      '',
      `- ${criticLine(critic)}`,
      `- Role ${critic.role}, lenses ${critic.lenses.join(', ')}`,
      `- Exit code ${String(critic.exit_code)}, ${String(critic.duration_ms)} ms`,
      ...(critic.detail === undefined ? [] : [`- Detail: ${critic.detail}`]),
      `- Prompt hashes: ${hashList({ ...critic.prompt_hashes })}`,
      `- Lens hashes: ${hashList(critic.lens_hashes)}`,
      '',
    ]),
    ...run.dropped.map((d) => `- Not run: ${d.client} (${d.family}): ${d.reason}`),
    '## Findings',
    '',
    ...(run.findings.length === 0 ? ['none', ''] : run.findings.flatMap(findingSection)),
    '## Dissent',
    '',
    ...(dissent.length === 0 ? ['none'] : dissent.map((f) => `- ${f.id}: ${f.verdict.reason ?? ''}`)),
    '',
  ].join('\n')
}

function hashList(hashes: Readonly<Record<string, string>>): string {
  return Object.entries(hashes)
    .map(([name, hash]) => `${name} ${hash}`)
    .join(', ')
}

function findingSection(finding: Adjudicated): readonly string[] {
  return [
    `### ${finding.id} [${finding.severity} ${finding.category}] ${finding.location}`,
    '',
    `Raised by: ${finding.raised_by.join(', ')}`,
    `Verdict: ${verdictText(finding.verdict)}`,
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
  const reason = verdict.reason === undefined ? '' : `: ${verdict.reason}`
  return `${verdict.verdict}${code}${reason}`
}

function verdictLine(run: AdjudicatedRun): string {
  const counts = countVerdicts(run)
  return `Verdicts: accepted ${String(counts.accepted)}, rejected ${String(counts.rejected)}, needs-human ${String(counts['needs-human'])}, hint ${String(counts.hint)}`
}

function criticLine(critic: RecordedCritic): string {
  const cost = critic.cost_usd === null ? 'cost unknown' : `$${critic.cost_usd.toFixed(4)}`
  const status = critic.status === 'ok' ? 'ok' : `failed (${critic.failure ?? 'unknown'})`
  const checked = critic.no_findings_reason === undefined ? '' : ` (checked: ${critic.no_findings_reason})`
  return `${critic.client} (${critic.model}, ${critic.family}): ${status}, ${String(critic.finding_count)} finding${critic.finding_count === 1 ? '' : 's'}, ${cost}${checked}`
}

function line(finding: RecordedFinding, tail?: string): string {
  const claim = finding.claim.length > CLAIM_MAX_CHARS ? `${finding.claim.slice(0, CLAIM_MAX_CHARS - 1)}…` : finding.claim
  return `- ${finding.id} [${finding.severity} ${finding.category}] ${finding.location}: ${claim}${tail === undefined ? '' : ` ${tail}`}`
}

function bullets(items: readonly string[], hidden: number): readonly string[] {
  if (items.length === 0 && hidden === 0) return ['none']
  return [...items, ...(hidden === 0 ? [] : [`- … ${String(hidden)} more in the report`])]
}
