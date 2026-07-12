import { describe, expect, it } from 'vitest'

import {
  buildPollUntilGreenMonitorArgs,
  evaluateWatchdog,
  parseWorkerLiveness,
  shouldEscalate,
} from './supervise.js'
import type {
  AutoNudgeConfig,
  NudgeRecord,
  PollUntilGreenInput,
  WatchdogWorkerEntry,
} from './supervise.js'

describe('parseWorkerLiveness', () => {
  it('returns done when output contains STATUS: DONE', () => {
    const result = parseWorkerLiveness('Some summary text.\nSTATUS: DONE')
    expect(result.kind).toBe('done')
  })

  it('returns done when STATUS: DONE appears inline', () => {
    const result = parseWorkerLiveness('STATUS: DONE')
    expect(result.kind).toBe('done')
  })

  it('returns waiting with parsed fields for WAITING(...)', () => {
    const output =
      'STATUS: WAITING(reason=CI not green, resume-condition=check actions runs, deadline=2026-07-12T10:00:00.000Z)'
    const result = parseWorkerLiveness(output)
    expect(result.kind).toBe('waiting')
    expect(result.reason).toBe('CI not green')
    expect(result.resumeCondition).toBe('check actions runs')
    expect(result.deadline).toBe('2026-07-12T10:00:00.000Z')
  })

  it('returns waiting with monitor field when monitor= is present', () => {
    const output =
      'STATUS: WAITING(reason=CI running, resume-condition=wait for green, deadline=2026-07-12T10:00:00.000Z, monitor=ci-sha-monitor)'
    const result = parseWorkerLiveness(output)
    expect(result.kind).toBe('waiting')
    expect(result.monitor).toBe('ci-sha-monitor')
  })

  it('returns stalled for bare waiting text without STATUS line', () => {
    const result = parseWorkerLiveness('Still waiting for CI to finish...')
    expect(result.kind).toBe('stalled')
  })

  it('returns stalled for empty output', () => {
    const result = parseWorkerLiveness('')
    expect(result.kind).toBe('stalled')
  })

  it('returns stalled for generic summary text', () => {
    const result = parseWorkerLiveness('I updated the file and now waiting for CI.')
    expect(result.kind).toBe('stalled')
  })
})

describe('evaluateWatchdog', () => {
  const baseTime = new Date('2026-07-12T10:00:00.000Z').getTime()
  const stallWindowMs = 5 * 60 * 1_000 // 5 minutes

  it('returns fresh when last activity is within stall window', () => {
    const entry: WatchdogWorkerEntry = {
      taskId: 'task-1',
      lastActivityAt: new Date(baseTime - 60_000).toISOString(), // 1 minute ago
    }
    const result = evaluateWatchdog(entry, baseTime, stallWindowMs, [])
    expect(result.kind).toBe('fresh')
    expect(result.taskId).toBe('task-1')
  })

  it('returns stalled when activity too old and no liveness report', () => {
    const entry: WatchdogWorkerEntry = {
      taskId: 'task-2',
      lastActivityAt: new Date(baseTime - 10 * 60_000).toISOString(), // 10 minutes ago
    }
    const result = evaluateWatchdog(entry, baseTime, stallWindowMs, [])
    expect(result.kind).toBe('stalled')
    expect(result.taskId).toBe('task-2')
  })

  it('returns stalled when stale with no WAITING liveness', () => {
    const entry: WatchdogWorkerEntry = {
      taskId: 'task-3',
      lastActivityAt: new Date(baseTime - 10 * 60_000).toISOString(),
      livenessReport: { kind: 'stalled' },
    }
    const result = evaluateWatchdog(entry, baseTime, stallWindowMs, [])
    expect(result.kind).toBe('stalled')
  })

  it('returns waiting-with-live-monitor when WAITING and live monitor exists', () => {
    const entry: WatchdogWorkerEntry = {
      taskId: 'task-4',
      lastActivityAt: new Date(baseTime - 10 * 60_000).toISOString(),
      livenessReport: {
        kind: 'waiting',
        reason: 'CI running',
        resumeCondition: 'check green',
        deadline: '2026-07-12T12:00:00.000Z',
        monitor: 'ci-monitor',
      },
      monitorName: 'ci-monitor',
    }
    const monitors = [{ name: 'ci-monitor', status: 'polling', dead: false }]
    const result = evaluateWatchdog(entry, baseTime, stallWindowMs, monitors)
    expect(result.kind).toBe('waiting-with-live-monitor')
    expect(result.taskId).toBe('task-4')
    expect(result.reason).toBe('ci-monitor')
  })

  it('returns stale when WAITING but monitor is dead', () => {
    const entry: WatchdogWorkerEntry = {
      taskId: 'task-5',
      lastActivityAt: new Date(baseTime - 10 * 60_000).toISOString(),
      livenessReport: {
        kind: 'waiting',
        reason: 'CI running',
        resumeCondition: 'check green',
        deadline: '2026-07-12T12:00:00.000Z',
        monitor: 'ci-monitor',
      },
      monitorName: 'ci-monitor',
    }
    const monitors = [{ name: 'ci-monitor', status: 'polling', dead: true }]
    const result = evaluateWatchdog(entry, baseTime, stallWindowMs, monitors)
    expect(result.kind).toBe('stale')
    expect(result.taskId).toBe('task-5')
  })

  it('returns stale when WAITING but monitor is not found', () => {
    const entry: WatchdogWorkerEntry = {
      taskId: 'task-6',
      lastActivityAt: new Date(baseTime - 10 * 60_000).toISOString(),
      livenessReport: {
        kind: 'waiting',
        reason: 'CI running',
        resumeCondition: 'check green',
        deadline: '2026-07-12T12:00:00.000Z',
        monitor: 'ci-monitor',
      },
      monitorName: 'ci-monitor',
    }
    const result = evaluateWatchdog(entry, baseTime, stallWindowMs, [])
    expect(result.kind).toBe('stale')
    expect(result.reason).toContain('not found')
  })

  it('returns stale when WAITING but monitor has timed-out status', () => {
    const entry: WatchdogWorkerEntry = {
      taskId: 'task-7',
      lastActivityAt: new Date(baseTime - 10 * 60_000).toISOString(),
      livenessReport: {
        kind: 'waiting',
        reason: 'CI running',
        resumeCondition: 'check green',
        deadline: '2026-07-12T12:00:00.000Z',
        monitor: 'ci-monitor',
      },
      monitorName: 'ci-monitor',
    }
    const monitors = [{ name: 'ci-monitor', status: 'timed-out', dead: false }]
    const result = evaluateWatchdog(entry, baseTime, stallWindowMs, monitors)
    expect(result.kind).toBe('stale')
  })

  it('uses monitor from livenessReport when entry.monitorName is absent', () => {
    const entry: WatchdogWorkerEntry = {
      taskId: 'task-8',
      lastActivityAt: new Date(baseTime - 10 * 60_000).toISOString(),
      livenessReport: {
        kind: 'waiting',
        reason: 'CI running',
        resumeCondition: 'check green',
        deadline: '2026-07-12T12:00:00.000Z',
        monitor: 'inferred-monitor',
      },
    }
    const monitors = [{ name: 'inferred-monitor', status: 'polling', dead: false }]
    const result = evaluateWatchdog(entry, baseTime, stallWindowMs, monitors)
    expect(result.kind).toBe('waiting-with-live-monitor')
    expect(result.reason).toBe('inferred-monitor')
  })
})

describe('shouldEscalate', () => {
  const config: AutoNudgeConfig = { stallWindowMs: 5 * 60_000, maxNudges: 3 }

  it('returns false when nudgeCount is below maxNudges', () => {
    const record: NudgeRecord = {
      taskId: 'task-1',
      nudgeCount: 2,
      lastNudgeAt: '2026-07-12T10:00:00.000Z',
    }
    expect(shouldEscalate(record, config)).toBe(false)
  })

  it('returns true when nudgeCount equals maxNudges', () => {
    const record: NudgeRecord = {
      taskId: 'task-2',
      nudgeCount: 3,
      lastNudgeAt: '2026-07-12T10:00:00.000Z',
    }
    expect(shouldEscalate(record, config)).toBe(true)
  })

  it('returns true when nudgeCount exceeds maxNudges', () => {
    const record: NudgeRecord = {
      taskId: 'task-3',
      nudgeCount: 5,
      lastNudgeAt: '2026-07-12T10:00:00.000Z',
    }
    expect(shouldEscalate(record, config)).toBe(true)
  })

  it('returns false when nudgeCount is zero', () => {
    const record: NudgeRecord = {
      taskId: 'task-4',
      nudgeCount: 0,
      lastNudgeAt: '2026-07-12T10:00:00.000Z',
    }
    expect(shouldEscalate(record, config)).toBe(false)
  })
})

describe('buildPollUntilGreenMonitorArgs', () => {
  it('produces correct argv with all required flags using defaults', () => {
    const input: PollUntilGreenInput = {
      sha: 'abc123',
      repo: 'owner/repo',
      monitorName: 'ci-green',
      execDir: '/tmp/exec',
    }
    const args = buildPollUntilGreenMonitorArgs(input)

    expect(args[0]).toBe('start')
    expect(args).toContain('--name')
    expect(args[args.indexOf('--name') + 1]).toBe('ci-green')
    expect(args).toContain('--interval')
    expect(args[args.indexOf('--interval') + 1]).toBe('30s')
    expect(args).toContain('--deadline')
    expect(args[args.indexOf('--deadline') + 1]).toBe('45m')
    expect(args).toContain('--cmd')
    expect(args[args.indexOf('--cmd') + 1]).toContain('abc123')
    expect(args[args.indexOf('--cmd') + 1]).toContain('owner/repo')
    expect(args).toContain('--until')
    expect(args[args.indexOf('--until') + 1]).toBe('.workflow_runs[0].conclusion == "success"')
    expect(args).toContain('--then')
    expect(args).toContain('--exec-dir')
    expect(args[args.indexOf('--exec-dir') + 1]).toBe('/tmp/exec')
  })

  it('uses provided intervalMs and deadlineMs when set', () => {
    const input: PollUntilGreenInput = {
      sha: 'def456',
      repo: 'org/proj',
      monitorName: 'build-check',
      execDir: '/tmp/exec',
      intervalMs: 60_000,    // 1m
      deadlineMs: 7_200_000, // 2h
    }
    const args = buildPollUntilGreenMonitorArgs(input)

    expect(args[args.indexOf('--interval') + 1]).toBe('1m')
    expect(args[args.indexOf('--deadline') + 1]).toBe('2h')
  })

  it('uses provided finalizer when set', () => {
    const input: PollUntilGreenInput = {
      sha: 'abc',
      repo: 'owner/repo',
      monitorName: 'mon',
      execDir: '/tmp/exec',
      finalizer: 'echo done',
    }
    const args = buildPollUntilGreenMonitorArgs(input)

    expect(args[args.indexOf('--then') + 1]).toBe('echo done')
  })

  it('uses empty string for then when finalizer is absent', () => {
    const input: PollUntilGreenInput = {
      sha: 'abc',
      repo: 'owner/repo',
      monitorName: 'mon',
      execDir: '/tmp/exec',
    }
    const args = buildPollUntilGreenMonitorArgs(input)

    expect(args[args.indexOf('--then') + 1]).toBe('')
  })

  it('converts sub-minute intervalMs to seconds string', () => {
    const input: PollUntilGreenInput = {
      sha: 'abc',
      repo: 'owner/repo',
      monitorName: 'mon',
      execDir: '/tmp/exec',
      intervalMs: 30_000, // 30s
    }
    const args = buildPollUntilGreenMonitorArgs(input)

    expect(args[args.indexOf('--interval') + 1]).toBe('30s')
  })
})
