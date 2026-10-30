import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  DECISION_CANARY,
  fenced,
  finding,
  harness,
  isolatedExport,
  SPEC_CANARY,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { FanOutResult, PrepareResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const LONG_CLAIM = 'The spec leaves the frobnication window undefined for widgets created during a run. '.repeat(6)

async function reviewed(
  findings: readonly Record<string, unknown>[],
  script?: VibeScript,
): Promise<{ h: Harness; run: PrepareResult; fan: FanOutResult }> {
  const h = await harness({
    scripts: [
      script ?? { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings })) },
    ],
  })
  open.push(h)
  await h.commands.allow({ repo: h.repo, families: ['mistral'] })
  const run = await h.commands.prepare({
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: '.scratch/widgets/spec.md',
    stage: 'spec',
  })
  if (run.status !== 'ready') throw new Error('not ready')
  const fan = await h.commands.fanOut({ run_dir: run.run_dir })
  return { fan, h, run }
}

function ledgerLines(h: Harness): Promise<Record<string, unknown>[]> {
  return readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8').then((text) =>
    text
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as Record<string, unknown>),
  )
}

const four = [
  finding({ id: 'F1', severity: 'critical' }),
  finding({ id: 'F2', category: 'omission', location: 'Family allowlist', severity: 'critical' }),
  finding({ id: 'F3', category: 'untestable', location: 'User story 9', severity: 'major' }),
  finding({ evidence: [], id: 'F4', location: 'Out of Scope', severity: 'minor' }),
]

const verdicts = [
  { finding_id: 'F1', verdict: 'accepted' },
  { finding_id: 'F2', reason: 'CONTEXT.md does not decide this; the quote is from an example.', reason_code: 'not-a-decision', verdict: 'rejected' },
  { finding_id: 'F3', reason: 'Which threshold should apply?', verdict: 'needs-human' },
  { finding_id: 'F4', verdict: 'hint' },
]

describe('record', () => {
  it('writes the report and a summary in the fixed shape', async () => {
    const { h, run } = await reviewed(four)

    const result = await h.commands.record({ run_dir: run.run_dir, verdicts })

    expect(result.report_path).toBe(join(run.run_dir, 'report.md'))
    expect(result.counts).toEqual({ accepted: 1, hint: 1, 'needs-human': 1, rejected: 1 })
    const summary = result.summary
    expect(summary).toContain('vibe-mistral-large (mistral-large-4-0-3, mistral): ok, 4 findings, $0.0123')
    expect(summary).toContain('Verdicts: accepted 1, rejected 1, needs-human 1, hint 1')
    const headings = summary.split('\n').filter((line) => line.startsWith('## '))
    expect(headings).toEqual(['## Critics', '## Accepted', '## Questions for you', '## Dissent', '## Report'])
    expect(section(summary, 'Accepted')).toContain('F1 [critical contradiction] User story 3')
    expect(section(summary, 'Questions for you')).toContain('F3 [major untestable] User story 9')
    expect(section(summary, 'Questions for you')).toContain('Which threshold should apply?')
    expect(section(summary, 'Report')).toContain(result.report_path)
    expect(await readFile(result.summary_path, 'utf8')).toBe(summary)
  })

  it('lists a rejected critical finding under Dissent with the reason', async () => {
    const { h, run } = await reviewed(four)

    const { summary, report_path } = await h.commands.record({ run_dir: run.run_dir, verdicts })

    expect(section(summary, 'Dissent')).toContain('F2 [critical omission] Family allowlist')
    expect(section(summary, 'Dissent')).toContain('CONTEXT.md does not decide this')
    expect(await readFile(report_path, 'utf8')).toContain('## Dissent')
  })

  it('puts every finding, its evidence and its verdict in the report', async () => {
    const { h, run } = await reviewed(four)

    const { report_path } = await h.commands.record({ run_dir: run.run_dir, verdicts })

    const report = await readFile(report_path, 'utf8')
    for (const id of ['F1', 'F2', 'F3', 'F4']) expect(report).toContain(`### ${id} `)
    expect(report).toContain(SPEC_CANARY)
    expect(report).toContain('Verdict: rejected (not-a-decision): CONTEXT.md does not decide this')
    expect(report).toContain('Suggestion: Say that only the nightly job frobnicates widgets.')
    expect(report).toContain('(no evidence)')
  })

  it('appends one run line and one line per finding to the ledger, with the prompt and lens hashes', async () => {
    const { fan, h, run } = await reviewed(four)

    await h.commands.record({ run_dir: run.run_dir, verdicts })

    const lines = await ledgerLines(h)
    expect(lines.map((line) => line.type)).toEqual(['run', 'finding', 'finding', 'finding', 'finding'])
    expect(lines[0]).toMatchObject({
      client: 'vibe-mistral-large',
      cost_usd: 0.0123,
      duration_ms: 1500,
      family: 'mistral',
      finding_count: 4,
      repository: 'github.com/jorisjonkers-dev/agent-kit',
      run_id: run.run_id,
      stage: 'spec',
      status: 'ok',
    })
    const critic = fan.critics[0]
    for (const line of lines) {
      expect(line.prompt_hashes).toEqual(critic?.prompt_hashes)
      expect(line.lens_hashes).toEqual(critic?.lens_hashes)
    }
    expect(lines[2]).toMatchObject({
      category: 'omission',
      finding_id: 'F2',
      lenses: ['fidelity', 'testability'],
      reason_code: 'not-a-decision',
      role: 'spec-redteam',
      severity: 'critical',
      verdict: 'rejected',
    })
  })

  it('keeps artifact text out of the ledger', async () => {
    const { h, run } = await reviewed([finding({ claim: `Claim quoting ${SPEC_CANARY}`, suggestion: DECISION_CANARY })])

    await h.commands.record({
      run_dir: run.run_dir,
      verdicts: [{ finding_id: 'F1', reason: `Rejected because ${SPEC_CANARY}`, verdict: 'rejected' }],
    })

    const ledger = await readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8')
    for (const text of [SPEC_CANARY, DECISION_CANARY, 'User story 3', 'frobnicat']) {
      expect(ledger).not.toContain(text)
    }
  })

  it('records a failed critic in the ledger and the summary', async () => {
    const { h, run } = await reviewed([], { exitCode: 0, exportJson: isolatedExport(), stdout: '' })

    const result = await h.commands.record({ run_dir: run.run_dir, verdicts: [] })

    expect(result.summary).toContain('vibe-mistral-large (mistral-large-4-0-3, mistral): failed (empty-output)')
    expect(section(result.summary, 'Accepted')).toBe('none')
    expect(await ledgerLines(h)).toEqual([
      expect.objectContaining({ failure: 'empty-output', finding_count: 0, status: 'failed', type: 'run' }),
    ])
  })

  it('says no critique ran when every critic failed', async () => {
    const { h, run } = await reviewed([], { exitCode: 0, exportJson: isolatedExport(), stdout: '' })

    const result = await h.commands.record({ run_dir: run.run_dir, verdicts: [] })

    expect(result.degraded).toBe(true)
    expect(result.summary.split('\n').slice(0, 3)).toEqual(['# Critique: spec, run 20261030T120000Z-spec', '', 'degraded: no critique ran'])
  })

  it('says no critique ran when no critic was allowed to run', async () => {
    const h = await harness()
    open.push(h)
    await h.commands.allow({ repo: h.repo, families: ['google'] })
    const run = await h.commands.prepare({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
    if (run.status !== 'ready') throw new Error('not ready')
    await h.commands.fanOut({ run_dir: run.run_dir })

    const { summary } = await h.commands.record({ run_dir: run.run_dir, verdicts: [] })

    expect(summary).toContain('degraded: no critique ran')
    expect(section(summary, 'Critics')).toBe('- Not run: vibe-mistral-large (mistral): family-not-allowed')
  })

  it('does not call a run degraded when a critic reviewed the artifact', async () => {
    const { h, run } = await reviewed([finding()])

    const result = await h.commands.record({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect(result.degraded).toBe(false)
    expect(result.summary).not.toContain('degraded')
  })

  it('says what a critic checked when it found nothing', async () => {
    const { h, run } = await reviewed([], {
      exitCode: 0,
      exportJson: isolatedExport({}),
      stdout: vibeStdout(fenced({ findings: [], no_findings_reason: 'Checked every story.' })),
    })

    const { summary } = await h.commands.record({ run_dir: run.run_dir, verdicts: [] })

    expect(summary).toContain('ok, 0 findings, $0.0123 (checked: Checked every story.)')
  })

  it('stays within its size budget however many findings there are', async () => {
    const many = Array.from({ length: 60 }, (_, i) => finding({ claim: LONG_CLAIM, id: `F${String(i + 1)}` }))
    const { h, run } = await reviewed(many)

    const { summary } = await h.commands.record({
      run_dir: run.run_dir,
      verdicts: many.map((_, i) => ({
        finding_id: `F${String(i + 1)}`,
        reason: 'Needs a decision on the frobnication window.',
        verdict: i % 2 === 0 ? 'accepted' : 'needs-human',
      })),
    })

    expect(summary.length).toBeLessThanOrEqual(6000)
    expect(section(summary, 'Accepted')).toMatch(/more in the report/)
    expect(section(summary, 'Report')).toContain(join(run.run_dir, 'report.md'))
  })

  it('stays within its size budget when a critic explains at length why it found nothing', async () => {
    const reason = `Checked every story against CONTEXT.md. ${'Nothing contradicts it. '.repeat(300)}`
    const { h, run } = await reviewed([], {
      exitCode: 0,
      exportJson: isolatedExport(),
      stdout: vibeStdout(fenced({ findings: [], no_findings_reason: reason })),
    })

    const { report_path, summary } = await h.commands.record({ run_dir: run.run_dir, verdicts: [] })

    expect(summary.length).toBeLessThanOrEqual(6000)
    expect(section(summary, 'Critics')).toContain('(checked: Checked every story against CONTEXT.md.')
    expect(await readFile(report_path, 'utf8')).toContain(reason.trim())
  })

  it('names critics that were dropped before the run', async () => {
    const routing = {
      clients: {
        'vibe-mistral-large': { agent: 'critic-mistral-large', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-large-4-0-3' },
        'vibe-glm': { agent: 'critic-glm', binary: 'vibe', family: 'zhipu', kind: 'vibe', model: 'glm-5-3' },
      },
      stages: {
        spec: {
          base: [
            { client: 'vibe-mistral-large', lenses: ['fidelity'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 },
            { client: 'vibe-glm', lenses: ['integration'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 },
          ],
          distinct_families: true,
        },
      },
      version: 1,
    }
    const h = await harness({
      routing,
      scripts: [{ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [], no_findings_reason: 'x' })) }],
    })
    open.push(h)
    await h.commands.allow({ repo: h.repo, families: ['mistral'] })
    const run = await h.commands.prepare({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
    if (run.status !== 'ready') throw new Error('not ready')
    await h.commands.fanOut({ run_dir: run.run_dir })

    const { summary } = await h.commands.record({ run_dir: run.run_dir, verdicts: [] })

    expect(section(summary, 'Critics')).toContain('Not run: vibe-glm (zhipu): family-not-allowed')
  })

  it.each<[string, unknown, RegExp]>([
    ['a missing verdict', [], /no verdict for F1/],
    ['an unknown finding', [{ finding_id: 'F1', verdict: 'accepted' }, { finding_id: 'F9', verdict: 'accepted' }], /unknown finding F9/],
    ['two verdicts for one finding', [{ finding_id: 'F1', verdict: 'accepted' }, { finding_id: 'F1', verdict: 'hint' }], /more than one verdict for F1/],
    ['an unknown verdict', [{ finding_id: 'F1', verdict: 'maybe' }], /verdict must be one of/],
    ['a rejection without a reason', [{ finding_id: 'F1', verdict: 'rejected' }], /reason must be a non-empty string/],
    ['a reason code that is free text', [{ finding_id: 'F1', reason: 'r', reason_code: 'Because I said so', verdict: 'rejected' }], /reason_code must be a short lowercase slug/],
    ['verdicts that are not a list', 'accepted', /verdicts must be a list/],
  ])('rejects %s', async (_name, given, message) => {
    const { h, run } = await reviewed([finding()])

    await expect(h.commands.record({ run_dir: run.run_dir, verdicts: given })).rejects.toThrow(message)
  })

  it('rejects merged findings that are not a fan-out result', async () => {
    const { fan, h, run } = await reviewed([finding()])
    await h.writeText(fan.findings_path, JSON.stringify({ ...fan, findings: [{ id: 'F1' }] }))

    await expect(
      h.commands.record({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] }),
    ).rejects.toThrow(/findings\.json: .*must be/)
  })

  it('records a run once', async () => {
    const { h, run } = await reviewed([finding()])
    await h.commands.record({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    await expect(
      h.commands.record({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] }),
    ).rejects.toThrow(/already recorded/)
    expect(await ledgerLines(h)).toHaveLength(2)
  })
})

function section(summary: string, heading: string): string {
  const start = summary.indexOf(`## ${heading}\n`)
  if (start === -1) return ''
  const body = summary.slice(start + heading.length + 4)
  const end = body.indexOf('\n## ')
  return (end === -1 ? body : body.slice(0, end)).trim()
}
