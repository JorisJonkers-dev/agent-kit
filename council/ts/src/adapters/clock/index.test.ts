import { describe, expect, it } from 'vitest'

import { ProcessEnvAdapter, SystemClockAdapter } from './index.js'

describe('SystemClockAdapter', () => {
  it('returns wall-clock dates, monotonic readings, and resolves sleeps', async () => {
    const clock = new SystemClockAdapter()

    expect(clock.now()).toBeInstanceOf(Date)
    const before = clock.monotonicMs()
    await clock.sleep(0)
    const after = clock.monotonicMs()

    expect(after).toBeGreaterThanOrEqual(before)
  })
})

describe('ProcessEnvAdapter', () => {
  it('reads, requires, and snapshots environment values', () => {
    const adapter = new ProcessEnvAdapter({
      EMPTY: undefined,
      REQUIRED: 'present',
    })

    expect(adapter.get('REQUIRED')).toBe('present')
    expect(adapter.get('MISSING')).toBeUndefined()
    expect(adapter.require('REQUIRED')).toBe('present')
    expect(adapter.all()).toEqual({ REQUIRED: 'present' })
    expect(() => adapter.require('MISSING')).toThrow(
      'required environment variable is missing: MISSING',
    )
    expect(new ProcessEnvAdapter().all()).toEqual(expect.any(Object))
  })
})
