import { describe, expect, it } from 'vitest'

import {
  advanceEscalation,
  appendLoopLine,
  createEscalationState,
  createLoopDetectorState,
  createStallDetectorState,
  evaluateDiskUsageCap,
  evaluateStall,
  extractActionLines,
} from './index.js'

describe('stall detector', () => {
  it('tracks log growth and reports a stall after the configured idle period', () => {
    const initial = createStallDetectorState(1_000)

    const beforeLimit = evaluateStall(initial, {
      logBytes: 0,
      nowMs: 2_999,
      stallAfterS: 2,
    })

    expect(beforeLimit).toEqual({ state: initial, detection: null })

    const afterGrowth = evaluateStall(beforeLimit.state, {
      logBytes: 12,
      nowMs: 3_000,
      stallAfterS: 2,
    })

    expect(afterGrowth).toEqual({
      state: { logBytes: 12, lastGrowthAtMs: 3_000 },
      detection: null,
    })

    expect(
      evaluateStall(afterGrowth.state, {
        logBytes: 12,
        nowMs: 5_000,
        stallAfterS: 2,
      }),
    ).toEqual({
      state: afterGrowth.state,
      detection: { kind: 'stall', idleMs: 2_000, logBytes: 12 },
    })
  })
})

describe('loop detector', () => {
  it('ignores blank lines and non-action log lines', () => {
    const state = createLoopDetectorState()
    const config = { windowSize: 3, repeatLimit: 3 }

    expect(appendLoopLine(state, '   ', config)).toEqual({ state, detection: null })
    expect(appendLoopLine(state, 'ordinary progress output', config)).toEqual({
      state,
      detection: null,
    })
  })

  it('extracts Claude stream-json tool_use records with stable normalized input', () => {
    const line = JSON.stringify({
      type: 'assistant',
      message: {
        content: [
          {
            type: 'tool_use',
            name: 'Bash',
            input: {
              z: ['npm', 'test'],
              a: 42,
              empty: null,
            },
          },
        ],
      },
    })

    expect(extractActionLines(line)).toEqual([
      {
        verbatim: 'tool:Bash:{"a":42,"empty":null,"z":["npm","test"]}',
        normalized: 'tool:bash:{"a":<n>,"empty":null,"z":["npm","test"]}',
      },
    ])
  })

  it('extracts array-form and sparse Claude tool_use records', () => {
    expect(
      extractActionLines(
        JSON.stringify([
          { type: 'tool_use', input: 'not-an-object' },
          { type: 'text', text: 'ignored' },
        ]),
      ),
    ).toEqual([
      {
        verbatim: 'tool:unknown:{}',
        normalized: 'tool:unknown:{}',
      },
    ])
  })

  it('extracts and normalizes Codex command lines', () => {
    expect(extractActionLines('$ npm run build -- --attempt 123 abcdef123456')).toEqual([
      {
        verbatim: 'npm run build -- --attempt 123 abcdef123456',
        normalized: 'npm run build -- --attempt <n> <hex>',
      },
    ])

    expect(extractActionLines('exec_command: npm test')).toEqual([
      { verbatim: 'npm test', normalized: 'npm test' },
    ])
    expect(extractActionLines('command node ./script.mjs')).toEqual([
      { verbatim: 'node ./script.mjs', normalized: 'node ./script.mjs' },
    ])
  })

  it('detects repeated normalized actions inside the sliding window', () => {
    const config = { windowSize: 4, repeatLimit: 3 }
    const first = appendLoopLine(createLoopDetectorState(), '$ npm test --attempt 1', config)
    const second = appendLoopLine(first.state, '$ npm test --attempt 2', config)
    const third = appendLoopLine(second.state, '$ npm test --attempt 3', config)

    expect(third.detection).toEqual({
      kind: 'loop-repeat',
      normalized: 'npm test --attempt <n>',
      count: 3,
    })
  })

  it('drops old actions outside the sliding window', () => {
    const config = { windowSize: 2, repeatLimit: 3 }
    const first = appendLoopLine(createLoopDetectorState(), '$ npm test', config)
    const second = appendLoopLine(first.state, '$ npm test', config)
    const third = appendLoopLine(second.state, '$ npm test', config)

    expect(third).toEqual({
      state: {
        actions: [
          { verbatim: 'npm test', normalized: 'npm test' },
          { verbatim: 'npm test', normalized: 'npm test' },
        ],
      },
      detection: null,
    })
  })

  it('detects verbatim k-gram cycles up to the configured gram size', () => {
    const config = { windowSize: 12, repeatLimit: 99 }
    const lines = ['$ make a', '$ make b', '$ make a', '$ make b', '$ make a', '$ make b']
    const result = lines.reduce(
      (current, line) => appendLoopLine(current.state, line, config),
      appendLoopLine(createLoopDetectorState(), 'not an action', config),
    )

    expect(result.detection).toEqual({
      kind: 'loop-cycle',
      gramSize: 2,
      sequence: ['make a', 'make b'],
    })
  })

  it('honors maxCycleGram when detecting cycles', () => {
    const config = { windowSize: 12, repeatLimit: 99, maxCycleGram: 1 }
    const lines = ['$ make a', '$ make b', '$ make a', '$ make b', '$ make a', '$ make b']
    const result = lines.reduce(
      (current, line) => appendLoopLine(current.state, line, config),
      appendLoopLine(createLoopDetectorState(), 'not an action', config),
    )

    expect(result.detection).toBeNull()
  })

  it('caps verbatim cycle detection at five-grams', () => {
    const config = { windowSize: 18, repeatLimit: 99, maxCycleGram: 6 }
    const lines = [
      '$ step a',
      '$ step b',
      '$ step c',
      '$ step d',
      '$ step e',
      '$ step f',
      '$ step a',
      '$ step b',
      '$ step c',
      '$ step d',
      '$ step e',
      '$ step f',
      '$ step a',
      '$ step b',
      '$ step c',
      '$ step d',
      '$ step e',
      '$ step f',
    ]
    const result = lines.reduce(
      (current, line) => appendLoopLine(current.state, line, config),
      appendLoopLine(createLoopDetectorState(), 'not an action', config),
    )

    expect(result.detection).toBeNull()
  })
})

describe('disk usage cap detector', () => {
  it('reports only values above the cap', () => {
    expect(evaluateDiskUsageCap({ duBytes: 10, capBytes: 10 })).toBeNull()
    expect(evaluateDiskUsageCap({ duBytes: 11, capBytes: 10 })).toEqual({
      kind: 'disk-cap',
      duBytes: 11,
      capBytes: 10,
    })
  })
})

describe('escalation state machine', () => {
  it('terminates, retries with preamble, escalates tier, then marks stalled', () => {
    const config = { enableTierEscalation: true }
    const terminate = advanceEscalation(createEscalationState(), config)
    const retry = advanceEscalation(terminate.state, config)
    const escalate = advanceEscalation(retry.state, config)
    const stalled = advanceEscalation(escalate.state, config)
    const terminal = advanceEscalation(stalled.state, config)

    expect([terminate.action, retry.action, escalate.action, stalled.action, terminal.action]).toEqual([
      'terminate',
      'retry-with-preamble',
      'escalate-tier',
      'stalled',
      'stalled',
    ])
    expect(terminal.state).toEqual({ phase: 'stalled' })
  })

  it('skips tier escalation when disabled', () => {
    const config = { enableTierEscalation: false }
    const terminate = advanceEscalation(createEscalationState(), config)
    const retry = advanceEscalation(terminate.state, config)

    expect(advanceEscalation(retry.state, config)).toEqual({
      state: { phase: 'stalled' },
      action: 'stalled',
    })
  })
})
