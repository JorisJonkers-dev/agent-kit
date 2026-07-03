import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { CouncilApp } from './council-app.js'
import type {
  SuperviseInput,
  SuperviseRunStore,
  SuperviseWorkerSupervisor,
  SuperviseWorkerSupervisorDependencies,
  SuperviseWorkerSupervisorEvent,
  SuperviseWorkerSupervisorResult,
  SuperviseWorkerSupervisorSession,
  SuperviseWorkerSupervisorSnapshot,
  SuperviseWorkerSupervisorStartRequest,
} from './council-app.js'
import {
  appendWorkerTraceEvents,
  createRepairLoopState,
  decideRepairLoop,
  projectWorkerTrace,
  validateWorkerTraceAppend,
} from '../contexts/orchestration/index.js'
import { workerOutputEvent, type RunStoreEvent, type WorkerLifecycleEvent } from '../contexts/runstore/index.js'
import type { RunSummary } from '../workflows/index.js'

const tempRoots: string[] = []

afterEach(async () => {
  await Promise.all(tempRoots.splice(0).map((root) => rm(root, { force: true, recursive: true })))
})

describe('CouncilApp.recommend', () => {
  it('returns lens recommendations without touching IO adapters', async () => {
    const app = new CouncilApp({
      readText: () => Promise.reject(new Error('readText should not be called')),
      writeText: () => Promise.reject(new Error('writeText should not be called')),
    })

    const recommendation = await app.recommend({
      profile: {
        kind: 'api',
        risk: 'high',
        signals: ['timeout budget'],
        size: 'medium',
      },
    })

    expect(recommendation.lenses.length).toBeGreaterThan(0)
    expect(recommendation.workerCount).toBe(recommendation.lenses.length)
  })
})

describe('CouncilApp.triage', () => {
  it('runs the triage gate and emits triage.json through the injected writer', async () => {
    const writes: { readonly path: string; readonly text: string }[] = []
    const app = new CouncilApp({
      writeText(path, text) {
        writes.push({ path, text })
        return Promise.resolve()
      },
    })

    const payload = await app.triage({
      runDir: '/runs/run-a',
      signals: ['shared files'],
      triage: {
        clarity: 'clear',
        kind: 'feature',
        landscape: 'brownfield',
        parallelism: 'high',
        risk: 'medium',
        size: 'medium',
      },
    })

    expect(payload).toMatchObject({
      council_worthy: true,
      route: 'program',
      topology: 'parallel',
    })
    expect(writes).toEqual([
      {
        path: '/runs/run-a/triage.json',
        text: `${JSON.stringify(payload, null, 2)}\n`,
      },
    ])
  })
})

describe('CouncilApp.eval', () => {
  it('scores run artifacts through the injected status and runstore seams', async () => {
    const store = new RecordingRunStore({
      runEvents: [
        { payload: { attempt: 1, task_id: 'T1', worker_id: 'worker-T1' }, type: 'worker_started' },
        {
          payload: { attempt: 2, reason: 'progress-stall', task_id: 'T1', worker_id: 'worker-T1' },
          type: 'worker_restarted',
        },
      ],
    })
    const roots: string[] = []
    const app = new CouncilApp({
      createRunStore(root) {
        roots.push(root)
        return store
      },
      status(input) {
        expect(input).toEqual({ runDir: '/runs/run-a' })
        return Promise.resolve(evalRunSummary())
      },
    })

    const result = await app.eval({ runDir: '/runs/run-a' })

    expect(roots).toEqual(['/runs'])
    expect(store.readEventRunIds).toEqual(['run-a'])
    expect(result).toMatchObject({
      run: 'run-a',
      summary: {
        retry_count: 1,
        task_count: 1,
        worker_result_count: 1,
      },
    })
  })

  it('treats a missing event log as an empty lifecycle stream', async () => {
    const missingEvents = new Error('missing events') as Error & { code: string }
    missingEvents.code = 'ENOENT'
    const store = new RecordingRunStore({ runEventsError: missingEvents })
    const app = new CouncilApp({
      createRunStore: () => store,
      status: () => Promise.resolve(evalRunSummary()),
    })

    await expect(app.eval({ runDir: '/runs/run-a' })).resolves.toMatchObject({
      summary: {
        retry_count: 0,
      },
    })
    expect(store.readEventRunIds).toEqual(['run-a'])
  })

  it('surfaces unexpected event log read failures', async () => {
    const blockedEvents = new Error('events denied') as Error & { code: string }
    blockedEvents.code = 'EACCES'
    const app = new CouncilApp({
      createRunStore: () => new RecordingRunStore({ runEventsError: blockedEvents }),
      status: () => Promise.resolve(evalRunSummary()),
    })

    await expect(app.eval({ runDir: '/runs/run-a' })).rejects.toThrow('events denied')
  })
})

describe('CouncilApp.supervise', () => {
  it('starts a worker and persists lifecycle events, snapshots, and result artifacts', async () => {
    const store = new RecordingRunStore()
    const supervisors: RecordingSupervisor[] = []
    const roots: string[] = []
    const app = new CouncilApp({
      createRunStore: (root) => {
        roots.push(root)
        return store
      },
      createWorkerSupervisor: (dependencies) => {
        const supervisor = new RecordingSupervisor(dependencies, completedSupervisorResult('T1'))
        supervisors.push(supervisor)
        return supervisor
      },
      nowIso: () => '2026-07-03T10:00:00.000Z',
    })

    const result = await app.supervise({
      args: ['-e', 'console.log("ok")'],
      command: 'node',
      mcpProfile: 'code-intel',
      modelTier: 'cheap',
      runDir: '/runs/run-a',
      taskId: 'T1',
      watchdog: { maxRestarts: 1, stallAfterS: 1 },
      worktree: '/worktrees/T1',
    })

    expect(roots).toEqual(['/runs'])
    expect(supervisors[0]?.startRequests).toEqual([
      {
        args: ['-e', 'console.log("ok")'],
        command: 'node',
        id: 'T1',
        mcpProfile: 'code-intel',
        modelTier: 'cheap',
        watchdog: { maxRestarts: 1, stallAfterS: 1 },
        worktree: '/worktrees/T1',
      },
    ])
    expect(supervisors[0]?.reattachRequests).toEqual([])
    expect(store.snapshots).toEqual([
      { runId: 'run-a', snapshot: supervisorSnapshot('T1'), taskId: 'T1' },
    ])
    expect(store.events).toEqual([
      {
        runId: 'run-a',
        event: {
          payload: {
            attempt: 1,
            command: ['node', '-e', 'console.log("ok")'],
            cwd: '/worktrees/T1',
            model_tier: 'cheap',
            pid: 101,
            started_at: '2026-07-03T10:00:00.000Z',
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_started',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            byte_count: 3,
            offset: 0,
            stream: 'stdout',
            tail: 'ok\n',
            tail_bytes: 3,
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_output',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            byte_count: 5,
            offset: 0,
            stream: 'stderr',
            tail: 'warn\n',
            tail_bytes: 5,
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_output',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            detected_at: '2026-07-03T10:00:00.000Z',
            pid: 101,
            status: 'progress-stall',
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_detected',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            attempt: 2,
            pid: 102,
            previous_pid: 101,
            reason: 'progress-stall',
            restarted_at: '2026-07-03T10:00:00.000Z',
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_restarted',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            exit_code: 0,
            exited_at: '2026-07-03T10:00:00.000Z',
            pid: 102,
            signal: null,
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_exited',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            finished_at: '2026-07-03T10:00:00.000Z',
            result_path: 'workers/T1/result.json',
            status: 'ok',
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_finished',
        },
      },
    ])
    expect(result).toEqual({
      model_tier: 'cheap',
      status: 'ok',
      stderr_bytes: 5,
      stderr_log_path: 'workers/T1/logs/stderr.log',
      stderr_tail: 'warn\n',
      stdout_bytes: 3,
      stdout_log_path: 'workers/T1/logs/stdout.log',
      stdout_tail: 'ok\n',
      task_id: 'T1',
      worktree: '/worktrees/T1',
    })
    expect(store.results).toEqual([{ result, runId: 'run-a', taskId: 'T1' }])
  })

  it('projects app-produced failure lifecycle events into append-only traces and repair decisions', async () => {
    const store = new RecordingRunStore()
    const app = new CouncilApp({
      createRunStore: () => store,
      createWorkerSupervisor: (dependencies) => new RecordingSupervisor(dependencies, supervisorResult('T6', 'failed')),
      nowIso: () => '2026-07-03T12:00:00.000Z',
    })

    const result = await app.supervise(superviseInput({ modelTier: 'cheap', taskId: 'T6' }))
    const trace = projectWorkerTrace(store.events.map(({ event }) => event))

    expect(result.status).toBe('failed')
    expect(trace).toEqual([
      {
        attempt: 1,
        command: ['node'],
        cwd: '/worktrees/T6',
        kind: 'attempt',
        modelTier: 'cheap',
        occurredAt: '2026-07-03T12:00:00.000Z',
        pid: 101,
        sourceEventType: 'worker_started',
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 1,
        byteCount: 3,
        kind: 'output',
        offset: 0,
        stream: 'stdout',
        tail: 'ok\n',
        tailBytes: 3,
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 1,
        byteCount: 5,
        kind: 'output',
        offset: 0,
        stream: 'stderr',
        tail: 'warn\n',
        tailBytes: 5,
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 1,
        kind: 'detection',
        occurredAt: '2026-07-03T12:00:00.000Z',
        pid: 101,
        status: 'progress-stall',
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 2,
        kind: 'attempt',
        occurredAt: '2026-07-03T12:00:00.000Z',
        pid: 102,
        previousPid: 101,
        reason: 'progress-stall',
        sourceEventType: 'worker_restarted',
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 2,
        exitCode: 0,
        kind: 'result',
        occurredAt: '2026-07-03T12:00:00.000Z',
        pid: 102,
        signal: null,
        sourceEventType: 'worker_exited',
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 2,
        kind: 'result',
        occurredAt: '2026-07-03T12:00:00.000Z',
        resultPath: 'workers/T6/result.json',
        sourceEventType: 'worker_finished',
        status: 'failed',
        taskId: 'T6',
        workerId: 'worker-T6',
      },
    ])

    const appendedTrace = appendWorkerTraceEvents(trace, [
      workerOutputEvent({
        byte_count: 12,
        offset: 5,
        stream: 'stderr',
        tail: 'still fails\n',
        tail_bytes: 12,
        task_id: 'T6',
        worker_id: 'worker-T6',
      }),
    ])

    expect(appendedTrace.slice(0, trace.length)).toEqual(trace)
    expect(appendedTrace.at(-1)).toEqual({
      attempt: 2,
      byteCount: 12,
      kind: 'output',
      offset: 5,
      stream: 'stderr',
      tail: 'still fails\n',
      tailBytes: 12,
      taskId: 'T6',
      workerId: 'worker-T6',
    })
    expect(() => {
      validateWorkerTraceAppend(
        trace,
        trace.map((entry, index) => (index === 0 && entry.kind === 'attempt' ? { ...entry, pid: 999 } : entry)),
      )
    }).toThrow('worker trace append mutates prior entry at index 0')
    expect(() => {
      validateWorkerTraceAppend(trace, trace.slice(1))
    }).toThrow('worker trace append removed prior entries')

    const firstDecision = decideRepairLoop({
      maxTailChars: 4,
      state: createRepairLoopState(),
      task: {
        id: 'T6',
        verify: 'npm test',
      },
      trace,
      workerResult: {
        status: 'verify-failed',
        verifyOutput: 'expected repairable verification failure',
        verifyRc: 1,
      },
    })

    expect(firstDecision).toEqual({
      kind: 'repair',
      plan: {
        artifacts: {
          stderrTail: 'arn\n',
          stdoutTail: 'ok\n',
          traceSummary: {
            attempts: [1, 2],
            detections: ['progress-stall'],
            latestResultStatus: 'failed',
            resultStatuses: ['worker_exited:exit-0', 'worker_finished:failed'],
            taskId: 'T6',
            workerIds: ['worker-T6'],
          },
          verifyOutput: 'expected repairable verification failure',
          verifyRc: 1,
          workerResultStatus: 'verify-failed',
        },
        attempt: 1,
        taskId: 'T6',
        verifyCommand: 'npm test',
      },
      state: { repairAttemptConsumed: true },
    })

    expect(
      decideRepairLoop({
        maxTailChars: 4,
        state: firstDecision.state,
        task: {
          id: 'T6',
          verify: 'npm test',
        },
        trace,
        workerResult: {
          status: 'verify-failed',
          verifyOutput: 'still failing after repair',
          verifyRc: 1,
        },
      }),
    ).toEqual({
      artifacts: {
        stderrTail: 'arn\n',
        stdoutTail: 'ok\n',
        traceSummary: {
          attempts: [1, 2],
          detections: ['progress-stall'],
          latestResultStatus: 'failed',
          resultStatuses: ['worker_exited:exit-0', 'worker_finished:failed'],
          taskId: 'T6',
          workerIds: ['worker-T6'],
        },
        verifyOutput: 'still failing after repair',
        verifyRc: 1,
        workerResultStatus: 'verify-failed',
      },
      kind: 'terminal-failure',
      reason: 'repair-attempt-consumed',
      state: { repairAttemptConsumed: true },
    })
  })

  it('reattaches from a saved snapshot and writes terminal result statuses', async () => {
    for (const status of ['failed', 'stalled', 'disk-cap', 'stopped'] as const) {
      const store = new RecordingRunStore({ snapshot: supervisorSnapshot('T2') })
      const supervisors: RecordingSupervisor[] = []
      const app = new CouncilApp({
        createRunStore: () => store,
        createWorkerSupervisor: (dependencies) => {
          const supervisor = new RecordingSupervisor(dependencies, supervisorResult('T2', status))
          supervisors.push(supervisor)
          return supervisor
        },
        nowIso: () => '2026-07-03T11:00:00.000Z',
      })

      const result = await app.supervise(superviseInput({ taskId: 'T2' }))

      expect(supervisors[0]?.startRequests).toEqual([])
      expect(supervisors[0]?.reattachRequests).toEqual([
        {
          request: {
            command: 'node',
            id: 'T2',
            worktree: '/worktrees/T2',
          },
          snapshot: supervisorSnapshot('T2'),
        },
      ])
      expect(result.status).toBe(status)
      expect(store.results).toEqual([{ result, runId: 'run-a', taskId: 'T2' }])
      expect(store.events.at(-1)).toEqual({
        runId: 'run-a',
        event: {
          payload: {
            finished_at: '2026-07-03T11:00:00.000Z',
            result_path: 'workers/T2/result.json',
            status,
            task_id: 'T2',
            worker_id: 'worker-T2',
          },
          type: 'worker_finished',
        },
      })
    }
  })

  it('surfaces unexpected snapshot read failures before starting a worker', async () => {
    const app = new CouncilApp({
      createRunStore: () => new RecordingRunStore({ snapshotError: new Error('read denied') }),
      createWorkerSupervisor: () => {
        throw new Error('supervisor should not be created')
      },
    })

    await expect(app.supervise(superviseInput())).rejects.toThrow('read denied')
  })

  it('composes the real fs run store and process supervisor adapters by default', async () => {
    const root = await tempRoot('council-supervise-')
    const runDir = join(root, 'run-real')
    const worktree = await tempRoot('council-supervise-worktree-')

    const result = await new CouncilApp().supervise({
      args: ['-e', 'process.stdout.write("real ok\\n")'],
      command: process.execPath,
      pollIntervalMs: 1,
      runDir,
      taskId: 'T-real',
      worktree,
    })

    expect(result).toMatchObject({
      status: 'ok',
      stdout_tail: 'real ok\n',
      task_id: 'T-real',
      worktree,
    })
    await expect(readFile(join(runDir, 'workers', 'T-real', 'result.json'), 'utf8')).resolves.toContain(
      '"status": "ok"',
    )
    await expect(readFile(join(runDir, 'events.jsonl'), 'utf8')).resolves.toContain('worker_finished')
    await expect(readFile(join(worktree, 'workers', 'T-real', 'logs', 'stdout.log'), 'utf8')).resolves.toBe(
      'real ok\n',
    )
  })

  it('rejects a run directory whose basename is empty', async () => {
    await expect(new CouncilApp().supervise(superviseInput({ runDir: '/' }))).rejects.toThrow(
      '--run must point to a run directory',
    )
  })
})

class RecordingRunStore implements SuperviseRunStore {
  readonly events: { readonly event: WorkerLifecycleEvent; readonly runId: string }[] = []
  readonly readEventRunIds: string[] = []
  readonly results: { readonly result: unknown; readonly runId: string; readonly taskId: string }[] = []
  readonly snapshots: {
    readonly runId: string
    readonly snapshot: SuperviseWorkerSupervisorSnapshot
    readonly taskId: string
  }[] = []
  private readonly runEvents: readonly RunStoreEvent[]
  private readonly runEventsError: Error | undefined
  private readonly snapshot: SuperviseWorkerSupervisorSnapshot | undefined
  private readonly snapshotError: Error | undefined

  constructor(options: {
    readonly runEvents?: readonly RunStoreEvent[]
    readonly runEventsError?: Error
    readonly snapshot?: SuperviseWorkerSupervisorSnapshot
    readonly snapshotError?: Error
  } = {}) {
    this.runEvents = options.runEvents ?? []
    this.runEventsError = options.runEventsError
    this.snapshot = options.snapshot
    this.snapshotError = options.snapshotError
  }

  appendWorkerEvent(runId: string, event: WorkerLifecycleEvent): Promise<void> {
    this.events.push({ event, runId })
    return Promise.resolve()
  }

  readEvents(runId: string): Promise<readonly RunStoreEvent[]> {
    this.readEventRunIds.push(runId)
    if (this.runEventsError !== undefined) return Promise.reject(this.runEventsError)
    return Promise.resolve(this.runEvents)
  }

  readWorkerSupervisorSnapshot(): Promise<SuperviseWorkerSupervisorSnapshot> {
    if (this.snapshot !== undefined) return Promise.resolve(this.snapshot)
    if (this.snapshotError !== undefined) return Promise.reject(this.snapshotError)
    const error = new Error('missing snapshot') as Error & { code: string }
    error.code = 'ENOENT'
    return Promise.reject(error)
  }

  writeWorkerResult(runId: string, taskId: string, result: unknown): Promise<void> {
    this.results.push({ result, runId, taskId })
    return Promise.resolve()
  }

  writeWorkerSupervisorSnapshot(
    runId: string,
    taskId: string,
    snapshot: SuperviseWorkerSupervisorSnapshot,
  ): Promise<void> {
    this.snapshots.push({ runId, snapshot, taskId })
    return Promise.resolve()
  }
}

class RecordingSupervisor implements SuperviseWorkerSupervisor {
  readonly reattachRequests: {
    readonly request: SuperviseWorkerSupervisorStartRequest
    readonly snapshot: SuperviseWorkerSupervisorSnapshot
  }[] = []
  readonly startRequests: SuperviseWorkerSupervisorStartRequest[] = []
  private readonly dependencies: SuperviseWorkerSupervisorDependencies
  private readonly supervisorResult: SuperviseWorkerSupervisorResult

  constructor(
    dependencies: SuperviseWorkerSupervisorDependencies,
    supervisorResult: SuperviseWorkerSupervisorResult,
  ) {
    this.dependencies = dependencies
    this.supervisorResult = supervisorResult
  }

  reattach(
    request: SuperviseWorkerSupervisorStartRequest,
    snapshot: SuperviseWorkerSupervisorSnapshot,
  ): SuperviseWorkerSupervisorSession {
    this.reattachRequests.push({ request, snapshot })
    return { inject: rejectingInject, result: Promise.resolve(this.supervisorResult), stop: resolvingStop }
  }

  start(request: SuperviseWorkerSupervisorStartRequest): SuperviseWorkerSupervisorSession {
    this.startRequests.push(request)
    this.emitFixtureEvents(request)
    return { inject: rejectingInject, result: Promise.resolve(this.supervisorResult), stop: resolvingStop }
  }

  private emitFixtureEvents(request: SuperviseWorkerSupervisorStartRequest): void {
    const detection = {
      idleMs: 1000,
      kind: 'progress-stall',
      lastProgressAtMs: 0,
    } as const
    const events: readonly SuperviseWorkerSupervisorEvent[] = [
      {
        attemptId: 1,
        ...(request.modelTier === undefined ? {} : { modelTier: request.modelTier }),
        pid: 101,
        restart: 1,
        restartCount: 0,
        taskId: request.id,
        type: 'started',
      },
      {
        attemptId: 1,
        byteCount: 3,
        logPath: `workers/${request.id}/logs/stdout.log`,
        offset: 0,
        pid: 101,
        restartCount: 0,
        tail: 'ok\n',
        tailBytes: 3,
        taskId: request.id,
        type: 'stdout',
      },
      {
        attemptId: 1,
        byteCount: 5,
        logPath: `workers/${request.id}/logs/stderr.log`,
        offset: 0,
        pid: 101,
        restartCount: 0,
        tail: 'warn\n',
        tailBytes: 5,
        taskId: request.id,
        type: 'stderr',
      },
      {
        attemptId: 1,
        detection,
        pid: 101,
        restartCount: 0,
        taskId: request.id,
        type: 'detected',
      },
      {
        attemptId: 2,
        detection,
        pid: 102,
        preamble: 'retry',
        previousPid: 101,
        restart: 1,
        restartCount: 1,
        taskId: request.id,
        type: 'restarted',
      },
      {
        attemptId: 2,
        exitCode: 0,
        pid: 102,
        restartCount: 1,
        signal: null,
        taskId: request.id,
        type: 'exited',
      },
      {
        attemptId: 2,
        mode: 'checkpoint-and-resume',
        pid: 102,
        restartCount: 1,
        taskId: request.id,
        type: 'injected',
      },
      {
        attemptId: 2,
        modelTier: 'max',
        pid: 102,
        restartCount: 1,
        taskId: request.id,
        type: 'tier-escalated',
      },
      {
        attemptId: 2,
        pid: 102,
        reason: 'operator',
        restartCount: 1,
        taskId: request.id,
        type: 'stopped',
      },
      {
        attemptId: 2,
        pid: 102,
        restartCount: 1,
        signal: 'SIGTERM',
        taskId: request.id,
        type: 'terminated',
      },
    ]

    events.forEach((event) => {
      this.dependencies.onEvent?.(event)
    })
    void this.dependencies.onSnapshot?.(supervisorSnapshot(request.id))
  }
}

function rejectingInject(): Promise<never> {
  return Promise.reject(new Error('not implemented in test'))
}

function resolvingStop(): Promise<void> {
  return Promise.resolve()
}

function superviseInput(overrides: Partial<SuperviseInput> = {}): SuperviseInput {
  return {
    command: 'node',
    runDir: '/runs/run-a',
    taskId: 'T1',
    worktree: `/worktrees/${overrides.taskId ?? 'T1'}`,
    ...overrides,
  }
}

function completedSupervisorResult(taskId: string): SuperviseWorkerSupervisorResult {
  return supervisorResult(taskId, 'completed')
}

function supervisorResult(
  taskId: string,
  status: SuperviseWorkerSupervisorResult['status'],
): SuperviseWorkerSupervisorResult {
  return {
    exitCode: status === 'failed' ? 1 : 0,
    id: taskId,
    modelTier: 'cheap',
    restarts: 0,
    signal: null,
    status,
    stderr: 'warn\n',
    stderrBytes: 5,
    stderrLogPath: `workers/${taskId}/logs/stderr.log`,
    stdout: 'ok\n',
    stdoutBytes: 3,
    stdoutLogPath: `workers/${taskId}/logs/stdout.log`,
  }
}

function supervisorSnapshot(taskId: string): SuperviseWorkerSupervisorSnapshot {
  return {
    attempt_id: 1,
    logs: {
      stderr: `workers/${taskId}/logs/stderr.log`,
      stdout: `workers/${taskId}/logs/stdout.log`,
    },
    model_tier: 'cheap',
    offsets: {
      stderr: 0,
      stdout: 0,
    },
    pid: 101,
    restart_count: 0,
    status: 'running',
    task_id: taskId,
    watchdog: {
      handling_detection: false,
      loop: { actions: [] },
      progress: {
        attemptStartedAtMs: 0,
        lastActionAtMs: 0,
        lastOutputAtMs: 0,
        lastProgressAtMs: 0,
        outputBytes: 0,
        startedAtMs: 0,
      },
      retry: {
        attempts: 0,
        failureFingerprints: [],
      },
    },
  }
}

function evalRunSummary(): RunSummary {
  return {
    run: 'run-a',
    state: { stage: 'fanout' },
    tasks: [
      {
        boundaries: 'Only touch src/example.ts.',
        depends_on: [],
        difficulty: 'moderate',
        id: 'T1',
        model: 'sonnet',
        objective: 'Score app-level eval wiring.',
        output_format: 'Code edits',
        paths: ['src/example.ts'],
        title: 'Eval app wiring',
        verify: 'npx vitest run src/example.test.ts',
      },
    ],
    waves: [['T1']],
    workerResults: [
      {
        files_changed: ['src/example.ts'],
        status: 'ok',
        task_id: 'T1',
        verdict: {
          engine: { cli: 'codex', model: 'gpt-5' },
          issues: [],
          reasons: 'complete',
          satisfied: true,
          task_id: 'T1',
        },
        verify_rc: 0,
      },
    ],
  }
}

async function tempRoot(prefix: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), prefix))
  tempRoots.push(root)
  return root
}
