import { describe, expect, it } from 'vitest'

import { workerOutputEvent, type RunStoreEvent } from '../contexts/runstore/index.js'
import type {
  LiveRunArtifacts,
  LiveRunDirReaderPort,
  NormalizedRunDirectory,
  WorkerResult,
  WorkerSupervisorSnapshot,
} from '../ports/index.js'
import type { RunState, Task, TaskId } from '../shared-kernel/index.js'

import {
  tailWorkflow,
  type TailWorkflowDeps,
  type TailWorkflowFrame,
  type TailWorkflowLogReadInput,
  type TailWorkflowLogReaderPort,
  type TailWorkflowLogStatInput,
  type TailWorkflowTicker,
} from './tail-workflow.js'

function bytes(text: string): Uint8Array {
  return new TextEncoder().encode(text)
}

class SequencedArtifactsReader implements LiveRunDirReaderPort {
  readonly runDirs: string[] = []
  private index = 0

  constructor(private readonly items: readonly [LiveRunArtifacts, ...LiveRunArtifacts[]]) {}

  readRunDir(runDir: string): Promise<LiveRunArtifacts> {
    this.runDirs.push(runDir)
    const item = this.items[Math.min(this.index, this.items.length - 1)] ?? this.items[0]
    this.index += 1
    return Promise.resolve(item)
  }
}

class MemoryLogReader implements TailWorkflowLogReaderPort {
  readonly reads: TailWorkflowLogReadInput[] = []
  readonly stats: TailWorkflowLogStatInput[] = []
  private readonly logs = new Map<string, Uint8Array>()

  set(path: string, text: string): void {
    this.logs.set(path, bytes(text))
  }

  delete(path: string): void {
    this.logs.delete(path)
  }

  stat(input: TailWorkflowLogStatInput): Promise<{ readonly sizeBytes: number } | undefined> {
    this.stats.push(input)
    const buffer = this.logs.get(input.path)
    return Promise.resolve(buffer === undefined ? undefined : { sizeBytes: buffer.byteLength })
  }

  read(input: TailWorkflowLogReadInput): Promise<Uint8Array> {
    this.reads.push(input)
    const buffer = this.logs.get(input.path) ?? new Uint8Array()
    return Promise.resolve(buffer.subarray(input.start, input.end))
  }
}

function ticker(onTicks: readonly (() => void)[] = []): TailWorkflowTicker {
  return {
    async *ticks(): AsyncIterable<void> {
      for (const onTick of onTicks) {
        onTick()
        await Promise.resolve()
        yield
      }
    },
  }
}

async function collect(input: Parameters<typeof tailWorkflow>[0], deps: TailWorkflowDeps): Promise<readonly TailWorkflowFrame[]> {
  const frames: TailWorkflowFrame[] = []
  for await (const frame of tailWorkflow(input, deps)) frames.push(frame)
  return frames
}

function deps(
  artifacts: readonly [LiveRunArtifacts, ...LiveRunArtifacts[]],
  logs: MemoryLogReader,
  tickSource: TailWorkflowTicker = ticker(),
): TailWorkflowDeps {
  return {
    artifacts: new SequencedArtifactsReader(artifacts),
    logs,
    ticker: tickSource,
  }
}

function task(id: TaskId = 'T1'): Task {
  return {
    boundaries: 'Only workflow code.',
    content_hash: `sha256:${id}`,
    depends_on: [],
    difficulty: 'moderate',
    id,
    model: 'sonnet',
    objective: `Tail ${id}.`,
    output_format: 'Frame stream.',
    paths: ['council/ts/src/workflows/tail-workflow.ts'],
    title: `Task ${id}`,
    verify: 'npx vitest run src/workflows/tail-workflow.test.ts',
  }
}

function runState(): RunState {
  return {
    stage: 'fanout',
  }
}

type ResultWithLogPaths = WorkerResult & {
  readonly stderr_log_path?: string
  readonly stdout_log_path?: string
}

function result(taskId: string, paths: { readonly stderr?: string; readonly stdout?: string }): WorkerResult {
  const value: ResultWithLogPaths = {
    status: 'ok',
    task_id: taskId,
    ...(paths.stderr === undefined ? {} : { stderr_log_path: paths.stderr }),
    ...(paths.stdout === undefined ? {} : { stdout_log_path: paths.stdout }),
  }
  return value
}

function snapshot(taskId: string, paths: { readonly stderr: string; readonly stdout: string }): WorkerSupervisorSnapshot {
  return {
    attempt_id: 1,
    logs: {
      stderr: paths.stderr,
      stdout: paths.stdout,
    },
    offsets: {
      stderr: 0,
      stdout: 0,
    },
    restart_count: 0,
    status: 'running',
    task_id: taskId,
    watchdog: {
      handling_detection: false,
      loop: {
        actions: [],
      },
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

function outputEvent(input: {
  readonly byteCount: number
  readonly observedAt?: string
  readonly offset: number
  readonly path?: string
  readonly stream: 'stderr' | 'stdout'
  readonly taskId?: string
}): RunStoreEvent {
  return workerOutputEvent({
    byte_count: input.byteCount,
    offset: input.offset,
    stream: input.stream,
    worker_id: `worker-${input.taskId ?? 'unknown'}`,
    ...(input.observedAt === undefined ? {} : { observed_at: input.observedAt }),
    ...(input.path === undefined ? {} : { log_path: input.path }),
    ...(input.taskId === undefined ? {} : { task_id: input.taskId }),
  })
}

function artifacts(input: {
  readonly events?: readonly RunStoreEvent[]
  readonly results?: readonly WorkerResult[]
  readonly snapshots?: readonly WorkerSupervisorSnapshot[]
  readonly tasks?: readonly Task[]
} = {}): LiveRunArtifacts {
  const workerResults = new Map((input.results ?? []).map((item) => [item.task_id, item]))
  const normalized: NormalizedRunDirectory = {
    report: undefined,
    runId: 'run-a',
    state: runState(),
    tasks: input.tasks ?? [task()],
    workerResults,
  }
  return {
    events: input.events ?? [],
    normalized,
    workerResults,
    workerSupervisorSnapshots: new Map((input.snapshots ?? []).map((item) => [item.task_id, item])),
  }
}

describe('tailWorkflow', () => {
  it('defaults to stdout and reads only the bounded window from result metadata', async () => {
    const logs = new MemoryLogReader()
    logs.set('workers/T1/logs/stdout.result.log', 'alpha\nbeta\ngamma\n')

    const frames = await collect(
      { maxBytes: 11, runDir: '/runs/run-a', taskId: 'T1' },
      deps([artifacts({ results: [result('T1', { stdout: 'workers/T1/logs/stdout.result.log' })] })], logs),
    )

    expect(frames).toEqual([
      {
        chunks: [{ byteCount: 11, offset: 6, stream: 'stdout', text: 'beta\ngamma\n' }],
        cursor: { offset: 17, stream: 'stdout' },
        logPath: 'workers/T1/logs/stdout.result.log',
        logPathSource: 'result',
        missing: false,
        rotated: false,
        stream: 'stdout',
        taskId: 'T1',
        truncated: true,
      },
    ])
    expect(logs.reads).toEqual([
      {
        end: 17,
        path: 'workers/T1/logs/stdout.result.log',
        runDir: '/runs/run-a',
        start: 6,
      },
    ])
  })

  it('selects stderr when requested', async () => {
    const logs = new MemoryLogReader()
    logs.set('workers/T1/logs/stderr.result.log', 'warn\n')

    const frames = await collect(
      { maxBytes: 100, runDir: '/runs/run-a', stream: 'stderr', taskId: 'T1' },
      deps([artifacts({ results: [result('T1', { stderr: 'workers/T1/logs/stderr.result.log' })] })], logs),
    )

    expect(frames[0]).toMatchObject({
      chunks: [{ byteCount: 5, offset: 0, stream: 'stderr', text: 'warn\n' }],
      cursor: { offset: 5, stream: 'stderr' },
      logPath: 'workers/T1/logs/stderr.result.log',
      stream: 'stderr',
    })
  })

  it('resolves log paths by result, snapshot, then output event precedence', async () => {
    const logs = new MemoryLogReader()
    logs.set('workers/T1/logs/stdout.result.log', 'result\n')
    logs.set('workers/T1/logs/stdout.snapshot.log', 'snapshot\n')
    logs.set('workers/T1/logs/stdout.event.log', 'event\n')

    const resultFrame = await collect(
      { maxBytes: 100, runDir: '/runs/run-a', taskId: 'T1' },
      deps(
        [
          artifacts({
            events: [outputEvent({ byteCount: 6, offset: 0, path: 'workers/T1/logs/stdout.event.log', stream: 'stdout', taskId: 'T1' })],
            results: [result('T1', { stdout: 'workers/T1/logs/stdout.result.log' })],
            snapshots: [snapshot('T1', { stderr: 'workers/T1/logs/stderr.snapshot.log', stdout: 'workers/T1/logs/stdout.snapshot.log' })],
          }),
        ],
        logs,
      ),
    )
    const snapshotFrame = await collect(
      { maxBytes: 100, runDir: '/runs/run-a', taskId: 'T1' },
      deps(
        [
          artifacts({
            events: [outputEvent({ byteCount: 6, offset: 0, path: 'workers/T1/logs/stdout.event.log', stream: 'stdout', taskId: 'T1' })],
            snapshots: [snapshot('T1', { stderr: 'workers/T1/logs/stderr.snapshot.log', stdout: 'workers/T1/logs/stdout.snapshot.log' })],
          }),
        ],
        logs,
      ),
    )
    const eventFrame = await collect(
      { maxBytes: 100, runDir: '/runs/run-a', taskId: 'T1' },
      deps(
        [
          artifacts({
            events: [
              outputEvent({ byteCount: 6, offset: 0, path: 'workers/T1/logs/stdout.event.log', stream: 'stdout', taskId: 'T1' }),
              outputEvent({ byteCount: 7, offset: 0, stream: 'stdout', taskId: 'T1' }),
              outputEvent({ byteCount: 8, offset: 0, path: 'workers/T2/logs/stdout.log', stream: 'stdout', taskId: 'T2' }),
              { payload: { id: 'A1', summary: 'ignored' }, type: 'amendment' },
            ],
          }),
        ],
        logs,
      ),
    )

    expect(resultFrame[0]).toMatchObject({ logPath: 'workers/T1/logs/stdout.result.log', logPathSource: 'result' })
    expect(snapshotFrame[0]).toMatchObject({
      logPath: 'workers/T1/logs/stdout.snapshot.log',
      logPathSource: 'snapshot',
    })
    expect(eventFrame[0]).toMatchObject({ logPath: 'workers/T1/logs/stdout.event.log', logPathSource: 'event' })
  })

  it('continues from explicit offsets and follow cursors', async () => {
    const logs = new MemoryLogReader()
    logs.set('workers/T1/logs/stdout.log', 'hello\nworld\n')

    const offsetFrames = await collect(
      { maxBytes: 100, offset: 6, runDir: '/runs/run-a', taskId: 'T1' },
      deps([artifacts({ results: [result('T1', { stdout: 'workers/T1/logs/stdout.log' })] })], logs),
    )
    const cursorFrames = await collect(
      { cursor: { offset: 6, stream: 'stdout' }, maxBytes: 100, runDir: '/runs/run-a', taskId: 'T1' },
      deps([artifacts({ results: [result('T1', { stdout: 'workers/T1/logs/stdout.log' })] })], logs),
    )

    expect(offsetFrames[0]?.chunks).toEqual([{ byteCount: 6, offset: 6, stream: 'stdout', text: 'world\n' }])
    expect(cursorFrames[0]?.cursor).toEqual({ offset: 12, stream: 'stdout' })
  })

  it('applies line and since filters from output event metadata', async () => {
    const logs = new MemoryLogReader()
    logs.set('workers/T1/logs/stdout.log', 'old\nnew\nlater\n')

    const frames = await collect(
      {
        lines: 1,
        maxBytes: 100,
        runDir: '/runs/run-a',
        since: '2026-07-03T10:05:00.000Z',
        taskId: 'T1',
      },
      deps(
        [
          artifacts({
            events: [
              outputEvent({ byteCount: 4, observedAt: '2026-07-03T10:00:00.000Z', offset: 0, stream: 'stdout', taskId: 'T1' }),
              outputEvent({ byteCount: 4, observedAt: '2026-07-03T10:05:00.000Z', offset: 4, stream: 'stdout', taskId: 'T1' }),
              outputEvent({ byteCount: 6, observedAt: '2026-07-03T10:06:00.000Z', offset: 8, stream: 'stdout', taskId: 'T1' }),
            ],
            results: [result('T1', { stdout: 'workers/T1/logs/stdout.log' })],
          }),
        ],
        logs,
      ),
    )

    expect(frames[0]?.chunks).toEqual([{ byteCount: 6, offset: 8, stream: 'stdout', text: 'later\n' }])
    expect(frames[0]?.cursor).toEqual({ offset: 14, stream: 'stdout' })
  })

  it('reports missing tasks without touching log dependencies', async () => {
    const logs = new MemoryLogReader()

    const frames = await collect(
      { maxBytes: 100, runDir: '/runs/run-a', taskId: 'T-missing' },
      deps([artifacts({ tasks: [task('T1')] })], logs),
    )

    expect(frames).toEqual([
      {
        chunks: [],
        cursor: { offset: 0, stream: 'stdout' },
        missing: true,
        missingReason: 'task',
        rotated: false,
        stream: 'stdout',
        taskId: 'T-missing',
        truncated: false,
      },
    ])
    expect(logs.stats).toEqual([])
    expect(logs.reads).toEqual([])
  })

  it('reports missing logs when metadata is absent or the selected path cannot be statted', async () => {
    const logs = new MemoryLogReader()
    logs.delete('workers/T1/logs/stdout.log')

    const withoutMetadata = await collect(
      { maxBytes: 100, runDir: '/runs/run-a', taskId: 'T1' },
      deps([artifacts()], logs),
    )
    const withoutFile = await collect(
      { maxBytes: 100, runDir: '/runs/run-a', taskId: 'T1' },
      deps([artifacts({ results: [result('T1', { stdout: 'workers/T1/logs/stdout.log' })] })], logs),
    )

    expect(withoutMetadata[0]).toEqual({
      chunks: [],
      cursor: { offset: 0, stream: 'stdout' },
      missing: true,
      missingReason: 'log',
      rotated: false,
      stream: 'stdout',
      taskId: 'T1',
      truncated: false,
    })
    expect(withoutFile[0]).toEqual({
      chunks: [],
      cursor: { offset: 0, stream: 'stdout' },
      logPath: 'workers/T1/logs/stdout.log',
      logPathSource: 'result',
      missing: true,
      missingReason: 'log',
      rotated: false,
      stream: 'stdout',
      taskId: 'T1',
      truncated: false,
    })
  })

  it('reports rotation when a cursor is beyond the current log size', async () => {
    const logs = new MemoryLogReader()
    logs.set('workers/T1/logs/stdout.log', 'fresh\n')

    const frames = await collect(
      { cursor: { offset: 99, stream: 'stdout' }, maxBytes: 100, runDir: '/runs/run-a', taskId: 'T1' },
      deps([artifacts({ results: [result('T1', { stdout: 'workers/T1/logs/stdout.log' })] })], logs),
    )

    expect(frames[0]).toMatchObject({
      chunks: [{ byteCount: 6, offset: 0, stream: 'stdout', text: 'fresh\n' }],
      cursor: { offset: 6, stream: 'stdout' },
      rotated: true,
    })
  })

  it('emits deterministic follow frames and advances cursors on injected ticks', async () => {
    const logs = new MemoryLogReader()
    logs.set('workers/T1/logs/stdout.log', 'one\n')

    const frames = await collect(
      { follow: true, maxBytes: 100, runDir: '/runs/run-a', taskId: 'T1' },
      deps(
        [artifacts({ results: [result('T1', { stdout: 'workers/T1/logs/stdout.log' })] })],
        logs,
        ticker([
          () => {
            logs.set('workers/T1/logs/stdout.log', 'one\ntwo\n')
          },
          () => {
            logs.set('workers/T1/logs/stdout.log', 'one\ntwo\nthree\n')
          },
        ]),
      ),
    )

    expect(frames.map((frame) => frame.chunks)).toEqual([
      [{ byteCount: 4, offset: 0, stream: 'stdout', text: 'one\n' }],
      [{ byteCount: 4, offset: 4, stream: 'stdout', text: 'two\n' }],
      [{ byteCount: 6, offset: 8, stream: 'stdout', text: 'three\n' }],
    ])
    expect(frames.map((frame) => frame.cursor)).toEqual([
      { offset: 4, stream: 'stdout' },
      { offset: 8, stream: 'stdout' },
      { offset: 14, stream: 'stdout' },
    ])
    expect(logs.reads).toHaveLength(3)
  })
})
