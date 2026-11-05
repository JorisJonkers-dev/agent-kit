import { describe, expect, it } from 'vitest'

import { liveReservations, meteredCharges, promptTokens, reservationRecord, settledRecord } from './caps.js'
import type { LedgerEntry } from './ledger.js'

describe('promptTokens', () => {
  it('counts ASCII text at 3.5 characters a token, rounded up', () => {
    expect(promptTokens('x'.repeat(3500))).toBe(1000)
    expect(promptTokens('x'.repeat(3501))).toBe(1001)
    expect(promptTokens('')).toBe(0)
  })

  it('counts every non-ASCII character as a token of its own, so CJK text is not undercounted', () => {
    expect(promptTokens('漢'.repeat(1000))).toBeGreaterThanOrEqual(1000)
    expect(promptTokens('x'.repeat(35) + '漢'.repeat(10))).toBe(20)
    expect(promptTokens('😀')).toBe(1)
  })
})

describe('what a settled reservation counts for', () => {
  const at = new Date('2026-11-04T10:00:00.000Z')
  const reserved = reservationRecord({
    client: 'codex',
    cost_class: 'metered-employer',
    estimate_usd: 1,
    model: 'gpt-5.5',
    reserved_at: at,
    run_dir: '/nowhere',
    run_id: 'r1',
    time_limit_s: 900,
    workflow: 'w',
  })

  it('counts the cost of a run that left a result but no line of its own, and nothing for one that never started or ran', () => {
    const ran: LedgerEntry[] = [reserved, { ...settledRecord(reserved, 7, at), reason: 'expired' as const }]
    const notRun: LedgerEntry[] = [reserved, { ...settledRecord(reserved, 0, at), reason: 'not-run' as const }]
    const notStarted: LedgerEntry[] = [reserved, { ...settledRecord(reserved, 0, at), reason: 'not-started' as const }]

    expect(meteredCharges(ran, 'metered-employer').map((charge) => charge.usd)).toEqual([7])
    expect(meteredCharges(notRun, 'metered-employer')).toEqual([])
    expect(meteredCharges(notStarted, 'metered-employer')).toEqual([])
    expect(liveReservations(ran)).toEqual([])
  })
})
