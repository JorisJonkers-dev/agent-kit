import { describe, expect, it } from 'vitest'

import { FakeFiles, FakeLedger, FakeWorkflows } from '../../../../test/model-gateway/harness.js'
import { RESERVATION_GRACE_S } from '../domain/caps.js'
import type { GatewayConfig } from '../domain/config.js'
import type { Grant } from '../domain/grants.js'
import type { WorkflowRecord } from '../domain/workflows.js'
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

describe('CapGuard.reserve under a grant', () => {
  const NOW = new Date('2026-11-04T10:00:00.000Z')

  function grant(expiresInS: number): Grant {
    return {
      answer: 'yes',
      cap_usd: 5,
      client: 'codex',
      expires_at: new Date(NOW.getTime() + expiresInS * 1000).toISOString(),
      id: 'grant-1',
      question: 'may I?',
      workflow: 'w',
      written_at: '2026-11-04T09:00:00.000Z',
    }
  }

  /** A workflow whose 24 hours run out `endsInS` seconds after NOW. */
  function workflow(endsInS: number): WorkflowRecord {
    return { name: 'w', started_at: new Date(NOW.getTime() + endsInS * 1000 - 24 * 3600 * 1000).toISOString() }
  }

  async function reserve(
    held: Grant,
    workflows: readonly WorkflowRecord[] = [],
    timeLimitS = 900,
  ): ReturnType<CapGuard['reserve']> {
    const deps = { files: new FakeFiles(), ledger: new FakeLedger(), workflows: new FakeWorkflows(workflows) } as unknown as ModelGatewayDependencies
    return new CapGuard(deps).reserve({
      cap: { grant: held, kind: 'grant' },
      config: CONFIG,
      costClass: 'metered-employer',
      estimate: { output_tokens: 1, prompt_tokens: 1, usd: 0.1 },
      input: { client: 'codex', model: 'gpt-5.5', prompt: 'hi', time_limit_s: timeLimitS, workflow: 'w' } as unknown as RunInput,
      now: NOW,
      runDir: '/run',
      runId: 'run-1',
    })
  }

  it('keeps the run its own time limit when the grant and the workflow outlast it', async () => {
    const reservation = await reserve(grant(7200), [workflow(7200)])

    expect(reservation?.time_limit_s).toBe(900)
    expect(reservation?.record.expires_at).toBe(new Date(NOW.getTime() + (900 + RESERVATION_GRACE_S) * 1000).toISOString())
  })

  it('clamps the time limit to the time left on the grant, and reserves for the clamped limit', async () => {
    const reservation = await reserve(grant(300))

    expect(reservation?.time_limit_s).toBe(300)
    expect(reservation?.record.expires_at).toBe(new Date(NOW.getTime() + (300 + RESERVATION_GRACE_S) * 1000).toISOString())
  })

  it('clamps the time limit to the time left in the workflow when that ends first', async () => {
    const reservation = await reserve(grant(7200), [workflow(120)])

    expect(reservation?.time_limit_s).toBe(120)
  })

  it('rounds the time left down to whole seconds', async () => {
    expect((await reserve(grant(100.9)))?.time_limit_s).toBe(100)
  })

  it('still runs with exactly 60 seconds left', async () => {
    expect((await reserve(grant(60)))?.time_limit_s).toBe(60)
  })

  it('refuses a run launched with under 60 seconds left on the grant, naming the time left', async () => {
    await expect(reserve(grant(59))).rejects.toThrow(expect.objectContaining({ code: 'grant-expiring' }) as unknown)
    await expect(reserve(grant(59))).rejects.toThrow(
      'the grant covering workflow w has 59 seconds left: wait for it to lapse, then ask the owner for a new grant, or start a new workflow',
    )
  })

  it('refuses a run launched with under 60 seconds left in the workflow, naming the time left', async () => {
    await expect(reserve(grant(7200), [workflow(30)])).rejects.toThrow(expect.objectContaining({ code: 'grant-expiring' }) as unknown)
    await expect(reserve(grant(7200), [workflow(30)])).rejects.toThrow(/30 seconds/)
  })

  it('refuses it without holding a reservation', async () => {
    const ledger = new FakeLedger()
    const deps = { files: new FakeFiles(), ledger, workflows: new FakeWorkflows() } as unknown as ModelGatewayDependencies
    await new CapGuard(deps)
      .reserve({
        cap: { grant: grant(10), kind: 'grant' },
        config: CONFIG,
        costClass: 'metered-employer',
        estimate: { output_tokens: 1, prompt_tokens: 1, usd: 0.1 },
        input: { client: 'codex', model: 'gpt-5.5', prompt: 'hi', time_limit_s: 900, workflow: 'w' } as unknown as RunInput,
        now: NOW,
        runDir: '/run',
        runId: 'run-1',
      })
      .catch(() => undefined)

    expect(ledger.records).toEqual([])
  })
})
