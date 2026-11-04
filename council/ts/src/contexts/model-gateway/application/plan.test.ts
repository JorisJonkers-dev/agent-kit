import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { FakeEnv, FakeFiles, FakeLedger, FixedClock } from '../../../../test/model-gateway/harness.js'
import type { LedgerRecord } from '../domain/ledger.js'
import { readPlanLimit } from './plan.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

/** The shipped defaults, which the bundle reads from its own folder. */
const SHIPPED_DEFAULTS = join(import.meta.dirname, '../../../../../../skills/workstation/model-gateway/defaults.json')

interface Machine {
  readonly deps: Parameters<typeof readPlanLimit>[0]
  writeOverride(value: unknown): Promise<void>
}

/** A machine with a home, the shipped defaults and a ledger a test fills. */
async function machine(records: readonly LedgerRecord[] = []): Promise<Machine> {
  const dir = await mkdtemp(join(tmpdir(), 'plan-limit-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const home = join(dir, 'home')
  const files = new FakeFiles()
  const ledger = new FakeLedger()
  for (const record of records) await ledger.append(record)
  return {
    deps: { clock: new FixedClock(new Date('2026-11-04T10:52:15.000Z')), defaultsPath: SHIPPED_DEFAULTS, env: new FakeEnv({ HOME: home }), files, ledger },
    writeOverride: (value) => files.writeText(join(home, '.config', 'agent-kit', 'model-gateway.json'), JSON.stringify(value)),
  }
}

/** One ledger record of a Vibe run that cost `cost` at `at`. */
function vibeRun(at: string, cost: number): LedgerRecord {
  return {
    at,
    client: 'vibe',
    cost_usd: cost,
    duration_s: 1,
    family: 'mistral',
    mode: 'read-only',
    model: 'mistral-large-latest',
    outcome: 'ok',
    repository: null,
    run_id: `run-${at}`,
    tokens: { cached: 0, input: 0, output: 0 },
    workflow: 'spec-293',
  }
}

describe('readPlanLimit', () => {
  it('leaves a client the machine file gives no plan limit unwatched', async () => {
    const m = await machine([vibeRun('2026-11-04T09:00:00.000Z', 40)])

    expect(await readPlanLimit(m.deps, 'vibe')).toEqual({ client: 'vibe', warnings: [], windows: [] })
  })

  it('reports each window of the plan with what the ledger says is spent in it', async () => {
    const m = await machine([vibeRun('2026-11-04T09:00:00.000Z', 3), vibeRun('2026-10-20T09:00:00.000Z', 20)])
    await m.writeOverride({ clients: { vibe: { plan_limits: [{ limit_usd: 12, window: '5h' }, { limit_usd: 60, window: 'month' }] } } })

    expect(await readPlanLimit(m.deps, 'vibe')).toEqual({
      client: 'vibe',
      warnings: [],
      windows: [
        { limit_usd: 12, ratio: 0.25, spent_usd: 3, state: 'ok', window: '5h' },
        { limit_usd: 60, ratio: 0.3833, spent_usd: 23, state: 'ok', window: 'month' },
      ],
    })
  })

  it('warns in the gateway’s own words at or above the warning ratio', async () => {
    const m = await machine([vibeRun('2026-11-04T09:00:00.000Z', 10)])
    await m.writeOverride({ clients: { vibe: { plan_limits: [{ limit_usd: 12, window: '5h' }] } } })

    expect(await readPlanLimit(m.deps, 'vibe')).toEqual({
      client: 'vibe',
      warnings: ['vibe has used 83% of its 5h plan limit ($10.00 of $12.00)'],
      windows: [{ limit_usd: 12, ratio: 0.8333, spent_usd: 10, state: 'warning', window: '5h' }],
    })
  })

  it('says a client at 100% of a window cannot start', async () => {
    const m = await machine([vibeRun('2026-11-04T09:00:00.000Z', 12)])
    await m.writeOverride({ clients: { vibe: { plan_limits: [{ limit_usd: 12, window: '5h' }] } } })

    expect(await readPlanLimit(m.deps, 'vibe')).toEqual({
      client: 'vibe',
      unable_to_start: 'at 100% of its 5h plan limit ($12.00 of $12.00)',
      warnings: ['vibe has used 100% of its 5h plan limit ($12.00 of $12.00)'],
      windows: [{ limit_usd: 12, ratio: 1, spent_usd: 12, state: 'exhausted', window: '5h' }],
    })
  })

  it('counts a window’s runs when the plan limit names runs instead of dollars', async () => {
    const m = await machine([vibeRun('2026-11-04T09:00:00.000Z', 0), vibeRun('2026-11-04T08:00:00.000Z', 0)])
    await m.writeOverride({ clients: { vibe: { plan_limits: [{ limit_runs: 2, window: 'day' }] } } })

    expect(await readPlanLimit(m.deps, 'vibe')).toEqual({
      client: 'vibe',
      unable_to_start: 'at 100% of its day plan limit (2 of 2 runs)',
      warnings: ['vibe has used 100% of its day plan limit (2 of 2 runs)'],
      windows: [{ limit_runs: 2, ratio: 1, runs: 2, state: 'exhausted', window: 'day' }],
    })
  })

  it('names a client the defaults do not know as unwatched', async () => {
    const m = await machine()

    expect(await readPlanLimit(m.deps, 'nobody')).toEqual({ client: 'nobody', warnings: [], windows: [] })
  })
})
