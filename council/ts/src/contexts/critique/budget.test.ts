import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  agyFindings,
  fenced,
  finding,
  harness,
  isolatedExport,
  vibeStdout,
  type AgyScript,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { OrchestratorFamily, PrepareResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

interface Flow {
  readonly families?: readonly string[]
  readonly orchestrator?: OrchestratorFamily
  readonly vibe?: VibeScript[]
  readonly agy?: AgyScript[]
  readonly platform?: NodeJS.Platform
  readonly routing?: unknown
}

async function opened(flow: Flow): Promise<Harness> {
  const h = await harness({
    agyScripts: flow.agy ?? [],
    platform: flow.platform ?? 'darwin',
    scripts: flow.vibe ?? [],
    ...(flow.routing === undefined ? {} : { routing: flow.routing }),
  })
  open.push(h)
  await h.commands.allow.run({ families: flow.families ?? ['mistral', 'google', 'zhipu'], repo: h.repo })
  return h
}

async function prepare(h: Harness, flow: Flow = {}, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const run = await h.commands.prepare.run({
    orchestrator_family: flow.orchestrator ?? 'anthropic',
    repo: h.repo,
    spec: '.scratch/widgets/spec.md',
    stage: 'spec',
    ...extra,
  })
  if (run.status !== 'ready') throw new Error('not ready')
  return run
}

const VIBE = 'c1-vibe-mistral-large'
const AGY = 'c2-agy-gemini-flash-high'

const vibeOk = (agent: string, cost = 0.0123): VibeScript => ({
  agent,
  exitCode: 0,
  exportJson: { ...isolatedExport(), cost_usd: cost },
  stdout: vibeStdout(fenced({ findings: [finding()] })),
})

const agyOk = (): AgyScript => agyFindings({ findings: [], no_findings_reason: 'Walked every failure path.' })

describe('the outer timeout', () => {
  it('kills a Vibe critic that never returns and records it as timed out', async () => {
    const h = await opened({ agy: [agyOk()], families: ['mistral', 'google'], vibe: [{ agent: 'critic-mistral-large', exitCode: 0, stdout: '', timedOut: true }] })
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(h.process.commands[0]?.timeoutMs).toBe((600 + 60) * 1000)
    expect(result.critics.find((critic) => critic.critic_id === VIBE)).toMatchObject({ failure: 'timeout', status: 'failed' })
  })

  it('kills an Antigravity critic that never returns and records it as timed out', async () => {
    const h = await opened({ agy: [{ stdout: '', timedOut: true }], families: ['mistral', 'google'], vibe: [vibeOk('critic-mistral-large')] })
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(h.agy.launches[0]?.timeoutMs).toBe((600 + 60) * 1000)
    expect(result.critics.find((critic) => critic.critic_id === AGY)).toMatchObject({ failure: 'timeout', status: 'failed' })
  })
})

const timedOutVibe = (agent: string): VibeScript => ({ agent, exitCode: 0, stdout: '', timedOut: true })

describe('a fallback retry for a failed critic', () => {
  it('retries a timed-out critic once through its first fallback, with the same role and lenses', async () => {
    const h = await opened({ agy: [agyOk()], vibe: [timedOutVibe('critic-mistral-large'), vibeOk('critic-glm')] })
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.critics.map((critic) => [critic.critic_id, critic.status, critic.lenses, critic.retry_of])).toEqual([
      [VIBE, 'failed', ['fidelity', 'testability'], undefined],
      [AGY, 'ok', ['failure-modes', 'security'], undefined],
      [`${VIBE}-retry-vibe-glm`, 'ok', ['fidelity', 'testability'], VIBE],
    ])
    expect(result.degraded).toBe(false)
  })

  it('runs the retry in its own copy of the packet', async () => {
    const h = await opened({ agy: [agyOk()], vibe: [timedOutVibe('critic-mistral-large'), vibeOk('critic-glm')] })
    const run = await prepare(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(h.process.commands.map((command) => command.cwd)).toEqual([
      `${run.run_dir}/critics/${VIBE}/packet`,
      `${run.run_dir}/critics/${VIBE}-retry-vibe-glm/packet`,
    ])
  })

  it('retries a failed Antigravity critic through Vibe', async () => {
    const h = await opened({ agy: [{ exitCode: 3, stdout: '' }], vibe: [vibeOk('critic-mistral-large'), vibeOk('critic-glm')] })
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.critics.map((critic) => [critic.critic_id, critic.status])).toEqual([
      [VIBE, 'ok'],
      [AGY, 'failed'],
      [`${AGY}-retry-vibe-glm`, 'ok'],
    ])
  })

  it('gives a fallback that fails no second retry', async () => {
    const h = await opened({ agy: [agyOk()], vibe: [timedOutVibe('critic-mistral-large'), timedOutVibe('critic-glm')] })
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.critics.map((critic) => [critic.critic_id, critic.status])).toEqual([
      [VIBE, 'failed'],
      [AGY, 'ok'],
      [`${VIBE}-retry-vibe-glm`, 'failed'],
    ])
  })

  it('never retries into a family another critic of the stage already has', async () => {
    const h = await opened({ vibe: [timedOutVibe('critic-mistral-large'), vibeOk('critic-glm')] })
    const run = await prepare(h, { orchestrator: 'google' })

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.critics.map((critic) => [critic.critic_id, critic.status])).toEqual([
      [VIBE, 'failed'],
      ['c2-vibe-glm', 'ok'],
    ])
  })

  it('never retries into a family the allowlist does not permit', async () => {
    const h = await opened({ agy: [agyOk()], families: ['mistral', 'google'], vibe: [timedOutVibe('critic-mistral-large')] })
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.critics.map((critic) => critic.critic_id)).toEqual([VIBE, AGY])
  })

  it('gives a critic that violated read-only no retry', async () => {
    const h = await opened({
      agy: [agyOk()],
      vibe: [
        {
          ...vibeOk('critic-mistral-large'),
          effect: async ({ cwd }) => {
            await writeFile(join(cwd, 'pwned.md'), 'x')
          },
        },
      ],
    })
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.critics.map((critic) => [critic.critic_id, critic.status])).toEqual([
      [VIBE, 'violated-readonly'],
      [AGY, 'ok'],
    ])
  })

  it('says no critique ran when every critic and its fallback failed', async () => {
    const h = await opened({
      agy: [{ exitCode: 3, stdout: '' }],
      vibe: [timedOutVibe('critic-mistral-large'), timedOutVibe('critic-glm')],
    })
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(summary.split('\n').slice(0, 3)).toEqual([`# Critique: spec, run ${run.run_id}`, '', 'degraded: no critique ran'])
  })

  it('names each fallback retry in the summary', async () => {
    const h = await opened({ agy: [agyOk()], vibe: [timedOutVibe('critic-mistral-large'), vibeOk('critic-glm')] })
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect(summary).toContain(`- vibe-glm (glm-5-3, zhipu): ok, 1 finding, $0.0123, fallback for ${VIBE} (timeout)`)
  })
})

const REPOSITORY = 'github.com/jorisjonkers-dev/agent-kit'
const SPEC = '.scratch/widgets/spec.md'

function ledgerFile(h: Harness): string {
  return join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl')
}

async function ledger(h: Harness): Promise<Record<string, unknown>[]> {
  const text = await h.readText(ledgerFile(h)).catch(() => '')
  return text
    .split('\n')
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line) as Record<string, unknown>)
}

async function invocations(h: Harness): Promise<Record<string, unknown>[]> {
  return (await ledger(h)).filter((line) => line.type === 'invocation')
}

/** Earlier critic launches on this machine, as fan-out logs them. */
async function seedLedger(h: Harness, lines: readonly Record<string, unknown>[]): Promise<void> {
  const invocation = (line: Record<string, unknown>) => ({
    charged_usd: 0,
    client: 'vibe-mistral-large',
    critic_id: 'c1-vibe-mistral-large',
    family: 'mistral',
    flow: SPEC,
    kind: 'vibe',
    model: 'mistral-large-4-0-3',
    recorded_at: '2026-10-30T09:00:00.000Z',
    repository: REPOSITORY,
    run_id: '20261030T090000Z-spec',
    stage: 'spec',
    status: 'ok',
    type: 'invocation',
    ...line,
  })
  await h.writeText(ledgerFile(h), lines.map((line) => `${JSON.stringify(invocation(line))}\n`).join(''))
}

async function chargedFor(script: VibeScript): Promise<unknown> {
  const h = await opened({ families: ['mistral'], vibe: [script] })
  const run = await prepare(h)
  await h.commands.fanOut.run({ run_dir: run.run_dir })
  return (await invocations(h))[0]?.charged_usd
}

describe('what a critic launch is charged', () => {
  it('logs every launch in the ledger as it ends, with metadata only', async () => {
    const h = await opened({ agy: [agyOk()], families: ['mistral', 'google'], vibe: [vibeOk('critic-mistral-large', 0.0421)] })
    const run = await prepare(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    // Critics run in parallel, so each line lands when its critic ends.
    const lines = (await invocations(h)).sort((a, b) => String(a.critic_id).localeCompare(String(b.critic_id)))
    expect(lines).toEqual([
      expect.objectContaining({ charged_usd: 0.0421, client: 'vibe-mistral-large', critic_id: VIBE, flow: SPEC, kind: 'vibe', status: 'ok' }),
      expect.objectContaining({ charged_usd: 0, client: 'agy-gemini-flash-high', critic_id: AGY, flow: SPEC, kind: 'agy', status: 'ok' }),
    ])
  })

  it('charges a Vibe critic the cost its export reports, even past its cap', async () => {
    const charged = await chargedFor({
      agent: 'critic-mistral-large',
      exitCode: 3,
      exportJson: { ...isolatedExport(), cost_usd: 0.83, outcome: 'price_limit' },
      stdout: '',
    })

    expect(charged).toBe(0.83)
  })

  it('charges a Vibe critic stopped at its deadline its price cap, since Vibe reports no cost for it', async () => {
    const charged = await chargedFor({
      agent: 'critic-mistral-large',
      exitCode: 3,
      exportJson: { ...isolatedExport(), cost_usd: 0, outcome: 'deadline' },
      stdout: '',
    })

    expect(charged).toBe(0.5)
  })

  it('charges a Vibe critic on an unpriced model its price cap', async () => {
    const charged = await chargedFor({
      ...vibeOk('critic-mistral-large', 0),
      exportJson: { ...isolatedExport(), cost_usd: 0, usage: { input_tokens: 7081, output_tokens: 14 } },
    })

    expect(charged).toBe(0.5)
  })

  it('charges a Vibe critic that used no tokens nothing', async () => {
    const charged = await chargedFor({
      agent: 'critic-mistral-large',
      exitCode: 2,
      exportJson: { ...isolatedExport(), cost_usd: 0, outcome: 'infrastructure_failure', usage: { input_tokens: 0, output_tokens: 0 } },
      stdout: '',
    })

    expect(charged).toBe(0)
  })

  it('charges a Vibe critic that wrote no export its price cap', async () => {
    expect(await chargedFor(timedOutVibe('critic-mistral-large'))).toBe(0.5)
  })

  it('puts the cost and the charge into the run line of the ledger', async () => {
    const h = await opened({ families: ['mistral'], vibe: [vibeOk('critic-mistral-large', 0.0421)] })
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect((await ledger(h)).find((line) => line.type === 'run')).toMatchObject({ charged_usd: 0.0421, cost_usd: 0.0421, flow: SPEC })
  })
})

describe('the per-flow Vibe cap', () => {
  it('skips every Vibe critic once the flow has spent its cap, and lets the rest run', async () => {
    const h = await opened({ agy: [agyOk()], families: ['mistral', 'google'] })
    await seedLedger(h, [{ charged_usd: 3.5 }, { charged_usd: 1.5 }])
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(h.process.commands).toEqual([])
    expect(result.critics.map((critic) => [critic.critic_id, critic.status])).toEqual([
      [VIBE, 'budget-skip'],
      [AGY, 'ok'],
    ])
  })

  it('lists a skipped critic in the summary', async () => {
    const h = await opened({ agy: [agyOk()], families: ['mistral', 'google'] })
    await seedLedger(h, [{ charged_usd: 5.25 }])
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(summary).toContain("- vibe-mistral-large (mistral-large-4-0-3, mistral): budget-skip: the flow's Vibe spend $5.2500 has reached its $5.00 cap")
  })

  it('records a skipped critic in the ledger without launching it', async () => {
    const h = await opened({ agy: [agyOk()], families: ['mistral', 'google'] })
    await seedLedger(h, [{ charged_usd: 5 }])
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    const lines = await ledger(h)
    expect(lines.filter((line) => line.type === 'invocation' && line.run_id === run.run_id).map((line) => line.client)).toEqual([
      'agy-gemini-flash-high',
    ])
    expect(lines.find((line) => line.type === 'run' && line.critic_id === VIBE)).toMatchObject({
      charged_usd: 0,
      exit_code: null,
      status: 'budget-skip',
    })
  })

  it('counts only the spend of this flow and this repository', async () => {
    const h = await opened({ families: ['mistral'], vibe: [vibeOk('critic-mistral-large')] })
    await seedLedger(h, [
      { charged_usd: 4, flow: 'github.com/jorisjonkers-dev/agent-kit#12' },
      { charged_usd: 4, repository: 'github.com/jorisjonkers-dev/other' },
      { charged_usd: 4, kind: 'agy' },
      { charged_usd: 0.9 },
    ])
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.critics.map((critic) => critic.status)).toEqual(['ok'])
  })

  it('names the flow from the input when the caller gives one', async () => {
    const h = await opened({ families: ['mistral'] })
    await seedLedger(h, [{ charged_usd: 5, flow: 'github.com/jorisjonkers-dev/agent-kit#167' }])
    const run = await prepare(h, {}, { flow: 'github.com/jorisjonkers-dev/agent-kit#167' })

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.critics.map((critic) => critic.status)).toEqual(['budget-skip'])
  })

  it('checks the spend again before a fallback retry, and skips the retry once the cap is reached', async () => {
    const h = await opened({ agy: [agyOk()], vibe: [timedOutVibe('critic-mistral-large')] })
    await seedLedger(h, [{ charged_usd: 4.75 }])
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.critics.map((critic) => [critic.critic_id, critic.status])).toEqual([
      [VIBE, 'failed'],
      [AGY, 'ok'],
      [`${VIBE}-retry-vibe-glm`, 'budget-skip'],
    ])
  })

  it('says no critique ran when every critic was skipped', async () => {
    const h = await opened({ families: ['mistral'] })
    await seedLedger(h, [{ charged_usd: 6 }])
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(summary).toContain('degraded: no critique ran')
  })
})
