import { describe, expect, it } from 'vitest'

import { FakeLedger, FixedClock } from '../../../../test/model-gateway/harness.js'
import { readBatchInput } from '../domain/batch.js'
import type { GatewayConfig } from '../domain/config.js'
import { reservationRecord } from '../domain/caps.js'
import { BatchCommand } from './batch.js'
import type { ModelGatewayDependencies } from './ports.js'
import type { Reservation } from './caps.js'
import type { RunCommand } from './run.js'

const CONFIG = { settings: { max_parallel_per_client: 3, summary_max_chars: 1500 } } as unknown as GatewayConfig

/** What the single-delegation path answers for a delegation, by its prompt. */
type Answer = (prompt: string) => Record<string, unknown>

function plan(delegations: readonly Record<string, unknown>[]) {
  return readBatchInput(
    {
      delegations: delegations.map((entry) => ({ client: 'claude', escalate: false, mode: 'read-only', model: 'claude-sonnet-5-5', prompt: `TASK-${String(entry.id)}`, ...entry })),
      workflow: 'w',
    },
    50,
  )
}

function setup(answer: Answer) {
  const ledger = new FakeLedger()
  const prompts: string[] = []
  const runs = {
    delegation: (input: { prompt: string }) => {
      prompts.push(input.prompt)
      return Promise.resolve(answer(input.prompt))
    },
  } as unknown as RunCommand
  const command = new BatchCommand({ clock: new FixedClock(), ledger } as unknown as ModelGatewayDependencies, runs)
  const execute = (input: ReturnType<typeof plan>, held: ReadonlyMap<string, Reservation> = new Map()) =>
    (command as unknown as { execute(i: unknown, c: unknown, h: unknown, b: string): Promise<ReadonlyMap<string, Record<string, unknown>>> }).execute(
      input,
      CONFIG,
      held,
      'batch-1',
    )
  return { execute, ledger, prompts }
}

function done(output: string | null, summary = 'fine'): Record<string, unknown> {
  return { attempts: [], client: 'claude', cost_usd: 0, model: 'claude-sonnet-5-5', output_path: output, status: 'ok', summary }
}

describe('BatchCommand: running a plan', () => {
  it('counts a delegation that is done without an output file as satisfied, and tells its dependent there is no file', async () => {
    const { execute, prompts } = setup((prompt) => (prompt.includes('TASK-a') ? done(null) : done('/runs/b/output.json')))

    const reports = await execute(plan([{ id: 'a' }, { depends_on: ['a'], id: 'b' }]))

    expect([...reports.values()].map((report) => report.status)).toEqual(['done', 'done'])
    expect(prompts[1]).toContain('- a: no output file')
  })

  it('reports every delegation that can never start as skipped, could-not-start, and gives its reservation back', async () => {
    const { execute, ledger, prompts } = setup(() => done('/runs/x/output.json'))
    const record = reservationRecord({
      client: 'claude',
      cost_class: 'metered-personal',
      estimate_usd: 0.5,
      model: 'claude-sonnet-5-5',
      reserved_at: new Date('2026-11-04T10:52:15.000Z'),
      run_dir: '/nowhere',
      run_id: 'r-b',
      time_limit_s: 900,
      workflow: 'w',
    })
    await ledger.append(record)

    const reports = await execute(plan([{ id: 'a' }, { depends_on: ['c'], id: 'b' }, { depends_on: ['b'], id: 'c' }]), new Map([['b', { estimate: { output_tokens: 1, prompt_tokens: 1, usd: 0.5 }, record }]]))

    expect([...reports.values()].map((report) => `${String(report.id)}:${String(report.status)}:${typeof report.skipped_because === 'string' ? report.skipped_because : ''}`)).toEqual([
      'a:done:',
      'b:skipped:could-not-start',
      'c:skipped:could-not-start',
    ])
    expect(prompts).toHaveLength(1)
    expect(ledger.records[1]).toMatchObject({ kind: 'settled', reason: 'not-run', run_id: 'r-b' })
  })
})
