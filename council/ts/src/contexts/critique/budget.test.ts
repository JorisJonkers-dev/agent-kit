import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  agyFindings,
  ASSETS_ROOT,
  fenced,
  finding,
  harness,
  isolatedExport,
  seedGatewayLedger,
  vibeStdout,
  type AgyScript,
  type Harness,
  type VibeScript,
  type VibeSignIn,
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
  readonly vibeOnPath?: boolean
  readonly vibeSignIn?: VibeSignIn
  readonly vibeAgents?: readonly string[]
  /** This machine's model gateway file, where Vibe's plan limits are declared. */
  readonly gatewayOverride?: unknown
}

async function opened(flow: Flow): Promise<Harness> {
  const h = await harness({
    agyScripts: flow.agy ?? [],
    platform: flow.platform ?? 'darwin',
    scripts: flow.vibe ?? [],
    ...(flow.routing === undefined ? {} : { routing: flow.routing }),
    ...(flow.vibeOnPath === undefined ? {} : { vibeOnPath: flow.vibeOnPath }),
    ...(flow.vibeSignIn === undefined ? {} : { vibeSignIn: flow.vibeSignIn }),
    ...(flow.vibeAgents === undefined ? {} : { vibeAgents: flow.vibeAgents }),
    ...(flow.gatewayOverride === undefined ? {} : { gatewayOverride: flow.gatewayOverride }),
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

    expect(summary).toContain(`- vibe-glm (zai-glm-5-3, zhipu): ok, 1 finding, $0.0123, fallback for ${VIBE} (timeout)`)
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
    model: 'mistral-large-4-0',
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

async function chargedFor(script: VibeScript, routing?: unknown): Promise<unknown> {
  const h = await opened({ families: ['mistral'], vibe: [script], ...(routing === undefined ? {} : { routing }) })
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

  it('charges a Vibe critic on a model neither routing nor Vibe prices its price cap', async () => {
    const charged = await chargedFor(
      { ...vibeOk('critic-mistral-large', 0), exportJson: { ...isolatedExport(), cost_usd: 0, usage: { input_tokens: 7081, output_tokens: 14 } } },
      await unpricedRouting(),
    )

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

/**
 * The token counts of a real Mistral Large critic of 2026-11-02 (run
 * 20261102T105318Z-ticket-review, c2-vibe-mistral-large): 84% of its input
 * was cached, and Vibe, with no cached price in its model table, priced it
 * all at the full rate.
 */
const CACHED_RUN_USAGE = { cached_input_tokens: 742_656, input_tokens: 881_212, output_tokens: 22_977, total_tokens: 904_189 }
const CACHED_RUN_VIBE_COST = 0.6651000999999999
/** At $0.70 per million uncached input, $0.07 cached and $2.10 output: 138,556 × 0.7 + 742,656 × 0.07 + 22,977 × 2.1, over a million. */
const CACHED_RUN_COST = 0.1972268

const cachedRun = (agent = 'critic-mistral-large'): VibeScript => ({
  ...vibeOk(agent, CACHED_RUN_VIBE_COST),
  exportJson: { ...isolatedExport(), cost_usd: CACHED_RUN_VIBE_COST, usage: CACHED_RUN_USAGE },
})

describe('what a Vibe critic costs, priced by the runner', () => {
  it("charges the cost the runner computes from the export's tokens at routing's prices, cached input at the cached price", async () => {
    const h = await opened({ families: ['mistral'], vibe: [cachedRun()] })
    const run = await prepare(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const [line] = await invocations(h)
    expect(line?.charged_usd).toBeCloseTo(CACHED_RUN_COST, 6)
    expect(line?.vibe_cost_usd).toBe(CACHED_RUN_VIBE_COST)
  })

  it("records the computed cost in the ledger's run line, with Vibe's own figure beside it", async () => {
    const h = await opened({ families: ['mistral'], vibe: [cachedRun()] })
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    const line = (await ledger(h)).find((entry) => entry.type === 'run')
    expect(line?.cost_usd).toBeCloseTo(CACHED_RUN_COST, 6)
    expect(line?.charged_usd).toBeCloseTo(CACHED_RUN_COST, 6)
    expect(line?.vibe_cost_usd).toBe(CACHED_RUN_VIBE_COST)
    expect(summary).toContain('$0.1972')
  })

  it('holds the flow to its cap by the computed cost, not by what Vibe reported', async () => {
    const h = await opened({ families: ['mistral'], vibe: [cachedRun(), vibeOk('critic-mistral-large')] })
    // With Vibe's $0.665 the flow would pass its $5 cap; with the computed $0.197 it stays under it.
    await seedLedger(h, [{ charged_usd: 4.5 }])
    const first = await prepare(h)
    await h.commands.fanOut.run({ run_dir: first.run_dir })
    const second = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: second.run_dir })

    expect(result.critics.map((critic) => critic.status)).toEqual(['ok'])
  })

  it("shows the computed cost in the flow's budget", async () => {
    const h = await opened({ families: ['mistral'], vibe: [cachedRun()] })
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const result = await h.commands.budget.run({ repo: h.repo, spec: SPEC })

    if (result.status === 'refused') throw new Error(result.message)
    expect(result.vibe.spent_usd).toBeCloseTo(CACHED_RUN_COST, 6)
  })

  it("falls back to Vibe's own figure for a client routing gives no prices", async () => {
    const h = await opened({ families: ['mistral'], routing: await unpricedRouting(), vibe: [cachedRun()] })
    const run = await prepare(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect((await invocations(h))[0]).toMatchObject({ charged_usd: CACHED_RUN_VIBE_COST, vibe_cost_usd: CACHED_RUN_VIBE_COST })
  })

  it.each([
    [{ cached_input: 0.07, input: 0, output: 2.1 }, /prices\.input must be above 0/],
    [{ cached_input: 0.07, input: 0.7, output: 0 }, /prices\.output must be above 0/],
    [{ cached_input: -0.01, input: 0.7, output: 2.1 }, /prices\.cached_input must be at least 0/],
    [{ input: 0.7, output: 2.1 }, /prices\.cached_input must be a number/],
  ])('refuses routing prices %j', async (prices, message) => {
    const routing = await shippedRouting()
    const clients = routing.clients as Record<string, Record<string, unknown>>
    const h = await opened({ families: ['mistral'], routing: { ...routing, clients: { ...clients, 'vibe-mistral-large': { ...clients['vibe-mistral-large'], prices } } } })

    await expect(prepare(h)).rejects.toThrow(message)
  })

  it('prices all input at the full rate when the export counts no cached tokens', async () => {
    const charged = await chargedFor({
      ...vibeOk('critic-mistral-large'),
      exportJson: { ...isolatedExport(), cost_usd: 0.9, usage: { input_tokens: 1_000_000, output_tokens: 100_000 } },
    })

    // 1,000,000 × $0.70 + 100,000 × $2.10, over a million.
    expect(charged).toBeCloseTo(0.91, 10)
  })

  it('charges the cap when the export counts more tokens than a finite number', async () => {
    const charged = await chargedFor({
      ...vibeOk('critic-mistral-large'),
      exportJson: { ...isolatedExport(), cost_usd: 0, usage: { input_tokens: Number.POSITIVE_INFINITY, output_tokens: 1 } },
    })
    expect(charged).toBe(0.5)
  })

  it("falls back to Vibe's own figure for an export that counts no tokens", async () => {
    expect(await chargedFor(vibeOk('critic-mistral-large', 0.0421))).toBe(0.0421)
  })
})

describe("Vibe's plan limit", () => {
  /** This machine's gateway file: Vibe's plan allows $5 a day. */
  const VIBE_PLAN = { clients: { vibe: { plan_limits: [{ limit_usd: 5, window: 'day' }] } } }
  const EXHAUSTED = 'at 100% of its day plan limit ($5.00 of $5.00)'

  it('drops every Vibe critic once Vibe is at 100% of its plan, and lets the rest run', async () => {
    const h = await opened({ agy: [agyOk()], families: ['mistral', 'google'], gatewayOverride: VIBE_PLAN })
    await seedGatewayLedger(h, [{ cost_usd: 5 }])
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(h.process.commands).toEqual([])
    expect(result.critics.map((critic) => [critic.critic_id, critic.status])).toEqual([['c1-agy-gemini-flash-high', 'ok']])
    expect(run.dropped[0]).toMatchObject({ client: 'vibe-mistral-large', detail: EXHAUSTED, reason: 'quota' })
  })

  it('names the dropped critic in the summary', async () => {
    const h = await opened({ agy: [agyOk()], families: ['mistral', 'google'], gatewayOverride: VIBE_PLAN })
    await seedGatewayLedger(h, [{ cost_usd: 5.25 }])
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(summary).toContain(
      '- Not run: vibe-mistral-large (mistral): quota (at 100% of its day plan limit ($5.25 of $5.00)); replaced by c1-agy-gemini-flash-high',
    )
  })

  it('records no launch for a dropped critic, in either ledger', async () => {
    const h = await opened({ agy: [agyOk()], families: ['mistral', 'google'], gatewayOverride: VIBE_PLAN })
    await seedGatewayLedger(h, [{ cost_usd: 5 }])
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect((await invocations(h)).map((line) => line.client)).toEqual(['agy-gemini-flash-high'])
    const gateway = (await h.gatewayLedger()).split('\n').filter((line) => line.length > 0).map((line) => JSON.parse(line) as Record<string, unknown>)
    expect(gateway.filter((line) => line.type === 'invocation').map((line) => line.critic_client)).toEqual(['agy-gemini-flash-high'])
  })

  it("counts a delegation's spend toward Vibe's plan, since the plan is the machine's", async () => {
    const h = await opened({ families: ['mistral'], gatewayOverride: VIBE_PLAN })
    await seedGatewayLedger(h, [{ client: 'vibe', cost_usd: 5, mode: 'write', workflow: 'delegate' }])
    const run = await prepare(h)

    expect(run.dropped[0]).toMatchObject({ client: 'vibe-mistral-large', reason: 'quota' })
  })

  it('counts a critic run the gateway ledger holds toward the plan', async () => {
    const h = await opened({ families: ['mistral'], gatewayOverride: VIBE_PLAN, vibe: [vibeOk('critic-mistral-large', 5)] })
    const first = await prepare(h)
    await h.commands.fanOut.run({ run_dir: first.run_dir })
    const second = await prepare(h)

    expect(second.dropped[0]).toMatchObject({ client: 'vibe-mistral-large', detail: EXHAUSTED, reason: 'quota' })
  })

  it('ignores a gateway ledger line that was cut short', async () => {
    const h = await opened({ families: ['mistral'], gatewayOverride: VIBE_PLAN })
    await seedGatewayLedger(h, [{ cost_usd: 5 }])
    await h.writeText(h.gatewayLedgerFile, `${await h.readText(h.gatewayLedgerFile)}{"at":"2026-10-30T09:00:00.000Z","client":"vi`)
    const run = await prepare(h)

    expect(run.dropped[0]).toMatchObject({ client: 'vibe-mistral-large', reason: 'quota' })
  })

  it('takes the plan limit from the machine gateway file, so this machine can lower it', async () => {
    const h = await opened({ families: ['mistral'], gatewayOverride: { clients: { vibe: { plan_limits: [{ limit_usd: 1, window: 'day' }] } } } })
    await seedGatewayLedger(h, [{ cost_usd: 1.2 }])
    const run = await prepare(h)

    expect(run.dropped[0]).toMatchObject({ client: 'vibe-mistral-large', detail: 'at 100% of its day plan limit ($1.20 of $1.00)', reason: 'quota' })
  })

  it('does not retry into Vibe once the plan is exhausted during the run, and prefers a fallback that costs no money', async () => {
    const routing = await shippedRouting()
    const h = await opened({
      agy: [agyOk()],
      families: ['mistral', 'google', 'deepseek'],
      gatewayOverride: VIBE_PLAN,
      routing: {
        ...routing,
        fallbacks: { 'vibe-mistral-large': ['vibe-deepseek-flash', 'agy-gemini-flash-high'] },
        stages: { spec: { ...routing.stages.spec, base: [routing.stages.spec.base[0]] } },
      },
      vibe: [timedOutVibe('critic-mistral-large')],
    })
    // $4.75 spent, and the timed-out critic is charged its $0.50 cap: the plan is spent when the retry is planned.
    await seedGatewayLedger(h, [{ cost_usd: 4.75 }])
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.critics.map((critic) => [critic.critic_id, critic.status])).toEqual([
      [VIBE, 'failed'],
      [`${VIBE}-retry-agy-gemini-flash-high`, 'ok'],
    ])
  })

  it('says no critique ran when every critic was dropped', async () => {
    const h = await opened({ families: ['mistral'], gatewayOverride: VIBE_PLAN })
    await seedGatewayLedger(h, [{ cost_usd: 6 }])
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })

    expect(summary).toContain('degraded: no critique ran')
  })

  it('warns in the summary while Vibe is at or above the warning ratio, and still runs', async () => {
    const h = await opened({ families: ['mistral'], gatewayOverride: VIBE_PLAN, vibe: [vibeOk('critic-mistral-large')] })
    await seedGatewayLedger(h, [{ cost_usd: 4.2 }])
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect(summary).toContain('Vibe plan: vibe has used 84% of its day plan limit ($4.20 of $5.00)')
  })
})

interface ShippedRouting {
  readonly fallbacks: Record<string, string[]>
  readonly stages: { readonly spec: { readonly base: unknown[] } & Record<string, unknown> }
  readonly [key: string]: unknown
}

async function shippedRouting(): Promise<ShippedRouting> {
  return JSON.parse(await readFile(join(ASSETS_ROOT, 'routing.json'), 'utf8')) as ShippedRouting
}

/** The shipped routing with no prices on its Vibe clients, so the runner has only Vibe's own figure. */
async function unpricedRouting(): Promise<ShippedRouting> {
  const routing = await shippedRouting()
  const clients = Object.fromEntries(
    Object.entries(routing.clients as Record<string, Record<string, unknown>>).map(([name, client]) => [
      name,
      Object.fromEntries(Object.entries(client).filter(([key]) => key !== 'prices')),
    ]),
  )
  return { ...routing, clients }
}

/** A decisions run of the shipped routing, which keeps no Antigravity reserve for that stage. */
const DECISIONS = { feature: 'Widgets get a frobnication window.', stage: 'decisions' }

const agyLaunches = (count: number, recordedAt = '2026-10-30T08:00:00.000Z'): Record<string, unknown>[] =>
  Array.from({ length: count }, () => ({
    client: 'agy-gemini-flash-high',
    critic_id: 'c2-agy-gemini-flash-high',
    family: 'google',
    flow: 'github.com/jorisjonkers-dev/agent-kit#12',
    kind: 'agy',
    model: 'gemini-3.8-flash-high',
    recorded_at: recordedAt,
  }))

describe('the Antigravity quota', () => {
  it('still runs Gemini for the spec stage once the reserve threshold is passed', async () => {
    const h = await opened({})
    await seedLedger(h, agyLaunches(10))

    const run = await prepare(h)

    expect(run.critics.map((critic) => critic.critic_id)).toEqual([VIBE, AGY])
  })

  it('falls back from Gemini at a stage the reserve is not kept for, once the reserve threshold is passed', async () => {
    const h = await opened({})
    await seedLedger(h, agyLaunches(10))

    const run = await prepare(h, {}, DECISIONS)

    expect(run.critics.map((critic) => critic.critic_id)).toEqual([VIBE, 'c2-vibe-glm'])
    expect(run.dropped).toEqual([
      {
        client: 'agy-gemini-flash-high',
        detail:
          'antigravity: 10 invocations today have reached the reserve threshold of 10; the rest is kept for spec and feature-review',
        family: 'google',
        reason: 'quota',
        replaced_by: 'c2-vibe-glm',
      },
    ])
  })

  it('runs Gemini at any stage below the reserve threshold', async () => {
    const h = await opened({})
    await seedLedger(h, agyLaunches(9))

    const run = await prepare(h, {}, DECISIONS)

    expect(run.critics.map((critic) => critic.critic_id)).toEqual([VIBE, AGY])
  })

  it('falls back from Gemini at every stage once the daily cap is reached, and the summary names the fallback', async () => {
    const h = await opened({ vibe: [vibeOk('critic-mistral-large'), vibeOk('critic-glm')] })
    await seedLedger(h, agyLaunches(20))
    const run = await prepare(h)
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect(h.agy.launches).toEqual([])
    expect(summary).toContain(
      '- Not run: agy-gemini-flash-high (google): quota (antigravity: 20 invocations today have reached the daily cap of 20); replaced by c2-vibe-glm',
    )
  })

  it('takes the cap and the reserve from routing, so this machine can change them', async () => {
    const h = await opened({})
    await h.writeText(
      join(h.home, '.config', 'agent-kit', 'critique-routing.json'),
      JSON.stringify({ antigravity_quota: { daily_cap: 3, reserve_threshold: 2, reserved_stages: ['feature-review'] } }),
    )
    await seedLedger(h, agyLaunches(2))

    const run = await prepare(h)

    expect(run.dropped).toMatchObject([
      {
        detail: 'antigravity: 2 invocations today have reached the reserve threshold of 2; the rest is kept for feature-review',
        reason: 'quota',
      },
    ])
  })

  it('counts only the invocations of the current day', async () => {
    const h = await opened({})
    await seedLedger(h, agyLaunches(25, '2026-10-29T23:59:59.000Z'))

    const run = await prepare(h)

    expect(run.critics.map((critic) => critic.critic_id)).toEqual([VIBE, AGY])
  })

  it('counts each Gemini launch toward the day', async () => {
    const h = await opened({ agy: [agyOk()], vibe: [vibeOk('critic-mistral-large')] })
    await seedLedger(h, agyLaunches(19))
    const first = await prepare(h)
    await h.commands.fanOut.run({ run_dir: first.run_dir })

    const second = await prepare(h)

    expect(second.dropped).toMatchObject([{ client: 'agy-gemini-flash-high', reason: 'quota' }])
  })

  it('falls back without touching the counter where agy is not set up, and says why', async () => {
    const h = await opened({ platform: 'linux', vibe: [vibeOk('critic-mistral-large'), vibeOk('critic-glm')] })
    const run = await prepare(h)

    await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect((await invocations(h)).map((line) => line.kind)).toEqual(['vibe', 'vibe'])
    expect(run.dropped).toEqual([
      {
        client: 'agy-gemini-flash-high',
        detail: 'antigravity: macOS only; not set up on linux',
        family: 'google',
        reason: 'unavailable',
        replaced_by: 'c2-vibe-glm',
      },
    ])
  })

  it('does not retry through Gemini once the daily cap is reached during the run', async () => {
    const routing = await shippedRouting()
    const h = await opened({
      routing: {
        ...routing,
        fallbacks: { 'vibe-mistral-large': ['agy-gemini-flash-high'] },
        stages: { spec: { ...routing.stages.spec, base: [routing.stages.spec.base[0]] } },
      },
      vibe: [
        {
          ...timedOutVibe('critic-mistral-large'),
          // Another session on this machine uses up the day's quota meanwhile.
          effect: () => seedLedger(h, agyLaunches(20)),
        },
      ],
    })
    const run = await prepare(h)

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(h.agy.launches).toEqual([])
    expect(result.critics.map((critic) => critic.critic_id)).toEqual([VIBE])
  })
})

describe('a Vibe critic that cannot run here', () => {
  it('falls back from a Vibe that is not on PATH before launching it, and says so', async () => {
    const h = await opened({ families: ['mistral', 'google'], vibeOnPath: false })

    const run = await prepare(h)

    expect(run.critics.map((critic) => critic.critic_id)).toEqual(['c1-agy-gemini-flash-high'])
    expect(run.dropped[0]).toEqual({
      client: 'vibe-mistral-large',
      detail: 'vibe: vibe is not on PATH',
      family: 'mistral',
      reason: 'unavailable',
      replaced_by: 'c1-agy-gemini-flash-high',
    })
  })

  it('finds vibe by an absolute path in routing', async () => {
    const routing = await shippedRouting()
    const clients = routing.clients as Record<string, Record<string, unknown>>
    const h = await opened({
      families: ['mistral'],
      routing: { ...routing, clients: { ...clients, 'vibe-mistral-large': { ...clients['vibe-mistral-large'], binary: '/opt/vibe/bin/vibe' } } },
      vibeOnPath: false,
    })

    const run = await prepare(h)

    expect(run.dropped).toEqual([
      expect.objectContaining({ client: 'vibe-mistral-large', detail: 'vibe: /opt/vibe/bin/vibe is not on PATH', reason: 'unavailable' }),
      expect.objectContaining({ client: 'agy-gemini-flash-high', reason: 'family-not-allowed' }),
    ])
  })

  it("names a Vibe that is not signed in in setup's words", async () => {
    const h = await opened({ families: ['mistral', 'google'], vibeSignIn: 'none' })

    const run = await prepare(h)

    expect(run.dropped[0]).toMatchObject({
      client: 'vibe-mistral-large',
      detail: 'vibe: not signed in; run vibe and sign in with your Mistral account, then re-run setup',
      reason: 'unavailable',
    })
  })

  it("asks the Keychain for Vibe's own sign-in item", async () => {
    const h = await opened({ families: ['mistral', 'google'] })

    await prepare(h)

    expect(h.process.checks[0]).toMatchObject({
      args: ['find-generic-password', '-s', 'ai.mistral.vibe', '-a', 'MISTRAL_API_KEY'],
      command: 'security',
    })
  })

  it("counts a key in the personal home's .env as signed in", async () => {
    const h = await opened({ families: ['mistral'], vibeSignIn: 'env-file' })

    const run = await prepare(h)

    expect(run.critics.map((critic) => critic.critic_id)).toEqual([VIBE])
  })

  it("counts a key in the personal home's .env as signed in where there is no Keychain to ask", async () => {
    const h = await opened({ families: ['mistral'], vibeSignIn: 'env-file-no-keychain' })

    const run = await prepare(h)

    expect(run.critics.map((critic) => critic.critic_id)).toEqual([VIBE])
  })

  it('names a critic agent that setup has not installed', async () => {
    const h = await opened({ families: ['mistral', 'google', 'zhipu'], vibeAgents: ['critic-mistral-large'] })

    const run = await prepare(h, {}, { depth: 'deep' })

    expect(run.dropped).toContainEqual(
      expect.objectContaining({
        client: 'vibe-glm',
        detail: 'vibe: the critic agent critic-glm is not installed in ~/.vibe/agents; re-run setup',
        reason: 'unavailable',
      }),
    )
  })
})
