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
