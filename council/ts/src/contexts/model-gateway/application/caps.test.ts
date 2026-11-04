import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { describe, expect, it } from 'vitest'

import { FakeFiles, FakeLedger } from '../../../../test/model-gateway/harness.js'
import { reservationRecord } from '../domain/caps.js'

import type { GatewayConfig } from '../domain/config.js'
import type { RunInput } from '../domain/run.js'
import { CapGuard, heldCostClass } from './caps.js'
import type { ModelGatewayDependencies } from './ports.js'

const CONFIG = { clients: {}, settings: { output_allowance_tokens: 1000 } } as unknown as GatewayConfig
const INPUT = { client: 'mystery', model: 'm', prompt: 'hi', workflow: 'w' } as unknown as RunInput

describe('heldCostClass', () => {
  it('refuses a client the configuration does not declare, rather than holding it to nothing', () => {
    expect(() => heldCostClass(INPUT, CONFIG)).toThrow(expect.objectContaining({ code: 'unknown-client' }) as unknown)
    expect(() => heldCostClass(INPUT, CONFIG)).toThrow(/mystery/)
  })

  it('refuses it before the guard estimates the run', () => {
    expect(() => new CapGuard({} as ModelGatewayDependencies).estimate(INPUT, undefined, CONFIG)).toThrow(
      expect.objectContaining({ code: 'unknown-client' }) as unknown,
    )
  })
})

describe('releaseUnrun', () => {
  const now = new Date('2026-11-04T10:00:00.000Z')

  async function held(runDir: string, ledger: FakeLedger) {
    const record = reservationRecord({
      client: 'codex',
      cost_class: 'metered-employer',
      estimate_usd: 1,
      model: 'gpt-5.5',
      reserved_at: now,
      run_dir: runDir,
      run_id: 'r1',
      time_limit_s: 900,
      workflow: 'w',
    })
    await ledger.append(record)
    return new CapGuard({ files: new FakeFiles(), ledger } as unknown as ModelGatewayDependencies)
  }

  it('settles a run that left a result at what the result says it cost, never at zero', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'caps-'))
    try {
      await writeFile(join(dir, 'result.json'), JSON.stringify({ cost_usd: 7 }))
      const ledger = new FakeLedger()
      const guard = await held(dir, ledger)

      await guard.releaseUnrun([ledger.records[0] as never], now)

      expect(ledger.records[1]).toMatchObject({ cost_usd: 7, kind: 'settled', reason: 'expired', run_id: 'r1' })
    } finally {
      await rm(dir, { force: true, recursive: true })
    }
  })

  it('settles a run that left no result at 0, as one that never ran, and leaves a settled one alone', async () => {
    const ledger = new FakeLedger()
    const guard = await held('/nowhere', ledger)
    const record = ledger.records[0] as never

    await guard.releaseUnrun([record], now)
    await guard.releaseUnrun([record], now)

    expect(ledger.records.slice(1)).toMatchObject([{ cost_usd: 0, kind: 'settled', reason: 'not-run' }])
  })
})
