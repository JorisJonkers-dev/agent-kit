import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  fenced,
  finding,
  harness,
  isolatedExport,
  SPEC_CANARY,
  SyntheticLedger,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { FanOutResult, OrchestratorFamily, PrepareResult, RefuteResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const slot = (client: string) => ({ client, lenses: ['fidelity'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 })

/** Mistral Large and GLM at base, DeepSeek Flash added at deep: three families the refuter can come from. */
const ROUTING = {
  clients: {
    'vibe-deepseek-flash': { agent: 'critic-deepseek-flash', binary: 'vibe', family: 'deepseek', kind: 'vibe', model: 'deepseek-v4-1-flash-1017' },
    'vibe-glm': { agent: 'critic-glm', binary: 'vibe', family: 'zhipu', kind: 'vibe', model: 'zai-glm-5-3' },
    'vibe-mistral-large': { agent: 'critic-mistral-large', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-large-4-0' },
  },
  stages: {
    spec: { base: [slot('vibe-mistral-large'), slot('vibe-glm')], deep: [slot('vibe-deepseek-flash')], distinct_families: true },
  },
  version: 1,
}

const vibe = (agent: string, value: unknown, cost = 0.0123): VibeScript => ({
  agent,
  exitCode: 0,
  exportJson: { ...isolatedExport(), cost_usd: cost },
  stdout: vibeStdout(fenced(value)),
})

const NOTHING = { findings: [], no_findings_reason: 'Compared every story with the glossary.' }

const critical = (overrides: Record<string, unknown> = {}) => finding({ severity: 'critical', ...overrides })

interface Reviewed {
  readonly h: Harness
  readonly run: PrepareResult
  readonly fan: FanOutResult
}

/**
 * A spec run whose Mistral Large critic raised `found`, and whose GLM
 * critic (when the orchestrator is not GLM's family) found nothing.
 */
async function reviewed(
  found: readonly unknown[],
  options: {
    readonly families?: readonly string[]
    readonly orchestrator?: OrchestratorFamily
    readonly ledger?: SyntheticLedger
    readonly assets?: Readonly<Record<string, (shipped: string) => string>>
    readonly routing?: unknown
  } = {},
): Promise<Reviewed> {
  const h = await harness({ routing: options.routing ?? ROUTING, ...(options.assets === undefined ? {} : { assets: options.assets }) })
  open.push(h)
  if (options.ledger !== undefined) await options.ledger.writeTo(h)
  await h.commands.allow.run({ families: options.families ?? ['mistral', 'zhipu', 'deepseek'], repo: h.repo })
  const run = await h.commands.prepare.run({
    orchestrator_family: options.orchestrator ?? 'anthropic',
    repo: h.repo,
    spec: '.scratch/widgets/spec.md',
    stage: 'spec',
  })
  if (run.status !== 'ready') throw new Error('not ready')
  h.process.queue(vibe('critic-mistral-large', { findings: found }), vibe('critic-glm', NOTHING))
  const fan = await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { fan, h, run }
}

const accepted = (...ids: string[]) => ids.map((id) => ({ finding_id: id, verdict: 'accepted' }))

/** The refuter's answer: one finding naming `refute` or `downgrade` in its check. */
const answer = (check: string, overrides: Record<string, unknown> = {}) =>
  finding({
    check,
    claim: 'The spec states the window, so nothing contradicts the decision.',
    evidence: [
      { file: 'artifact/spec.md', kind: 'quote', value: SPEC_CANARY },
      { kind: 'input', value: 'A widget created at second three is frobnicated by second seven, as the story says.' },
    ],
    ...overrides,
  })

function ledgerLines(h: Harness): Promise<Record<string, unknown>[]> {
  return readFile(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8').then((text) =>
    text
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as Record<string, unknown>),
  )
}

async function refuted(r: Reviewed, refuter: VibeScript, verdicts: readonly Record<string, unknown>[] = accepted('F1')): Promise<RefuteResult> {
  r.h.process.queue(refuter)
  return r.h.commands.refute.run({ run_dir: r.run.run_dir, verdicts })
}

describe('refute: the refuter answers', () => {
  it('leaves the verdict as it was on no objection', async () => {
    const r = await reviewed([critical()])

    const result = await refuted(r, vibe('critic-glm', NOTHING))

    expect(result.results).toEqual([
      expect.objectContaining({ answer: 'no-objection', finding_id: 'F1', outcome: 'no-objection' }),
    ])
    const recorded = await r.h.commands.record.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })
    expect(recorded.counts).toMatchObject({ accepted: 1, rejected: 0 })
  })

  it('rejects the finding when the refutation has evidence the packet holds', async () => {
    const r = await reviewed([critical()])

    const result = await refuted(r, vibe('critic-glm', { findings: [answer('refute')] }))

    expect(result.results[0]).toMatchObject({ answer: 'refute', finding_id: 'F1', outcome: 'refute' })
    expect(result.counts).toMatchObject({ refute: 1 })
    const recorded = await r.h.commands.record.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })
    expect(recorded.counts).toMatchObject({ accepted: 0, rejected: 1 })
    const report = await readFile(recorded.report_path, 'utf8')
    expect(report).toContain('Verdict: rejected (refuted)')
    expect(report).toContain('## Refute round')
    expect(report).toContain('F1: refute by r-F1-vibe-glm')
  })

  it('lets the verdict stand when the refutation quotes text the packet does not hold', async () => {
    const r = await reviewed([critical()])

    const result = await refuted(
      r,
      vibe('critic-glm', {
        findings: [answer('refute', { evidence: [{ file: 'artifact/spec.md', kind: 'quote', value: 'Widgets are never frobnicated.' }] })],
      }),
    )

    expect(result.results[0]).toMatchObject({ answer: 'refute', outcome: 'stands', reason_code: 'fabricated-evidence' })
    const recorded = await r.h.commands.record.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })
    expect(recorded.counts).toMatchObject({ accepted: 1, rejected: 0 })
  })

  it("does not count a quote of the finding itself as evidence, since it is not a packet file", async () => {
    const r = await reviewed([critical()])
    const claim = String(critical().claim)

    const result = await refuted(
      r,
      vibe('critic-glm', { findings: [answer('refute', { evidence: [{ file: 'finding', kind: 'quote', value: claim }] })] }),
    )

    expect(result.results[0]).toMatchObject({ outcome: 'stands', reason_code: 'fabricated-evidence' })
  })

  it('lets the verdict stand when the refutation has nothing the stage counts as evidence', async () => {
    const r = await reviewed([critical()])

    const result = await refuted(r, vibe('critic-glm', { findings: [answer('refute', { evidence: [] })] }))

    expect(result.results[0]).toMatchObject({ answer: 'refute', outcome: 'stands', reason_code: 'no-evidence' })
  })

  it('lowers the severity on a downgrade with evidence, and keeps the finding accepted', async () => {
    const r = await reviewed([critical()])

    const result = await refuted(r, vibe('critic-glm', { findings: [answer('downgrade', { severity: 'minor' })] }))

    expect(result.results[0]).toMatchObject({ answer: 'downgrade', outcome: 'downgrade', severity: 'minor' })
    const recorded = await r.h.commands.record.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })
    expect(recorded.counts).toMatchObject({ accepted: 1 })
    expect(await readFile(recorded.report_path, 'utf8')).toContain('### F1 [minor contradiction]')
  })

  it.each([
    ['a downgrade that keeps the finding critical', { findings: [answer('downgrade', { severity: 'critical' })] }],
    ['an answer naming no refuter check', { findings: [answer('decision-dropped')] }],
    ['a second entry', { findings: [answer('refute'), answer('refute', { id: 'F2' })] }],
  ])('lets the verdict stand on %s, so the round cannot add work', async (_name, value) => {
    const r = await reviewed([critical()])

    const result = await refuted(r, vibe('critic-glm', value))

    expect(result.results[0]).toMatchObject({ outcome: 'stands', reason_code: 'invalid-answer' })
  })

  it('lets the verdict stand when the refuter gives no usable review', async () => {
    const r = await reviewed([critical()])

    const result = await refuted(r, { agent: 'critic-glm', exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout('No block.') })

    expect(result.results[0]).toMatchObject({ outcome: 'stands', reason_code: 'no-review', refuter: { status: 'failed' } })
  })

  it('lets the verdict stand when the refuter writes to its packet', async () => {
    const r = await reviewed([critical()])
    const writer: VibeScript = {
      ...vibe('critic-glm', { findings: [answer('refute')] }),
      effect: async ({ cwd }) => {
        await r.h.writeText(join(cwd, 'artifact', 'spec.md'), 'rewritten')
      },
    }

    const result = await refuted(r, writer)

    expect(result.results[0]).toMatchObject({ outcome: 'stands', reason_code: 'no-review', refuter: { status: 'violated-readonly' } })
  })

  it('runs the refuter with the refuter role on the original packet, with the finding in its prompt', async () => {
    const r = await reviewed([critical()])

    const result = await refuted(r, vibe('critic-glm', NOTHING))

    const launch = r.h.process.commands.at(-1)
    expect(launch?.args).toContain('critic-glm')
    expect(launch?.cwd).toBe(join(r.run.run_dir, 'refute', 'F1', 'packet'))
    const refuter = result.results[0]?.refuter
    expect(refuter).toMatchObject({ client: 'vibe-glm', critic_id: 'r-F1-vibe-glm', family: 'zhipu', lenses: [], role: 'refuter' })
    const prompt = await readFile(refuter?.prompt_path ?? '', 'utf8')
    expect(prompt).toContain('# Role: refuter')
    expect(prompt).not.toContain('---\n\n\n\n---')
    expect(prompt).toContain('# The finding')
    expect(prompt).toContain(String(critical().claim))
    expect(prompt).toContain('- `artifact/spec.md`')
    expect(await r.h.readText(join(launch?.cwd ?? '', 'artifact', 'spec.md'))).toContain(SPEC_CANARY)
  })

  it('sends only accepted critical findings to the round', async () => {
    const r = await reviewed([
      critical(),
      finding({ category: 'omission', check: 'omission', id: 'F2', location: 'User story 1', severity: 'major' }),
      critical({ category: 'invention', check: 'invention', id: 'F3', location: 'User story 9' }),
    ])

    const result = await refuted(r, vibe('critic-glm', NOTHING), [
      ...accepted('F1', 'F2'),
      { finding_id: 'F3', reason: 'Not a defect.', verdict: 'rejected' },
    ])

    expect(result.results.map((entry) => entry.finding_id)).toEqual(['F1'])
    expect(r.h.process.commands.filter((command) => command.args.includes('critic-glm'))).toHaveLength(2)
  })

  it('selects by the severity that counts: a verdict override down from critical is not refuted, one up to critical is', async () => {
    // The fidelity lens fixes `wording-changed` critical and `term-meaning` major in this fixture.
    const fixed = (text: string) =>
      text.replace('- `wording-changed`:', '- `wording-changed` (critical):').replace('- `term-meaning`:', '- `term-meaning` (major):')
    const r = await reviewed(
      [
        finding({ category: 'invention', check: 'wording-changed', id: 'F1', location: 'User story 1', severity: 'minor' }),
        finding({ category: 'omission', check: 'term-meaning', id: 'F2', location: 'User story 2', severity: 'minor' }),
      ],
      { assets: { 'lenses/fidelity.md': fixed } },
    )
    expect(r.fan.findings).toEqual([
      expect.objectContaining({ declared_severity: 'critical', id: 'F1', severity: 'critical' }),
      expect.objectContaining({ declared_severity: 'major', id: 'F2', severity: 'major' }),
    ])
    const verdicts = [
      { finding_id: 'F1', severity: 'major', severity_reason: 'copy-only-change', verdict: 'accepted' },
      { finding_id: 'F2', severity: 'critical', severity_reason: 'blocks-release', verdict: 'accepted' },
    ]

    const result = await refuted(r, vibe('critic-glm', NOTHING), verdicts)

    expect(result.results.map((entry) => entry.finding_id)).toEqual(['F2'])
    expect(r.h.process.commands.filter((command) => command.args.includes('critic-glm'))).toHaveLength(2)
    const refuteLine = (await ledgerLines(r.h)).find((line) => line.type === 'refute')
    expect(refuteLine).toMatchObject({ finding_id: 'F2', severity_before: 'critical' })
  })

  it('keeps the severity override on a finding it refutes, so the ledger still shows it', async () => {
    const fixed = (text: string) => text.replace('- `term-meaning`:', '- `term-meaning` (major):')
    const r = await reviewed([finding({ check: 'term-meaning' })], {
      assets: { 'lenses/fidelity.md': fixed },
    })
    const verdicts = [{ finding_id: 'F1', severity: 'critical', severity_reason: 'blocks-release', verdict: 'accepted' }]

    const result = await refuted(r, vibe('critic-glm', { findings: [answer('refute')] }), verdicts)
    expect(result.results[0]).toMatchObject({ finding_id: 'F1', outcome: 'refute' })
    const recorded = await r.h.commands.record.run({ run_dir: r.run.run_dir, verdicts })

    expect(recorded.counts).toMatchObject({ accepted: 0, rejected: 1 })
    const line = (await ledgerLines(r.h)).find((entry) => entry.type === 'finding')
    expect(line).toMatchObject({ declared_severity: 'major', severity: 'critical', severity_override: 'critical', severity_reason: 'blocks-release' })
  })

  it('refuses a severity override record would refuse', async () => {
    const r = await reviewed([critical()])

    await expect(
      r.h.commands.refute.run({ run_dir: r.run.run_dir, verdicts: [{ finding_id: 'F1', severity: 'major', severity_reason: 'copy-only-change', verdict: 'accepted' }] }),
    ).rejects.toMatchObject({ code: 'invalid-input', message: 'F1: only a finding whose check fixes its severity can be overridden' })
  })

  it('has nothing to refute when no critical finding is accepted, and launches no refuter', async () => {
    const r = await reviewed([critical()])
    const launches = r.h.process.commands.length

    const result = await r.h.commands.refute.run({ run_dir: r.run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'needs-human' }] })

    expect(result).toMatchObject({ counts: { refute: 0, skipped: 0 }, results: [] })
    expect(r.h.process.commands).toHaveLength(launches)
    const recorded = await r.h.commands.record.run({ run_dir: r.run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'needs-human' }] })
    expect(await readFile(recorded.report_path, 'utf8')).toContain('## Refute round\n\nno accepted critical finding')
  })
})

describe('refute: choosing the refuter', () => {
  it("prefers a base client over a deep one, skipping the raiser's family", async () => {
    const r = await reviewed([critical()])

    const result = await refuted(r, vibe('critic-glm', NOTHING))

    expect(result.results[0]?.refuter?.client).toBe('vibe-glm')
  })

  it("skips the orchestrator's family, and takes a deep client when no base client is left", async () => {
    const r = await reviewed([critical()], { orchestrator: 'zhipu' })

    const result = await refuted(r, vibe('critic-deepseek-flash', NOTHING))

    expect(result.results[0]?.refuter).toMatchObject({ client: 'vibe-deepseek-flash', family: 'deepseek' })
  })

  it('skips every family that raised the finding', async () => {
    const h = await harness({ routing: ROUTING })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral', 'zhipu', 'deepseek'], repo: h.repo })
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
    if (run.status !== 'ready') throw new Error('not ready')
    h.process.queue(vibe('critic-mistral-large', { findings: [critical()] }), vibe('critic-glm', { findings: [critical()] }))
    const fan = await h.commands.fanOut.run({ run_dir: run.run_dir })
    expect(fan.findings[0]?.raised_by).toEqual(['c1-vibe-mistral-large', 'c2-vibe-glm'])

    const result = await refuted({ fan, h, run }, vibe('critic-deepseek-flash', NOTHING))

    expect(result.results[0]?.refuter?.client).toBe('vibe-deepseek-flash')
  })

  it('refuses a routing whose deep roster names a client it does not define', async () => {
    const h = await harness({
      routing: { ...ROUTING, stages: { spec: { ...ROUTING.stages.spec, deep: [slot('vibe-nobody')] } } },
    })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral', 'zhipu'], repo: h.repo })
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
    if (run.status !== 'ready') throw new Error('not ready')
    h.process.queue(vibe('critic-mistral-large', { findings: [critical()] }), vibe('critic-glm', NOTHING))
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    await expect(h.commands.refute.run({ run_dir: run.run_dir, verdicts: accepted('F1') })).rejects.toMatchObject({
      code: 'invalid-routing',
      message: /unknown client vibe-nobody/,
    })
  })

  it('marks the finding skipped, and its verdict standing, when no eligible client is left', async () => {
    const r = await reviewed([critical()], { families: ['mistral', 'zhipu'], orchestrator: 'zhipu' })
    const launches = r.h.process.commands.length

    const result = await r.h.commands.refute.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })

    expect(result.results).toEqual([
      expect.objectContaining({ finding_id: 'F1', outcome: 'skipped', reason_code: 'no-eligible-client' }),
    ])
    expect(result.results[0]?.refuter).toBeUndefined()
    expect(r.h.process.commands).toHaveLength(launches)
    const recorded = await r.h.commands.record.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })
    expect(recorded.counts).toMatchObject({ accepted: 1 })
    expect(await readFile(recorded.report_path, 'utf8')).toContain('F1: skipped (no-eligible-client)')
  })

  it('skips a Vibe client once the flow has spent its cap', async () => {
    const flow = '.scratch/widgets/spec.md'
    const ledger = new SyntheticLedger().invocation({ charged_usd: 4.99, flow, kind: 'vibe' })
    const r = await reviewed([critical()], { ledger })

    const result = await r.h.commands.refute.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })

    expect(result.results[0]).toMatchObject({ outcome: 'skipped', reason_code: 'no-eligible-client' })
    expect(result.results[0]?.detail).toContain('cap')
  })
})

describe('refute: records and spend', () => {
  it("writes each result to the ledger with the refuter's client and outcome", async () => {
    const r = await reviewed([critical()])

    await refuted(r, vibe('critic-glm', { findings: [answer('downgrade', { severity: 'major' })] }))

    const line = (await ledgerLines(r.h)).find((entry) => entry.type === 'refute')
    expect(line).toMatchObject({
      answer: 'downgrade',
      check: 'contradiction',
      client: 'vibe-glm',
      critic_id: 'r-F1-vibe-glm',
      family: 'zhipu',
      finding_id: 'F1',
      flow: '.scratch/widgets/spec.md',
      model: 'zai-glm-5-3',
      outcome: 'downgrade',
      reason_code: null,
      repository: 'github.com/jorisjonkers-dev/agent-kit',
      role: 'refuter',
      run_id: r.run.run_id,
      severity: 'major',
      stage: 'spec',
      status: 'ok',
    })
    expect(JSON.stringify(line)).not.toContain('nothing contradicts')
  })

  it('writes a skip to the ledger with no client', async () => {
    const r = await reviewed([critical()], { families: ['mistral', 'zhipu'], orchestrator: 'zhipu' })

    await r.h.commands.refute.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })

    expect((await ledgerLines(r.h)).find((entry) => entry.type === 'refute')).toMatchObject({
      answer: null,
      client: null,
      outcome: 'skipped',
      reason_code: 'no-eligible-client',
    })
  })

  it("counts the refuter's spend against the run's flow cap", async () => {
    const r = await reviewed([critical()])

    await refuted(r, vibe('critic-glm', NOTHING, 0.042))

    const invocation = (await ledgerLines(r.h)).find((entry) => entry.type === 'invocation' && entry.critic_id === 'r-F1-vibe-glm')
    expect(invocation).toMatchObject({ charged_usd: 0.042, flow: '.scratch/widgets/spec.md', kind: 'vibe', run_id: r.run.run_id })
  })

  it("charges the refuter the cost computed from its tokens, with Vibe's own figure beside it", async () => {
    const clients = { ...ROUTING.clients, 'vibe-glm': { ...ROUTING.clients['vibe-glm'], prices: { cached_input: 0.14, input: 1.4, output: 4.4 } } }
    const r = await reviewed([critical()], { routing: { ...ROUTING, clients } })
    // 100,000 input tokens, 80,000 of them cached, and 10,000 output: 20,000 × 1.4 + 80,000 × 0.14 + 10,000 × 4.4, over a million.
    const usage = { cached_input_tokens: 80_000, input_tokens: 100_000, output_tokens: 10_000 }

    const result = await refuted(r, { ...vibe('critic-glm', NOTHING), exportJson: { ...isolatedExport(), cost_usd: 0.2, usage } })

    const invocation = (await ledgerLines(r.h)).find((entry) => entry.type === 'invocation' && entry.critic_id === 'r-F1-vibe-glm')
    expect(invocation?.charged_usd).toBeCloseTo(0.0832, 9)
    expect(invocation?.vibe_cost_usd).toBe(0.2)
    const refuter = result.results[0] !== undefined && 'refuter' in result.results[0] ? result.results[0].refuter : undefined
    expect(refuter?.cost_usd).toBeCloseTo(0.0832, 9)
    expect(refuter?.vibe_cost_usd).toBe(0.2)
  })

  it("leaves the flow's next critics unlaunched once the refuter's spend reaches the cap", async () => {
    const flow = '.scratch/widgets/spec.md'
    const r = await reviewed([critical()], { ledger: new SyntheticLedger().invocation({ charged_usd: 4.9, flow, kind: 'vibe' }) })
    // Fan-out spent $0.0246; this refuter's $0.08 takes the flow past its $5 cap.
    await refuted(r, vibe('critic-glm', NOTHING, 0.08))
    r.h.clock.advance(1)

    const next = await r.h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: r.h.repo, spec: flow, stage: 'spec' })
    if (next.status !== 'ready') throw new Error('not ready')
    const fan = await r.h.commands.fanOut.run({ run_dir: next.run_dir })

    expect(fan.critics.map((critic) => critic.status)).toEqual(['budget-skip', 'budget-skip'])
  })

  it('counts only the criticals still accepted after the round toward a second round', async () => {
    const four = ['F1', 'F2', 'F3', 'F4'].map((id, index) =>
      critical({ id, location: `User story ${String(index + 1)}`, evidence: [{ file: 'artifact/spec.md', kind: 'quote', value: SPEC_CANARY }] }),
    )
    const before = await reviewed(four)
    const unrefuted = await before.h.commands.record.run({ run_dir: before.run.run_dir, verdicts: accepted('F1', 'F2', 'F3', 'F4') })
    expect(unrefuted.second_round_due).toBe(true)

    const r = await reviewed(four)
    r.h.process.queue(
      vibe('critic-glm', { findings: [answer('refute')] }),
      vibe('critic-glm', NOTHING),
      vibe('critic-glm', NOTHING),
      vibe('critic-glm', NOTHING),
    )
    const result = await r.h.commands.refute.run({ run_dir: r.run.run_dir, verdicts: accepted('F1', 'F2', 'F3', 'F4') })
    expect(result.counts).toMatchObject({ 'no-objection': 3, refute: 1 })

    const recorded = await r.h.commands.record.run({ run_dir: r.run.run_dir, verdicts: accepted('F1', 'F2', 'F3', 'F4') })
    expect(recorded.counts).toMatchObject({ accepted: 3, rejected: 1 })
    expect(recorded.second_round_due).toBe(false)
  })

  it('writes its results beside the run, once, and only before record', async () => {
    const r = await reviewed([critical()])

    const result = await refuted(r, vibe('critic-glm', NOTHING))

    expect(result.refute_path).toBe(join(r.run.run_dir, 'refute.json'))
    expect(JSON.parse(await readFile(result.refute_path, 'utf8'))).toEqual(JSON.parse(JSON.stringify(result)))
    await expect(r.h.commands.refute.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })).rejects.toMatchObject({
      code: 'already-refuted',
    })
    await r.h.commands.record.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })
    await r.h.removeFile(result.refute_path)
    await expect(r.h.commands.refute.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })).rejects.toMatchObject({
      code: 'already-recorded',
    })
  })

  it.each([
    ['an unknown finding', [{ finding_id: 'F9', verdict: 'accepted' }], /unknown finding F9/],
    ['two verdicts for one finding', [...accepted('F1'), ...accepted('F1')], /more than one verdict for F1/],
  ])('refuses %s', async (_name, verdicts, message) => {
    const r = await reviewed([critical()])

    await expect(r.h.commands.refute.run({ run_dir: r.run.run_dir, verdicts })).rejects.toMatchObject({ code: 'invalid-input', message })
  })

  it('refuses to accept a finding rejected as fabricated evidence', async () => {
    const r = await reviewed([critical({ evidence: [{ file: 'artifact/spec.md', kind: 'quote', value: 'Not in the spec at all.' }] })])

    await expect(r.h.commands.refute.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })).rejects.toMatchObject({
      code: 'invalid-input',
      message: /fabricated-evidence/,
    })
  })

  it('reads a refute file record cannot match to the run as an invalid run', async () => {
    const r = await reviewed([critical()])
    const result = await refuted(r, vibe('critic-glm', NOTHING))
    const file = JSON.parse(await readFile(result.refute_path, 'utf8')) as { results: { finding_id: string }[] }
    file.results[0] = { ...file.results[0], finding_id: 'F9' }
    await r.h.writeText(result.refute_path, JSON.stringify(file))

    await expect(r.h.commands.record.run({ run_dir: r.run.run_dir, verdicts: accepted('F1') })).rejects.toMatchObject({
      code: 'invalid-run',
      message: /F9/,
    })
  })
})
