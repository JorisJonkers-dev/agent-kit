import { describe, expect, it } from 'vitest'

import { ModelGatewayError } from '../../../model-gateway/index.js'
import { FakeEngineGuard } from '../../../../../test/council/fake-engine-guard.js'
import { EngineGate, engineLeaseFileText, parseEngineLeaseFile } from './engine-gate.js'

/** A clock the test moves: every reading is 2.5 seconds after the one before. */
class StepClock {
  constructor(private ms = Date.parse('2026-11-04T10:00:00.000Z')) {}

  now(): Date {
    const at = new Date(this.ms)
    this.ms += 2_500
    return at
  }
}

describe('EngineGate', () => {
  it('opens no lease for an engine the gateway does not guard, and asks nothing', async () => {
    const guard = new FakeEngineGuard()

    const lease = await new EngineGate(guard, new StepClock()).open({
      engine: 'generic',
      model: 'm',
      promptChars: 10,
      workflow: 'run-1',
    })

    expect(lease).toBeUndefined()
    expect(guard.authorized).toEqual([])
  })

  it('authorises a guarded engine under the run’s safe workflow name and records how long the run took', async () => {
    const guard = new FakeEngineGuard()
    const gate = new EngineGate(guard, new StepClock())

    const lease = await gate.open({ engine: 'codex', model: 'gpt-5.5', promptChars: 90, repo: '/repo', workflow: 'run 1' })
    await lease?.end({ costUsd: 0.25, outcome: 'ok', tokens: { cached: 0, input: 5, output: 7 } })

    expect(guard.authorized).toEqual([
      { carries_history: true, client: 'codex', model: 'gpt-5.5', prompt_chars: 90, repo: '/repo', workflow: 'run-1' },
    ])
    expect(lease?.runId).toBe('fake-run-1')
    expect(guard.recorded).toEqual([
      {
        cost_usd: 0.25,
        duration_s: 2.5,
        outcome: 'ok',
        reservation: expect.objectContaining({ run_id: 'fake-run-1' }) as unknown,
        run_id: 'fake-run-1',
        tokens: { cached: 0, input: 5, output: 7 },
      },
    ])
  })

  it('passes a time limit and no repository through when it has them, and a failure’s reason', async () => {
    const guard = new FakeEngineGuard()

    const lease = await new EngineGate(guard, new StepClock()).open({
      engine: 'claude',
      model: 'haiku',
      promptChars: 1,
      timeLimitS: 600,
      workflow: 'run-1',
    })
    await lease?.end({ outcome: 'timeout', reason: 'stalled' })

    expect(guard.authorized[0]).toEqual({ carries_history: true, client: 'claude', model: 'haiku', prompt_chars: 1, time_limit_s: 600, workflow: 'run-1' })
    expect(guard.recorded[0]).toMatchObject({ outcome: 'timeout', reason: 'stalled' })
    expect(guard.recorded[0]).not.toHaveProperty('tokens')
    expect(guard.recorded[0]).not.toHaveProperty('cost_usd')
  })

  it('records a run once, however many times its end is reported', async () => {
    const guard = new FakeEngineGuard()
    const lease = await new EngineGate(guard, new StepClock()).open({ engine: 'codex', model: 'gpt-5.5', promptChars: 1, workflow: 'run-1' })

    await lease?.end({ outcome: 'ok' })
    await lease?.end({ outcome: 'failed' })

    expect(guard.recorded).toHaveLength(1)
  })

  it('begins a lease only for an engine the gateway guards', async () => {
    const guard = new FakeEngineGuard()
    const gate = new EngineGate(guard, new StepClock())

    await expect(gate.begin({ engine: 'generic', model: 'm', promptChars: 1, workflow: 'run-1' })).rejects.toThrow('generic is not an engine the model gateway guards')
    expect((await gate.begin({ engine: 'codex', model: 'gpt-5.5', promptChars: 1, workflow: 'run-1' })).runId).toBe('fake-run-1')
  })

  it('says a prompt carries no history only when the caller says so', async () => {
    const guard = new FakeEngineGuard()
    const gate = new EngineGate(guard, new StepClock())

    await gate.open({ carriesHistory: false, engine: 'codex', model: 'gpt-5.5', promptChars: 1, workflow: 'run-1' })

    expect(guard.authorized[0]).toMatchObject({ carries_history: false })
  })

  it('keeps what another process needs to end the lease, and ends it from that with the run’s real duration', async () => {
    const guard = new FakeEngineGuard()
    const first = await new EngineGate(guard, new StepClock()).open({ engine: 'codex', model: 'gpt-5.5', promptChars: 1, taskId: 'T1', workflow: 'run-1' })
    const persisted = JSON.parse(JSON.stringify(first?.persisted())) as NonNullable<typeof first>['persisted'] extends () => infer P ? P : never
    expect(persisted).toMatchObject({
      council_run: 'run-1',
      engine: 'codex',
      model: 'gpt-5.5',
      reservation: { run_id: 'fake-run-1' },
      started_at: '2026-11-04T10:00:00.000Z',
      task_id: 'T1',
    })

    const resumed = await new EngineGate(guard, new StepClock(Date.parse('2026-11-04T10:10:00.000Z'))).resume(persisted, { repo: '/repo' })
    await resumed.end({ outcome: 'ok' })

    expect(guard.authorized).toHaveLength(1)
    expect(guard.reauthorized).toEqual([{ carries_history: true, repo: '/repo', reservation: persisted.reservation }])
    expect(guard.recorded[0]).toMatchObject({ duration_s: 600, outcome: 'ok', run_id: 'fake-run-1' })
    expect(guard.recorded[0]).not.toHaveProperty('reattached')
  })

  it('refuses to resume a lease the gateway no longer stands behind, and ends it as failed with the gateway’s reason', async () => {
    const guard = new FakeEngineGuard()
    const first = await new EngineGate(guard, new StepClock()).open({ engine: 'codex', model: 'gpt-5.5', promptChars: 1, workflow: 'run-1' })
    guard.reauthorizeRefusal = new ModelGatewayError('no-grant', 'no live grant covers workflow run-1')

    await expect(new EngineGate(guard, new StepClock()).resume(first?.persisted() as never, { carriesHistory: false })).rejects.toThrow('no live grant')

    expect(guard.reauthorized[0]).toMatchObject({ carries_history: false })
    expect(guard.reauthorized[0]).not.toHaveProperty('repo')
    expect(guard.recorded[0]).toMatchObject({ outcome: 'failed', reason: 'no-grant', run_id: 'fake-run-1' })
  })

  it('ends it with a reason of its own when the refusal carries no code', async () => {
    const guard = new FakeEngineGuard()
    const first = await new EngineGate(guard, new StepClock()).open({ engine: 'codex', model: 'gpt-5.5', promptChars: 1, workflow: 'run-1' })
    guard.reauthorizeRefusal = new Error('ledger locked')

    await expect(new EngineGate(guard, new StepClock()).resume(first?.persisted() as never)).rejects.toThrow('ledger locked')

    expect(guard.recorded[0]).toMatchObject({ outcome: 'failed', reason: 'reauthorize-failed' })
  })

  it('marks a lease opened for a run whose supervisor died', async () => {
    const guard = new FakeEngineGuard()
    const lease = await new EngineGate(guard, new StepClock()).open({ engine: 'codex', model: 'gpt-5.5', promptChars: 1, workflow: 'run-1' }, { reattached: true })

    await lease?.end({ outcome: 'ok' })

    expect(lease?.persisted().reattached).toBe(true)
    expect(guard.recorded[0]).toMatchObject({ reattached: true })
  })

  it('lets the gateway’s refusal through untouched', async () => {
    const guard = new FakeEngineGuard()
    guard.refusal = new ModelGatewayError('no-grant', 'no live grant covers workflow run-1')

    await expect(
      new EngineGate(guard, new StepClock()).open({ engine: 'codex', model: 'gpt-5.5', promptChars: 1, workflow: 'run-1' }),
    ).rejects.toMatchObject({ code: 'no-grant', message: 'no live grant covers workflow run-1' })
  })
})

describe('the lease file', () => {
  const lease = { council_run: 'run-1', engine: 'codex', model: 'gpt-5.5', reservation: { run_id: 'r1' }, started_at: '2026-11-04T10:00:00.000Z', task_id: 'T1' }

  it('round-trips an open lease and an ended one', () => {
    const open = { lease, state: 'open' } as unknown as Parameters<typeof engineLeaseFileText>[0]
    expect(parseEngineLeaseFile(engineLeaseFileText(open))).toEqual(open)
    expect(parseEngineLeaseFile(engineLeaseFileText({ state: 'ended' }))).toEqual({ state: 'ended' })
  })

  it('refuses a file that is not JSON, or not a lease, rather than read it as no lease', () => {
    expect(() => parseEngineLeaseFile('{')).toThrow('not JSON')
    for (const text of ['null', '{}', '{"state":"open"}', '{"state":"open","lease":{"started_at":"x"}}', '{"state":"open","lease":{"reservation":{"run_id":1},"started_at":"x"}}', '{"state":"open","lease":{"reservation":{"run_id":"r"},"started_at":1}}', '{"state":"open","lease":{"reservation":{"run_id":"r"},"started_at":"x"}}', '{"state":"open","lease":null}', '{"state":"later"}']) {
      expect(() => parseEngineLeaseFile(text)).toThrow('not a lease the gateway wrote')
    }
  })
})
