import { describe, expect, it } from 'vitest'

import { ledgerLine, type LedgerRecord } from './ledger.js'

const RECORD: LedgerRecord = {
  at: '2026-11-04T10:52:15.123Z',
  client: 'vibe',
  cost_usd: 0.0123,
  duration_s: 12.5,
  family: 'zhipu',
  mode: 'read-only',
  model: 'zai-glm-5-3',
  outcome: 'ok',
  repository: 'github.com/jorisjonkers-dev/agent-kit',
  run_id: '20261104T105215Z-a1b2c3',
  tokens: { cached: 10, input: 100, output: 20 },
  workflow: 'spec-293',
}

describe('ledgerLine', () => {
  it('is one JSON object on one line', () => {
    const line = ledgerLine(RECORD)

    expect(line.endsWith('\n')).toBe(true)
    expect(line.split('\n')).toHaveLength(2)
    expect(JSON.parse(line)).toEqual(RECORD)
  })

  it('carries the reason of a failed run, and no repository for an inline one', () => {
    const line = ledgerLine({ ...RECORD, outcome: 'failed', reason: 'timeout', repository: null })

    expect(JSON.parse(line)).toMatchObject({ outcome: 'failed', reason: 'timeout', repository: null })
  })
})
