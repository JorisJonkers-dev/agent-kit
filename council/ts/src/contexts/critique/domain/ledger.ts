import { familiesOf, type AdjudicatedRun } from './verdicts.js'

/**
 * Ledger lines hold metadata only: identifiers, enums, numbers and hashes.
 * Claims, locations, evidence, suggestions and free-text reasons stay in the
 * report, so the ledger can be kept indefinitely.
 */
export function ledgerLines(run: AdjudicatedRun, recordedAt: Date): readonly string[] {
  const common = {
    depth: run.depth,
    recorded_at: recordedAt.toISOString(),
    repository: run.repository,
    run_id: run.run_id,
    stage: run.stage,
  }
  const criticFields = (criticId: string) => {
    const critic = run.critics.find((c) => c.critic_id === criticId)
    return {
      client: critic?.client,
      critic_id: criticId,
      family: critic?.family,
      lens_hashes: critic?.lens_hashes,
      lenses: critic?.lenses,
      model: critic?.model,
      prompt_hashes: critic?.prompt_hashes,
      role: critic?.role,
    }
  }
  const runLines = run.critics.map((critic) => ({
    type: 'run',
    ...common,
    ...criticFields(critic.critic_id),
    cost_usd: critic.cost_usd,
    duration_ms: critic.duration_ms,
    exit_code: critic.exit_code,
    failure: critic.failure ?? null,
    finding_count: critic.finding_count,
    status: critic.status,
  }))
  const findingLines = run.findings.map((finding) => ({
    type: 'finding',
    ...common,
    ...criticFields(finding.raised_by[0] ?? ''),
    category: finding.category,
    evidence_kinds: finding.evidence.map((e) => e.kind),
    finding_id: finding.id,
    raised_by: finding.raised_by,
    raised_by_families: familiesOf(run, finding),
    reason_code: finding.verdict.reason_code ?? null,
    severity: finding.severity,
    verdict: finding.verdict.verdict,
  }))
  return [...runLines, ...findingLines].map((line) => JSON.stringify(line))
}
