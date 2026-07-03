import { describe, expect, it } from 'vitest'

import {
  advanceEscalation,
  appendLoopLine,
  createWatchdogProgressState,
  createEscalationState,
  createLoopDetectorState,
  createRetryPolicyState,
  evaluateDiskUsageCap,
  evaluateWatchdogProgress,
  extractActionLines,
} from './index.js'

describe('progress watchdog', () => {
  it('tracks output heartbeats without treating byte count alone as progress', () => {
    const initial = createWatchdogProgressState(1_000)

    const afterOutput = evaluateWatchdogProgress(initial, {
      nowMs: 1_500,
      outputBytes: 12,
      outputHeartbeatAfterMs: 1_000,
      progressAfterMs: 1_000,
    })

    expect(afterOutput).toEqual({
      state: {
        attemptStartedAtMs: 1_000,
        lastActionAtMs: 1_000,
        lastOutputAtMs: 1_500,
        lastProgressAtMs: 1_000,
        outputBytes: 12,
        startedAtMs: 1_000,
      },
      detection: null,
    })

    expect(
      evaluateWatchdogProgress(afterOutput.state, {
        nowMs: 2_500,
        outputBytes: 12,
        outputHeartbeatAfterMs: 1_000,
      }),
    ).toEqual({
      state: afterOutput.state,
      detection: {
        idleMs: 1_000,
        kind: 'output-heartbeat-stall',
        lastOutputAtMs: 1_500,
      },
    })
  })

  it('uses optional heartbeat snapshots for action and progress clocks', () => {
    const initial = createWatchdogProgressState(1_000)
    const snapshotted = evaluateWatchdogProgress(initial, {
      heartbeat: {
        lastActionAtMs: 4_900,
        lastOutputAtMs: 4_700,
        lastProgressAtMs: 4_800,
        outputBytes: 33,
      },
      nowMs: 5_000,
      actionHeartbeatAfterMs: 1_000,
      outputHeartbeatAfterMs: 1_000,
      progressAfterMs: 1_000,
    })

    expect(snapshotted).toEqual({
      state: {
        attemptStartedAtMs: 1_000,
        lastActionAtMs: 4_900,
        lastOutputAtMs: 4_700,
        lastProgressAtMs: 4_800,
        outputBytes: 33,
        startedAtMs: 1_000,
      },
      detection: null,
    })

    expect(
      evaluateWatchdogProgress(snapshotted.state, {
        nowMs: 5_900,
        actionHeartbeatAfterMs: 1_000,
      }),
    ).toEqual({
      state: snapshotted.state,
      detection: {
        idleMs: 1_000,
        kind: 'action-heartbeat-stall',
        lastActionAtMs: 4_900,
      },
    })

    expect(
      evaluateWatchdogProgress(snapshotted.state, {
        nowMs: 5_800,
        progressAfterMs: 1_000,
      }),
    ).toEqual({
      state: snapshotted.state,
      detection: {
        idleMs: 1_000,
        kind: 'progress-stall',
        lastProgressAtMs: 4_800,
      },
    })
  })

  it('reports wall-clock, output, and attempt budget detections deterministically', () => {
    const initial = createWatchdogProgressState(1_000, 10)
    const secondAttempt = {
      ...initial,
      attemptStartedAtMs: 2_000,
      lastActionAtMs: 2_000,
      lastOutputAtMs: 2_000,
      lastProgressAtMs: 2_000,
    }

    expect(
      evaluateWatchdogProgress(initial, {
        nowMs: 3_000,
        wallClockCapMs: 2_000,
      }),
    ).toEqual({
      state: initial,
      detection: { capMs: 2_000, elapsedMs: 2_000, kind: 'wall-clock-cap' },
    })

    expect(
      evaluateWatchdogProgress(initial, {
        nowMs: 1_100,
        outputBytes: 20,
        outputCapBytes: 20,
      }),
    ).toEqual({
      state: {
        ...initial,
        lastOutputAtMs: 1_100,
        outputBytes: 20,
      },
      detection: null,
    })

    expect(
      evaluateWatchdogProgress(initial, {
        nowMs: 1_100,
        outputBytes: 21,
        outputCapBytes: 20,
      }),
    ).toEqual({
      state: {
        ...initial,
        lastOutputAtMs: 1_100,
        outputBytes: 21,
      },
      detection: { capBytes: 20, kind: 'output-cap', outputBytes: 21 },
    })

    expect(
      evaluateWatchdogProgress(secondAttempt, {
        nowMs: 3_000,
        attemptTimeoutMs: 1_000,
      }),
    ).toEqual({
      state: secondAttempt,
      detection: { elapsedMs: 1_000, kind: 'attempt-timeout', timeoutMs: 1_000 },
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

describe('watchdog domain barrel', () => {
  it('exports retry policy surface', () => {
    expect(createRetryPolicyState()).toEqual({ attempts: 0, failureFingerprints: [] })
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
