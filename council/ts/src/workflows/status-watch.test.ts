import { describe, expect, it } from 'vitest'

import { workerFinishedEvent, workerStartedEvent } from '../contexts/runstore/index.js'
import type {
  LiveRunArtifacts,
  NormalizedRunDirectory,
  WorkerResult,
  WorkerSupervisorSnapshot,
} from '../ports/index.js'
import type { Task } from '../shared-kernel/index.js'

import {
  DEFAULT_STATUS_WATCH_INTERVAL_MS,
  statusWatchWorkflow,
  type StatusWatchFrame,
  type StatusWatchTickerPort,
} from './status-watch.js'

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

function workerResult(input: WorkerResult): WorkerResult {
  return input
}

function supervisorSnapshot(input: Partial<WorkerSupervisorSnapshot> & Pick<WorkerSupervisorSnapshot, 'task_id'>): WorkerSupervisorSnapshot {
  const { task_id, ...overrides } = input
  return {
    attempt_id: 1,
    logs: { stderr: 'err.log', stdout: 'out.log' },
    model_tier: 'sonnet',
    offsets: { stderr: 0, stdout: 0 },
    restart_count: 0,
    status: 'running',
    task_id,
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
      retry: { attempts: 0, failureFingerprints: [] },
    },
    ...overrides,
  }
}

function normalized(input: {
  readonly report?: NormalizedRunDirectory['report']
  readonly runId?: string
  readonly tasks: readonly Task[]
  readonly workerResults?: ReadonlyMap<string, WorkerResult>
}): NormalizedRunDirectory {
  return {
    report: input.report,
    runId: input.runId ?? 'run-a',
    state: { stage: 'fanout' },
    tasks: input.tasks,
    workerResults: input.workerResults ?? new Map<string, WorkerResult>(),
  }
}

function artifacts(input: {
  readonly events?: LiveRunArtifacts['events']
  readonly normalized: NormalizedRunDirectory
  readonly snapshots?: ReadonlyMap<string, WorkerSupervisorSnapshot>
  readonly workerResults?: ReadonlyMap<string, WorkerResult>
}): LiveRunArtifacts {
  return {
    events: input.events ?? [],
    normalized: input.normalized,
    workerResults: input.workerResults ?? input.normalized.workerResults,
    workerSupervisorSnapshots: input.snapshots ?? new Map(),
  }
}

function createReader(reads: readonly LiveRunArtifacts[]): {
  readonly calls: readonly string[]
  readonly readRunDir: (runDir: string) => Promise<LiveRunArtifacts>
} {
  const calls: string[] = []
  let index = 0
  return {
    calls,
    readRunDir(runDir) {
      calls.push(runDir)
      const current = reads[index] ?? reads.at(-1)
      index += 1
      if (current === undefined) throw new Error('missing test artifact')
      return Promise.resolve(current)
    },
  }
}

function failingReader(error: Error): (runDir: string) => Promise<LiveRunArtifacts> {
  return () => Promise.reject(error)
}

function ticker(ticks: readonly unknown[]): StatusWatchTickerPort & { readonly intervals: readonly number[] } {
  const intervals: number[] = []
  return {
    intervals,
    ticks(input) {
      intervals.push(input.intervalMs)
      let index = 0
      const iterable: AsyncIterable<unknown> & AsyncIterator<unknown> = {
        [Symbol.asyncIterator](): AsyncIterator<unknown> {
          return iterable
        },
        next(): Promise<IteratorResult<unknown>> {
          const value = ticks[index]
          index += 1
          return Promise.resolve(value === undefined ? { done: true, value } : { done: false, value })
        },
      }
      return iterable
    },
  }
}

async function collectFrames(input: Parameters<typeof statusWatchWorkflow>[0], deps: Parameters<typeof statusWatchWorkflow>[1]): Promise<readonly StatusWatchFrame[]> {
  const frames: StatusWatchFrame[] = []
  for await (const frame of statusWatchWorkflow(input, deps)) {
    frames.push(frame)
  }
  return frames
}

describe('statusWatchWorkflow', () => {
  it('renders a --json one-shot snapshot from live run artifacts', async () => {
    const tasks = [task({ id: 'T1' })]
    const reader = createReader([
      artifacts({
        events: [
          workerStartedEvent({
            attempt: 1,
            model_tier: 'haiku',
            pid: 4321,
            started_at: '2026-07-03T10:00:00.000Z',
            task_id: 'T1',
            worker_id: 'worker-1',
          }),
        ],
        normalized: normalized({ tasks }),
        snapshots: new Map([['T1', supervisorSnapshot({ task_id: 'T1' })]]),
      }),
    ])

    await expect(
      collectFrames(
        { json: true, runDir: '/runs/run-a' },
        { clock: { now: () => NOW }, readRunDir: reader.readRunDir },
      ),
    ).resolves.toEqual([
      {
        format: 'json',
        output: `{
  "rollup": {
    "countsByState": {
      "running": 1
    },
    "criticalPath": [
      "T1"
    ],
    "elapsedMs": 600000,
    "readySet": [],
    "startedAt": "2026-07-03T10:00:00.000Z",
    "updatedAt": "2026-07-03T10:00:00.000Z"
  },
  "run": "run-a",
  "state": {
    "stage": "fanout"
  },
  "tasks": [
    {
      "attempt": 1,
      "blockedBy": [],
      "dependenciesSatisfied": true,
      "durationMs": 600000,
      "lastDetection": null,
      "modelTier": "haiku",
      "pid": 4321,
      "restarts": 0,
      "startedAt": "2026-07-03T10:00:00.000Z",
      "state": "running",
      "taskId": "T1",
      "terminalStatus": null,
      "title": "Task T1",
      "updatedAt": "2026-07-03T10:00:00.000Z",
      "wave": 0,
      "workerId": "worker-1"
    }
  ],
  "waves": [
    [
      "T1"
    ]
  ]
}\n`,
        sequence: 0,
      },
    ])
    expect(reader.calls).toEqual(['/runs/run-a'])
  })

  it('renders a --once table snapshot without requiring a ticker', async () => {
    const tasks = [
      task({ id: 'T1' }),
      task({ depends_on: ['T1'], id: 'T2' }),
    ]
    const reader = createReader([
      artifacts({
        normalized: normalized({
          report: {
            run: 'run-a',
            tasks: [],
            waves: [['T1'], ['T2']],
          },
          tasks,
          workerResults: new Map([['T1', workerResult({ status: 'ok', task_id: 'T1' })]]),
        }),
      }),
    ])

    await expect(
      collectFrames(
        { once: true, runDir: '/runs/run-a' },
        { clock: { now: () => NOW }, readRunDir: reader.readRunDir },
      ),
    ).resolves.toEqual([
      {
        format: 'table',
        output: `run run-a stage=fanout elapsed=0s started=- updated=-
rollup counts=ready:1 succeeded:1 ready=T2 critical=T2
active -
wave 0
badge     task  duration  details
[OK]      T1    0s        Task T1; terminal=ok
wave 1
badge     task  duration  details
[READY]   T2    0s        Task T2
`,
        sequence: 0,
      },
    ])
  })

  it('polls on injected watch ticks and renders every changed frame', async () => {
    const tasks = [task({ id: 'T1' })]
    const watchTicker = ticker(['tick-1', 'tick-2'])
    const reader = createReader([
      artifacts({
        normalized: normalized({ tasks }),
      }),
      artifacts({
        events: [
          workerStartedEvent({
            attempt: 1,
            started_at: '2026-07-03T10:05:00.000Z',
            task_id: 'T1',
            worker_id: 'worker-1',
          }),
        ],
        normalized: normalized({ tasks }),
      }),
      artifacts({
        events: [
          workerFinishedEvent({
            duration_ms: 30_000,
            finished_at: '2026-07-03T10:05:30.000Z',
            status: 'ok',
            task_id: 'T1',
            worker_id: 'worker-1',
          }),
        ],
        normalized: normalized({ tasks }),
      }),
    ])

    const frames = await collectFrames(
      { intervalMs: 250, runDir: '/runs/run-a' },
      {
        clock: { now: () => NOW },
        readRunDir: reader.readRunDir,
        ticker: watchTicker,
      },
    )

    expect(frames.map((frame) => frame.sequence)).toEqual([0, 1, 2])
    expect(frames.map((frame) => frame.output.split('\n')[1])).toEqual([
      'rollup counts=ready:1 ready=T1 critical=T1',
      'rollup counts=running:1 ready=- critical=T1',
      'rollup counts=succeeded:1 ready=- critical=-',
    ])
    expect(reader.calls).toEqual(['/runs/run-a', '/runs/run-a', '/runs/run-a'])
    expect(watchTicker.intervals).toEqual([250])
  })

  it('suppresses unchanged watch frames when requested', async () => {
    const tasks = [task({ id: 'T1' })]
    const watchTicker = ticker(['tick-1', 'tick-2'])
    const unchanged = artifacts({ normalized: normalized({ tasks }) })
    const reader = createReader([
      unchanged,
      unchanged,
      artifacts({
        normalized: normalized({
          tasks,
          workerResults: new Map([['T1', workerResult({ status: 'ok', task_id: 'T1' })]]),
        }),
      }),
    ])

    const frames = await collectFrames(
      { runDir: '/runs/run-a', suppressUnchanged: true },
      {
        clock: { now: () => NOW },
        readRunDir: reader.readRunDir,
        ticker: watchTicker,
      },
    )

    expect(frames.map((frame) => ({ output: frame.output.split('\n')[1], sequence: frame.sequence }))).toEqual([
      { output: 'rollup counts=ready:1 ready=T1 critical=T1', sequence: 0 },
      { output: 'rollup counts=succeeded:1 ready=- critical=-', sequence: 1 },
    ])
    expect(reader.calls).toEqual(['/runs/run-a', '/runs/run-a', '/runs/run-a'])
    expect(watchTicker.intervals).toEqual([DEFAULT_STATUS_WATCH_INTERVAL_MS])
  })

  it('keeps partial run directories deterministic by planning waves from tasks', async () => {
    const tasks = [
      task({ id: 'T1' }),
      task({ depends_on: ['T1'], id: 'T2' }),
      task({ depends_on: ['T2'], id: 'T3' }),
    ]
    const reader = createReader([
      artifacts({
        normalized: normalized({
          tasks,
          workerResults: new Map([['T1', workerResult({ status: 'ok', task_id: 'T1' })]]),
        }),
        workerResults: new Map([['T2', workerResult({ status: 'ok', task_id: 'T2' })]]),
      }),
    ])

    const frames = await collectFrames(
      { once: true, runDir: '/runs/partial' },
      { clock: { now: () => NOW }, readRunDir: reader.readRunDir },
    )

    expect(frames[0]?.output).toContain(`rollup counts=ready:1 succeeded:2 ready=T3 critical=T3`)
    expect(frames[0]?.output).toContain(`wave 0
badge     task  duration  details
[OK]      T1    0s        Task T1; terminal=ok
wave 1
badge     task  duration  details
[OK]      T2    0s        Task T2; terminal=ok
wave 2
badge     task  duration  details
[READY]   T3    0s        Task T3
`)
  })

  it('propagates adapter errors without manufacturing output frames', async () => {
    await expect(
      collectFrames(
        { once: true, runDir: '/runs/broken' },
        { clock: { now: () => NOW }, readRunDir: failingReader(new Error('read failed')) },
      ),
    ).rejects.toThrow('read failed')
  })

  it('requires a ticker for watch mode', async () => {
    await expect(
      collectFrames(
        { runDir: '/runs/run-a' },
        { clock: { now: () => NOW }, readRunDir: createReader([]).readRunDir },
      ),
    ).rejects.toThrow('ticker dependency is required for status watch mode')
  })
})
