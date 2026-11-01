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
