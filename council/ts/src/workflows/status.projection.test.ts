import { describe, expect, it } from 'vitest'

import {
  amendmentEvent,
  workerDetectedEvent,
  workerExitedEvent,
  workerFinishedEvent,
  workerOutputEvent,
  workerRestartedEvent,
  workerStartedEvent,
} from '../contexts/runstore/index.js'
import type { RunStoreEvent } from '../contexts/runstore/index.js'
import type { Task } from '../shared-kernel/index.js'

import { projectRunView } from './status.js'
import type { RunSummary, RunViewSupervisorSnapshot } from './status.js'

const NOW = new Date('2026-07-03T10:10:00.000Z')

function task(input: Partial<Task> & Pick<Task, 'id'>): Task {
  const { id, ...overrides } = input
  return {
    boundaries: 'stay in workflow',
    content_hash: id,
    depends_on: [],
    difficulty: 'moderate',
    id,
    model: 'haiku',
    objective: `Implement ${id}`,
    output_format: 'patch',
    paths: [`src/${id}.ts`],
    title: `Task ${id}`,
    verify: 'npm test',
    ...overrides,
  }
}

function summary(input: {
  readonly tasks: readonly Task[]
  readonly waves?: readonly (readonly string[])[]
  readonly workerResults?: RunSummary['workerResults']
}): RunSummary {
  return {
    run: 'run-a',
    state: { stage: 'fanout' },
    tasks: input.tasks,
    waves: input.waves ?? input.tasks.map((item) => [item.id]),
    workerResults: input.workerResults ?? [],
  }
}

function snapshot(input: Partial<RunViewSupervisorSnapshot> & Pick<RunViewSupervisorSnapshot, 'task_id'>): RunViewSupervisorSnapshot {
  const { task_id, ...overrides } = input
  return {
    attempt_id: 1,
    model_tier: 'sonnet',
    restart_count: 0,
    status: 'running',
    task_id,
    watchdog: {
      pending_detection: { kind: 'progress-stall' },
    },
    ...overrides,
  }
}

function project(input: {
  readonly events?: readonly RunStoreEvent[]
  readonly snapshots?: readonly RunViewSupervisorSnapshot[]
  readonly summary: RunSummary
}) {
  return projectRunView({
    clock: { now: () => NOW },
    events: input.events ?? [],
    summary: input.summary,
    supervisorSnapshots: input.snapshots ?? [],
  })
}

describe('projectRunView', () => {
  it('uses lifecycle events before supervisor snapshots for task freshness', () => {
    const tasks = [task({ id: 'T1' })]
    const runSummary = summary({ tasks, waves: [['T1']] })
    const events = [
      amendmentEvent({ content_hash: 'a1', id: 'amend-1', summary: 'ignored by status view' }),
      workerStartedEvent({
        attempt: 2,
        model_tier: 'haiku',
        pid: 101,
        started_at: '2026-07-03T10:00:00.000Z',
        task_id: 'T1',
        worker_id: 'worker-1',
      }),
      workerOutputEvent({
        byte_count: 11,
        offset: 11,
        stream: 'stdout',
        tail: 'hello world',
        worker_id: 'worker-1',
      }),
      workerDetectedEvent({
        detected_at: '2026-07-03T10:01:00.000Z',
        status: 'progress-stall',
        task_id: 'T1',
        worker_id: 'worker-1',
      }),
      workerRestartedEvent({
        attempt: 3,
        reason: 'progress-stall',
        restarted_at: '2026-07-03T10:02:00.000Z',
        task_id: 'T1',
        worker_id: 'worker-1',
      }),
      workerExitedEvent({
        duration_ms: 180_000,
        exit_code: 1,
        exited_at: '2026-07-03T10:03:00.000Z',
        task_id: 'T1',
        worker_id: 'worker-1',
      }),
    ]

    const view = project({
      events,
      snapshots: [
        snapshot({
          attempt_id: 9,
          model_tier: 'opus',
          restart_count: 8,
          status: 'failed',
          task_id: 'T1',
          watchdog: { pending_detection: { kind: 'snapshot-only' } },
        }),
      ],
      summary: runSummary,
    })

    expect(view.tasks).toEqual([
      {
        attempt: 3,
        blockedBy: [],
        dependenciesSatisfied: true,
        durationMs: 180_000,
        lastDetection: 'progress-stall',
        modelTier: 'haiku',
        pid: 101,
        restarts: 2,
        startedAt: '2026-07-03T10:00:00.000Z',
        state: 'exited',
        taskId: 'T1',
        terminalStatus: null,
        title: 'Task T1',
        updatedAt: '2026-07-03T10:03:00.000Z',
        wave: 0,
        workerId: 'worker-1',
      },
    ])
    expect(view.rollup.countsByState).toEqual({ exited: 1 })
    expect(runSummary.tasks).toBe(tasks)
    expect(runSummary.waves).toEqual([['T1']])
  })

  it('uses supervisor snapshots only where lifecycle events leave gaps', () => {
    const view = project({
      snapshots: [
        snapshot({
          attempt_id: 4,
          model_tier: 'opus',
          restart_count: 3,
          status: 'detected',
          task_id: 'T1',
          watchdog: { pending_detection: { kind: 'budget-cap' } },
        }),
      ],
      summary: summary({ tasks: [task({ id: 'T1' })] }),
    })

    expect(view.tasks[0]).toMatchObject({
      attempt: 4,
      durationMs: 0,
      lastDetection: 'budget-cap',
      modelTier: 'opus',
      restarts: 3,
      startedAt: null,
      state: 'detected',
      updatedAt: null,
    })
  })

  it('projects worker-result terminal states without requiring lifecycle artifacts', () => {
    const tasks = [
      task({ id: 'T1' }),
      task({ id: 'T2' }),
      task({ id: 'T3' }),
      task({ id: 'T4' }),
      task({ id: 'T5' }),
      task({ id: 'T6' }),
      task({ id: 'T7' }),
      task({ id: 'T8' }),
      task({ id: 'T9' }),
      task({ id: 'T10' }),
    ]

    const view = project({
      summary: summary({
        tasks,
        workerResults: [
          { model_tier: 'sonnet', status: 'ok', task_id: 'T1' },
          { status: 'failed', task_id: 'T2' },
          { status: 'stalled', task_id: 'T3' },
          { status: 'budget-cap', task_id: 'T4' },
          { status: 'disk-cap', task_id: 'T5' },
          { status: 'dead-snapshot', task_id: 'T6' },
          { status: 'stale-snapshot', task_id: 'T7' },
          { status: 'stopped', task_id: 'T8' },
          { status: 'no-op', task_id: 'T9' },
          { status: 'unexpected-error', task_id: 'T10' },
        ],
      }),
    })

    expect(view.tasks.map((item) => [item.taskId, item.state, item.terminalStatus])).toEqual([
      ['T1', 'succeeded', 'ok'],
      ['T2', 'failed', 'failed'],
      ['T3', 'stalled', 'stalled'],
      ['T4', 'budget-cap', 'budget-cap'],
      ['T5', 'disk-cap', 'disk-cap'],
      ['T6', 'dead-snapshot', 'dead-snapshot'],
      ['T7', 'stale-snapshot', 'stale-snapshot'],
      ['T8', 'stopped', 'stopped'],
      ['T9', 'succeeded', 'no-op'],
      ['T10', 'failed', 'unexpected-error'],
    ])
    expect(view.tasks[0]?.modelTier).toBe('sonnet')
    expect(view.rollup.countsByState).toEqual({
      'budget-cap': 1,
      'dead-snapshot': 1,
      'disk-cap': 1,
      failed: 2,
      'stale-snapshot': 1,
      stalled: 1,
      stopped: 1,
      succeeded: 2,
    })
  })

  it('keeps missing and partial run artifacts deterministic and dependency-blocked', () => {
    const view = project({
      events: [
        workerStartedEvent({ task_id: 'T99', worker_id: 'worker-99' }),
        workerOutputEvent({
          byte_count: 1,
          offset: 1,
          stream: 'stderr',
          task_id: 'T99',
          worker_id: 'worker-99',
        }),
        workerDetectedEvent({ task_id: 'T99', worker_id: 'worker-99' }),
        workerRestartedEvent({ attempt: 2, task_id: 'T99', worker_id: 'worker-99' }),
        workerExitedEvent({ exit_code: 0, task_id: 'T99', worker_id: 'worker-99' }),
        workerFinishedEvent({ status: 'ok', task_id: 'T99', worker_id: 'worker-99' }),
      ],
      snapshots: [snapshot({ task_id: 'T99' })],
      summary: summary({
        tasks: [
          task({ id: 'T1' }),
          task({ depends_on: ['T1'], id: 'T2' }),
          task({ depends_on: ['T404'], id: 'T3' }),
        ],
        waves: [],
        workerResults: [{ status: 'ok', task_id: 'T99' }],
      }),
    })

    expect(view.tasks.map((item) => ({
      blockedBy: item.blockedBy,
      dependenciesSatisfied: item.dependenciesSatisfied,
      state: item.state,
      taskId: item.taskId,
      wave: item.wave,
    }))).toEqual([
      { blockedBy: [], dependenciesSatisfied: true, state: 'pending', taskId: 'T1', wave: null },
      { blockedBy: ['T1'], dependenciesSatisfied: false, state: 'blocked', taskId: 'T2', wave: null },
      { blockedBy: ['T404'], dependenciesSatisfied: false, state: 'blocked', taskId: 'T3', wave: null },
    ])
    expect(view.rollup.readySet).toEqual(['T1'])
    expect(view.rollup.elapsedMs).toBe(0)
  })

  it('orders the ready set and critical path by longest remaining path then stable task order', () => {
    const view = project({
      summary: summary({
        tasks: [
          task({ id: 'T2' }),
          task({ id: 'T1' }),
          task({ depends_on: ['T1'], id: 'T3' }),
          task({ depends_on: ['T3'], id: 'T4' }),
          task({ depends_on: ['T2'], id: 'T5' }),
          task({ depends_on: ['T2'], id: 'T6' }),
          task({ id: 'T7' }),
          task({ depends_on: ['T5'], id: 'T8' }),
          task({ depends_on: ['T6'], id: 'T9' }),
        ],
      }),
    })

    expect(view.rollup.readySet).toEqual(['T2', 'T1', 'T7'])
    expect(view.rollup.criticalPath).toEqual(['T2', 'T5', 'T8'])
  })

  it('calculates task durations and run elapsed time with the injected clock', () => {
    const view = project({
      events: [
        workerStartedEvent({
          started_at: '2026-07-03T10:00:00.000Z',
          task_id: 'T1',
          worker_id: 'worker-1',
        }),
        workerStartedEvent({
          started_at: '2026-07-03T10:01:00.000Z',
          task_id: 'T2',
          worker_id: 'worker-2',
        }),
        workerFinishedEvent({
          duration_ms: 60_000,
          finished_at: '2026-07-03T10:02:00.000Z',
          status: 'ok',
          task_id: 'T2',
          worker_id: 'worker-2',
        }),
      ],
      summary: summary({ tasks: [task({ id: 'T1' }), task({ id: 'T2' })] }),
    })

    expect(view.tasks.map((item) => [item.taskId, item.state, item.durationMs])).toEqual([
      ['T1', 'running', 600_000],
      ['T2', 'succeeded', 60_000],
    ])
    expect(view.rollup).toMatchObject({
      elapsedMs: 600_000,
      startedAt: '2026-07-03T10:00:00.000Z',
      updatedAt: '2026-07-03T10:02:00.000Z',
    })
  })
})
