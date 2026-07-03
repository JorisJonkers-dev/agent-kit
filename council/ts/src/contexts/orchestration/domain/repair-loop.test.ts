import { describe, expect, it } from 'vitest'

import type { Task } from '../../../shared-kernel/index.js'
import {
  createRepairLoopState,
  decideRepairLoop,
  type RepairLoopDecision,
} from './repair-loop.js'
import type { WorkerTraceEntry } from './worker-trace.js'

const baseTask: Task = {
  boundaries: 'Stay in bounds.',
  content_hash: 'sha256:task',
  context_refs: ['ctx'],
  depends_on: [],
  difficulty: 'moderate',
  engine: { cli: 'codex', model: 'gpt-5' },
  id: 'T1',
  model: 'sonnet',
  objective: 'Make the tests pass.',
  output_format: 'patch',
  paths: ['src/domain.ts'],
  title: 'Domain task',
  verify: 'npm test',
}

const failureTrace: readonly WorkerTraceEntry[] = [
  {
    attempt: 1,
    kind: 'attempt',
    sourceEventType: 'worker_started',
    taskId: 'T1',
    workerId: 'worker-T1',
  },
  {
    attempt: 1,
    byteCount: 26,
    kind: 'output',
    offset: 0,
    stream: 'stdout',
    tail: 'stdout prefix abcdefghij',
    taskId: 'T1',
    workerId: 'worker-T1',
  },
  {
    attempt: 1,
    byteCount: 24,
    kind: 'output',
    offset: 0,
    stream: 'stderr',
    tail: 'stderr prefix klmnopqrst',
    taskId: 'T1',
    workerId: 'worker-T1',
  },
  {
    attempt: 1,
    exitCode: 1,
    kind: 'result',
    sourceEventType: 'worker_exited',
    taskId: 'T1',
    workerId: 'worker-T1',
  },
  {
    attempt: 1,
    kind: 'result',
    sourceEventType: 'worker_finished',
    status: 'verify-failed',
    taskId: 'T1',
    workerId: 'worker-T1',
  },
]

describe('repair loop decisions', () => {
  it('emits no repair for successful worker results', () => {
    expect(
      decideRepairLoop({
        state: createRepairLoopState(),
        task: baseTask,
        trace: [
          ...failureTrace.slice(0, 1),
          {
            attempt: 1,
            kind: 'result',
            sourceEventType: 'worker_finished',
            status: 'completed',
            taskId: 'T1',
            workerId: 'worker-T1',
          },
        ],
        workerResult: {
          status: 'completed',
          verifyOutput: 'ok',
          verifyRc: 0,
        },
      }),
    ).toEqual({
      kind: 'no-repair',
      reason: 'succeeded',
      state: { repairAttemptConsumed: false },
    })
  })

  it('plans exactly one repair for an eligible verify failure and captures bounded artifacts', () => {
    const decision = decideRepairLoop({
      maxTailChars: 11,
      state: createRepairLoopState(),
      task: baseTask,
      trace: failureTrace,
      workerResult: {
        error: 'verify command failed',
        status: 'verify-failed',
        stderrTail: 'worker stderr tail',
        stdoutTail: 'worker stdout tail',
        verifyOutput: 'expected true to be false',
        verifyRc: 1,
      },
    })

    expect(decision).toEqual({
      kind: 'repair',
      plan: {
        artifacts: {
          stderrTail: 'stderr tail',
          stdoutTail: 'stdout tail',
          traceSummary: {
            attempts: [1],
            detections: [],
            latestResultStatus: 'verify-failed',
            resultStatuses: ['worker_exited:exit-1', 'worker_finished:verify-failed'],
            taskId: 'T1',
            workerIds: ['worker-T1'],
          },
          verifyOutput: 'expected true to be false',
          verifyRc: 1,
          workerError: 'verify command failed',
          workerResultStatus: 'verify-failed',
        },
        attempt: 1,
        taskId: 'T1',
        verifyCommand: 'npm test',
      },
      state: { repairAttemptConsumed: true },
    })
  })

  it('keeps one-repair-attempt invariant after a consumed repair attempt fails again', () => {
    expect(
      decideRepairLoop({
        state: { repairAttemptConsumed: true },
        task: baseTask,
        trace: failureTrace,
        workerResult: {
          status: 'verify-failed',
          verifyOutput: 'still failing',
          verifyRc: 1,
        },
      }),
    ).toEqual({
      artifacts: {
        stderrTail: 'stderr prefix klmnopqrst',
        stdoutTail: 'stdout prefix abcdefghij',
        traceSummary: {
          attempts: [1],
          detections: [],
          latestResultStatus: 'verify-failed',
          resultStatuses: ['worker_exited:exit-1', 'worker_finished:verify-failed'],
          taskId: 'T1',
          workerIds: ['worker-T1'],
        },
        verifyOutput: 'still failing',
        verifyRc: 1,
        workerResultStatus: 'verify-failed',
      },
      kind: 'terminal-failure',
      reason: 'repair-attempt-consumed',
      state: { repairAttemptConsumed: true },
    })
  })

  it('summarizes missing trace details as unknown and ignores output chunks without tails', () => {
    expect(
      decideRepairLoop({
        state: createRepairLoopState(),
        task: baseTask,
        trace: [
          {
            attempt: 2,
            byteCount: 12,
            kind: 'output',
            offset: 0,
            stream: 'stdout',
            taskId: 'T1',
            workerId: 'worker-T1b',
          },
          {
            attempt: 1,
            kind: 'detection',
            taskId: 'T1',
            workerId: 'worker-T1',
          },
          {
            attempt: 1,
            kind: 'result',
            sourceEventType: 'worker_finished',
            taskId: 'T1',
            workerId: 'worker-T1',
          },
        ],
        workerResult: {
          status: 'failed',
          verifyRc: 1,
        },
      }),
    ).toEqual({
      kind: 'repair',
      plan: {
        artifacts: {
          stderrTail: '',
          stdoutTail: '',
          traceSummary: {
            attempts: [1, 2],
            detections: ['unknown'],
            latestResultStatus: 'unknown',
            resultStatuses: ['worker_finished:unknown'],
            taskId: 'T1',
            workerIds: ['worker-T1', 'worker-T1b'],
          },
          verifyOutput: '',
          verifyRc: 1,
          workerResultStatus: 'failed',
        },
        attempt: 1,
        taskId: 'T1',
        verifyCommand: 'npm test',
      },
      state: { repairAttemptConsumed: true },
    })
  })

  it('treats out-of-bounds failures as terminal without consuming repair', () => {
    expect(terminalReasonFor({
      status: 'failed',
      outOfBounds: ['../outside.ts'],
      verifyRc: 1,
    })).toEqual({
      reason: 'out-of-bounds',
      state: { repairAttemptConsumed: false },
    })
  })

  it('treats human-review-required tasks as terminal without consuming repair', () => {
    expect(
      terminalReasonFor(
        {
          status: 'verify-failed',
          verifyRc: 1,
        },
        { ...baseTask, human_review_required: true },
      ),
    ).toEqual({
      reason: 'human-review-required',
      state: { repairAttemptConsumed: false },
    })
  })

  it('treats non-retryable resource failures as terminal without consuming repair', () => {
    expect(
      terminalReasonFor(
        {
          status: 'failed',
          verifyOutput: 'disk budget exceeded',
          verifyRc: null,
        },
        baseTask,
        [
          {
            attempt: 1,
            kind: 'detection',
            status: 'disk-cap',
            taskId: 'T1',
            workerId: 'worker-T1',
          },
        ],
      ),
    ).toEqual({
      reason: 'non-retryable-resource-failure',
      state: { repairAttemptConsumed: false },
    })
  })
})

function terminalReasonFor(
  workerResult: Parameters<typeof decideRepairLoop>[0]['workerResult'],
  task: Task = baseTask,
  trace: readonly WorkerTraceEntry[] = failureTrace,
): Pick<Extract<RepairLoopDecision, { readonly kind: 'terminal-failure' }>, 'reason' | 'state'> {
  const decision = decideRepairLoop({
    state: createRepairLoopState(),
    task,
    trace,
    workerResult,
  })
  expect(decision.kind).toBe('terminal-failure')
  const terminalDecision = decision as Extract<RepairLoopDecision, { readonly kind: 'terminal-failure' }>

  return {
    reason: terminalDecision.reason,
    state: terminalDecision.state,
  }
}
