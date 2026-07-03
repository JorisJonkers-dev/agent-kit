import { describe, expect, it } from 'vitest'

import {
  workerDetectedEvent,
  workerExitedEvent,
  workerFinishedEvent,
  workerOutputEvent,
  workerRestartedEvent,
  workerStartedEvent,
} from '../../runstore/index.js'
import {
  appendWorkerTraceEvents,
  assertValidWorkerTrace,
  projectWorkerTrace,
  validateWorkerTraceAppend,
  type WorkerTraceEntry,
} from './worker-trace.js'

describe('worker trace projection', () => {
  it('projects lifecycle events into immutable attempt, output, detection, and result entries', () => {
    const events = [
      workerStartedEvent({
        attempt: 1,
        command: ['npm', 'test'],
        content_hash: 'sha256:started-1',
        cwd: '/workspace',
        engine: { cli: 'codex', model: 'gpt-5' },
        model_tier: 'frontier',
        pid: 101,
        started_at: '2026-07-03T10:00:00.000Z',
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
      workerOutputEvent({
        byte_count: 14,
        content_hash: 'sha256:stdout',
        log_path: 'workers/T1/logs/stdout.log',
        offset: 0,
        observed_at: '2026-07-03T10:00:01.000Z',
        sha256: 'sha256:chunk-1',
        stream: 'stdout',
        tail: 'first line',
        tail_bytes: 10,
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
      workerDetectedEvent({
        content_hash: 'sha256:detected',
        detected_at: '2026-07-03T10:01:00.000Z',
        pid: 101,
        status: 'output-heartbeat-stall',
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
      workerRestartedEvent({
        attempt: 2,
        content_hash: 'sha256:restarted',
        pid: 202,
        previous_pid: 101,
        reason: 'output-heartbeat-stall',
        restarted_at: '2026-07-03T10:02:00.000Z',
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
      workerOutputEvent({
        byte_count: 8,
        offset: 0,
        stream: 'stderr',
        tail: 'retrying',
        tail_bytes: 8,
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
      workerExitedEvent({
        content_hash: 'sha256:exited',
        duration_ms: 1234,
        exit_code: 0,
        exited_at: '2026-07-03T10:03:00.000Z',
        pid: 202,
        signal: null,
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
      workerFinishedEvent({
        content_hash: 'sha256:finished',
        duration_ms: 1300,
        finished_at: '2026-07-03T10:03:01.000Z',
        result_path: 'workers/T1/result.json',
        status: 'completed',
        task_id: 'T1',
        worker_id: 'worker-T1',
      }),
    ]

    expect(projectWorkerTrace(events)).toEqual([
      {
        attempt: 1,
        command: ['npm', 'test'],
        contentHash: 'sha256:started-1',
        cwd: '/workspace',
        engine: { cli: 'codex', model: 'gpt-5' },
        kind: 'attempt',
        modelTier: 'frontier',
        occurredAt: '2026-07-03T10:00:00.000Z',
        pid: 101,
        sourceEventType: 'worker_started',
        taskId: 'T1',
        workerId: 'worker-T1',
      },
      {
        attempt: 1,
        byteCount: 14,
        contentHash: 'sha256:stdout',
        kind: 'output',
        logPath: 'workers/T1/logs/stdout.log',
        occurredAt: '2026-07-03T10:00:01.000Z',
        offset: 0,
        sha256: 'sha256:chunk-1',
        stream: 'stdout',
        tail: 'first line',
        tailBytes: 10,
        taskId: 'T1',
        workerId: 'worker-T1',
      },
      {
        attempt: 1,
        contentHash: 'sha256:detected',
        kind: 'detection',
        occurredAt: '2026-07-03T10:01:00.000Z',
        pid: 101,
        status: 'output-heartbeat-stall',
        taskId: 'T1',
        workerId: 'worker-T1',
      },
      {
        attempt: 2,
        contentHash: 'sha256:restarted',
        kind: 'attempt',
        occurredAt: '2026-07-03T10:02:00.000Z',
        pid: 202,
        previousPid: 101,
        reason: 'output-heartbeat-stall',
        sourceEventType: 'worker_restarted',
        taskId: 'T1',
        workerId: 'worker-T1',
      },
      {
        attempt: 2,
        byteCount: 8,
        kind: 'output',
        offset: 0,
        stream: 'stderr',
        tail: 'retrying',
        tailBytes: 8,
        taskId: 'T1',
        workerId: 'worker-T1',
      },
      {
        attempt: 2,
        contentHash: 'sha256:exited',
        durationMs: 1234,
        exitCode: 0,
        kind: 'result',
        occurredAt: '2026-07-03T10:03:00.000Z',
        pid: 202,
        signal: null,
        sourceEventType: 'worker_exited',
        taskId: 'T1',
        workerId: 'worker-T1',
      },
      {
        attempt: 2,
        contentHash: 'sha256:finished',
        durationMs: 1300,
        kind: 'result',
        occurredAt: '2026-07-03T10:03:01.000Z',
        resultPath: 'workers/T1/result.json',
        sourceEventType: 'worker_finished',
        status: 'completed',
        taskId: 'T1',
        workerId: 'worker-T1',
      },
    ])
  })

  it('appends new lifecycle events on top of an existing trace', () => {
    const existing = projectWorkerTrace([
      workerStartedEvent({
        attempt: 1,
        task_id: 'T2',
        worker_id: 'worker-T2',
      }),
    ])

    const next = appendWorkerTraceEvents(existing, [
      workerOutputEvent({
        byte_count: 5,
        log_path: 'workers/T2/logs/stdout.log',
        offset: 0,
        observed_at: '2026-07-03T10:04:00.000Z',
        stream: 'stdout',
        task_id: 'T2',
        worker_id: 'worker-T2',
      }),
    ])

    expect(next).toEqual([
      {
        attempt: 1,
        kind: 'attempt',
        sourceEventType: 'worker_started',
        taskId: 'T2',
        workerId: 'worker-T2',
      },
      {
        attempt: 1,
        byteCount: 5,
        kind: 'output',
        logPath: 'workers/T2/logs/stdout.log',
        occurredAt: '2026-07-03T10:04:00.000Z',
        offset: 0,
        stream: 'stdout',
        taskId: 'T2',
        workerId: 'worker-T2',
      },
    ])
    expect(existing).toHaveLength(1)
  })
})

describe('worker trace validation', () => {
  it('rejects decreasing output offsets for the same task, attempt, and stream', () => {
    const trace: readonly WorkerTraceEntry[] = [
      {
        attempt: 1,
        kind: 'attempt',
        sourceEventType: 'worker_started',
        taskId: 'T1',
        workerId: 'worker-T1',
      },
      {
        attempt: 1,
        byteCount: 10,
        kind: 'output',
        offset: 20,
        stream: 'stdout',
        taskId: 'T1',
        workerId: 'worker-T1',
      },
      {
        attempt: 1,
        byteCount: 4,
        kind: 'output',
        offset: 19,
        stream: 'stdout',
        taskId: 'T1',
        workerId: 'worker-T1',
      },
    ]

    expect(() => {
      assertValidWorkerTrace(trace)
    }).toThrow(
      'worker trace output offset decreased for T1 attempt 1 stdout',
    )
  })

  it('rejects duplicate terminal events for one attempt', () => {
    const trace: readonly WorkerTraceEntry[] = [
      {
        attempt: 1,
        kind: 'attempt',
        sourceEventType: 'worker_started',
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
        exitCode: 0,
        kind: 'result',
        sourceEventType: 'worker_exited',
        taskId: 'T1',
        workerId: 'worker-T1',
      },
    ]

    expect(() => {
      assertValidWorkerTrace(trace)
    }).toThrow(
      'worker trace has duplicate worker_exited terminal event for T1 attempt 1',
    )
  })

  it('rejects lifecycle events without task ids', () => {
    expect(() =>
      projectWorkerTrace([
        workerStartedEvent({
          attempt: 1,
          worker_id: 'worker-missing-task',
        }),
      ]),
    ).toThrow('worker trace event worker_started at index 0 is missing task_id')

    expect(() =>
      projectWorkerTrace([
        workerStartedEvent({
          attempt: 1,
          task_id: '   ',
          worker_id: 'worker-blank-task',
        }),
      ]),
    ).toThrow('worker trace event worker_started at index 0 is missing task_id')
  })

  it('rejects projected trace entries without task ids', () => {
    expect(() => {
      assertValidWorkerTrace([
        {
          attempt: 1,
          kind: 'attempt',
          sourceEventType: 'worker_started',
          taskId: '',
          workerId: 'worker-empty-task',
        },
      ])
    }).toThrow('worker trace entry at index 0 is missing taskId')
  })

  it('rejects output and result events that cannot be associated with an attempt', () => {
    expect(() =>
      projectWorkerTrace([
        workerOutputEvent({
          byte_count: 1,
          offset: 0,
          stream: 'stdout',
          task_id: 'T3',
          worker_id: 'worker-T3',
        }),
      ]),
    ).toThrow('worker trace event worker_output at index 0 has no active attempt for T3')
  })

  it('rejects append attempts that mutate prior trace entries', () => {
    const previous = projectWorkerTrace([
      workerStartedEvent({
        attempt: 1,
        pid: 100,
        task_id: 'T4',
        worker_id: 'worker-T4',
      }),
    ])
    const mutated: readonly WorkerTraceEntry[] = [
      {
        attempt: 1,
        kind: 'attempt',
        pid: 101,
        sourceEventType: 'worker_started',
        taskId: 'T4',
        workerId: 'worker-T4',
      },
    ]

    expect(() => {
      validateWorkerTraceAppend(previous, mutated)
    }).toThrow(
      'worker trace append mutates prior entry at index 0',
    )
  })

  it('rejects append attempts that remove prior trace entries', () => {
    const previous = projectWorkerTrace([
      workerStartedEvent({
        attempt: 1,
        task_id: 'T5',
        worker_id: 'worker-T5',
      }),
    ])

    expect(() => {
      validateWorkerTraceAppend(previous, [])
    }).toThrow(
      'worker trace append removed prior entries',
    )
  })
})
