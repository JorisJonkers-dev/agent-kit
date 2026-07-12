import { describe, expect, it } from 'vitest'

import { evaluatePredicate, isMonitorDead, parseDuration } from './index.js'
import type { MonitorState } from './index.js'

describe('parseDuration', () => {
  it('converts seconds', () => {
    expect(parseDuration('30s')).toBe(30_000)
    expect(parseDuration('1s')).toBe(1_000)
  })

  it('converts minutes', () => {
    expect(parseDuration('45m')).toBe(2_700_000)
    expect(parseDuration('1m')).toBe(60_000)
  })

  it('converts hours', () => {
    expect(parseDuration('2h')).toBe(7_200_000)
    expect(parseDuration('1h')).toBe(3_600_000)
  })

  it('throws on invalid format', () => {
    expect(() => parseDuration('100')).toThrow('invalid duration')
    expect(() => parseDuration('1d')).toThrow('invalid duration')
    expect(() => parseDuration('')).toThrow('invalid duration')
    expect(() => parseDuration('abc')).toThrow('invalid duration')
  })
})

function makeMonitorState(overrides: Partial<MonitorState> = {}): MonitorState {
  const now = new Date('2026-01-01T00:00:00.000Z')
  return {
    name: 'test-monitor',
    status: 'polling',
    startedAt: now.toISOString(),
    deadline: new Date(now.getTime() + 60_000).toISOString(),
    lastTickAt: now.toISOString(),
    lastOutput: '',
    intervalMs: 5_000,
    cmd: 'echo hello',
    until: 'hello',
    then: 'echo done',
    ...overrides,
  }
}

describe('isMonitorDead', () => {
  it('returns false when status is not polling', () => {
    const state = makeMonitorState({ status: 'passed' })
    const nowMs = new Date('2026-01-01T01:00:00.000Z').getTime()
    expect(isMonitorDead(state, nowMs)).toBe(false)
  })

  it('returns false when lastTickAt is recent enough', () => {
    const lastTickAt = new Date('2026-01-01T00:00:00.000Z').toISOString()
    const state = makeMonitorState({ lastTickAt, intervalMs: 5_000 })
    // 2.5 * 5000 = 12500ms threshold; 10000ms elapsed — not dead
    const nowMs = new Date('2026-01-01T00:00:10.000Z').getTime()
    expect(isMonitorDead(state, nowMs)).toBe(false)
  })

  it('returns true when lastTickAt is older than 2.5x interval and status is polling', () => {
    const lastTickAt = new Date('2026-01-01T00:00:00.000Z').toISOString()
    const state = makeMonitorState({ lastTickAt, intervalMs: 5_000 })
    // 2.5 * 5000 = 12500ms threshold; 15000ms elapsed — dead
    const nowMs = new Date('2026-01-01T00:00:15.000Z').getTime()
    expect(isMonitorDead(state, nowMs)).toBe(true)
  })

  it('returns false when at exactly the threshold', () => {
    const lastTickAt = new Date('2026-01-01T00:00:00.000Z').toISOString()
    const state = makeMonitorState({ lastTickAt, intervalMs: 5_000 })
    // 2.5 * 5000 = 12500ms; exactly 12500ms elapsed — not dead (> not >=)
    const nowMs = new Date('2026-01-01T00:00:12.500Z').getTime()
    expect(isMonitorDead(state, nowMs)).toBe(false)
  })

  it('returns false when status is timed-out', () => {
    const state = makeMonitorState({ status: 'timed-out' })
    const nowMs = new Date('2026-01-01T01:00:00.000Z').getTime()
    expect(isMonitorDead(state, nowMs)).toBe(false)
  })
})

describe('evaluatePredicate', () => {
  it('does substring match when predicate does not start with dot', () => {
    expect(evaluatePredicate('hello', 'hello world')).toBe(true)
    expect(evaluatePredicate('missing', 'hello world')).toBe(false)
  })

  it('handles exact substring match', () => {
    expect(evaluatePredicate('success', 'status: success')).toBe(true)
    expect(evaluatePredicate('success', 'status: failed')).toBe(false)
  })

  it('evaluates jq-style .field path against JSON output', () => {
    const output = JSON.stringify({ status: 'completed' })
    expect(evaluatePredicate('.status', output)).toBe(true)
  })

  it('evaluates jq-style .field == "value" equality expression', () => {
    const output = JSON.stringify({ status: 'completed' })
    expect(evaluatePredicate('.status == "completed"', output)).toBe(true)
    expect(evaluatePredicate('.status == "failed"', output)).toBe(false)
  })

  it('evaluates nested .a.b path', () => {
    const output = JSON.stringify({ run: { conclusion: 'success' } })
    expect(evaluatePredicate('.run.conclusion == "success"', output)).toBe(true)
    expect(evaluatePredicate('.run.conclusion == "failure"', output)).toBe(false)
  })

  it('returns false for jq path when output is not valid JSON', () => {
    expect(evaluatePredicate('.status', 'not json')).toBe(false)
  })

  it('returns false for jq path when field is missing', () => {
    const output = JSON.stringify({ other: 'value' })
    expect(evaluatePredicate('.status == "completed"', output)).toBe(false)
  })

  it('returns false for jq path when field is null or false', () => {
    expect(evaluatePredicate('.ok', JSON.stringify({ ok: null }))).toBe(false)
    expect(evaluatePredicate('.ok', JSON.stringify({ ok: false }))).toBe(false)
    expect(evaluatePredicate('.ok', JSON.stringify({ ok: '' }))).toBe(false)
  })

  it('returns true for jq path when field is a truthy value', () => {
    expect(evaluatePredicate('.ok', JSON.stringify({ ok: true }))).toBe(true)
    expect(evaluatePredicate('.ok', JSON.stringify({ ok: 'yes' }))).toBe(true)
  })
})

describe('evaluatePredicate - edge cases for resolveJsonPath', () => {
  it('returns false when JSON value at path is an array (not object)', () => {
    const output = JSON.stringify({ items: ['a', 'b'] })
    // Trying to traverse into an array as if it's an object
    expect(evaluatePredicate('.items.foo', output)).toBe(false)
  })

  it('returns false when intermediate path segment is null', () => {
    const output = JSON.stringify({ nested: null })
    expect(evaluatePredicate('.nested.field', output)).toBe(false)
  })
})
