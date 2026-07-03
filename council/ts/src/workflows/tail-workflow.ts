import type { RunStoreEvent, WorkerOutputStream } from '../contexts/runstore/index.js'
import type { LiveRunArtifacts, LiveRunDirReaderPort, WorkerResult } from '../ports/index.js'

import {
  selectTaskLogTail,
  type TailCursor,
  type TailFormattedChunk,
  type TailLogEventRange,
  type TailLogSource,
  type TailRequest,
} from './tail.js'

export type TailWorkflowLogPathSource = 'event' | 'result' | 'snapshot'
export type TailWorkflowMissingReason = 'log' | 'task'

export interface TailWorkflowInput {
  readonly cursor?: TailCursor
  readonly follow?: boolean
  readonly lines?: number
  readonly maxBytes: number
  readonly offset?: number
  readonly runDir: string
  readonly since?: string
  readonly stream?: WorkerOutputStream
  readonly taskId: string
}

export interface TailWorkflowLogStatInput {
  readonly path: string
  readonly runDir: string
}

export interface TailWorkflowLogReadInput extends TailWorkflowLogStatInput {
  readonly end: number
  readonly start: number
}

export interface TailWorkflowLogStat {
  readonly sizeBytes: number
}

export interface TailWorkflowLogReaderPort {
  stat(input: TailWorkflowLogStatInput): Promise<TailWorkflowLogStat | undefined>
  read(input: TailWorkflowLogReadInput): Promise<Uint8Array>
}

export interface TailWorkflowTicker {
  ticks(): AsyncIterable<void>
}

export interface TailWorkflowDeps {
  readonly artifacts: LiveRunDirReaderPort
  readonly logs: TailWorkflowLogReaderPort
  readonly ticker: TailWorkflowTicker
}

export interface TailWorkflowFrame {
  readonly chunks: readonly TailFormattedChunk[]
  readonly cursor: TailCursor
  readonly logPath?: string
  readonly logPathSource?: TailWorkflowLogPathSource
  readonly missing: boolean
  readonly missingReason?: TailWorkflowMissingReason
  readonly rotated: boolean
  readonly stream: WorkerOutputStream
  readonly taskId: string
  readonly truncated: boolean
}

interface WorkerResultLogMetadata extends WorkerResult {
  readonly stderr_log_path?: string
  readonly stdout_log_path?: string
}

interface ResolvedLogPath {
  readonly path: string
  readonly source: TailWorkflowLogPathSource
}

export async function* tailWorkflow(
  input: TailWorkflowInput,
  deps: TailWorkflowDeps,
): AsyncGenerator<TailWorkflowFrame> {
  const stream = input.stream ?? 'stdout'
  const first = await readTailFrame(input, deps, stream, input.cursor, input.offset)
  yield first

  if (input.follow !== true) return

  const ticks = deps.ticker.ticks()[Symbol.asyncIterator]()
  let tick = await ticks.next()
  let cursor = first.cursor
  while (!tick.done) {
    const frame = await readTailFrame(input, deps, stream, cursor, undefined)
    yield frame
    cursor = frame.cursor
    tick = await ticks.next()
  }
}

async function readTailFrame(
  input: TailWorkflowInput,
  deps: TailWorkflowDeps,
  stream: WorkerOutputStream,
  cursor: TailCursor | undefined,
  offset: number | undefined,
): Promise<TailWorkflowFrame> {
  const artifacts = await deps.artifacts.readRunDir(input.runDir)
  const request = tailRequest(input, stream, cursor, offset)

  if (!hasTask(artifacts, input.taskId)) {
    return frameFromSelection(input.taskId, selectTaskLogTail({ request, sources: [] }), stream, 'task')
  }

  const resolved = resolveLogPath(artifacts, input.taskId, stream)
  if (resolved === undefined) {
    return frameFromSelection(input.taskId, selectTaskLogTail({ request, sources: [] }), stream, 'log')
  }

  const stat = await deps.logs.stat({ path: resolved.path, runDir: input.runDir })
  if (stat === undefined) {
    return frameFromSelection(input.taskId, selectTaskLogTail({ request, sources: [] }), stream, 'log', resolved)
  }

  const maxBytes = Math.max(0, input.maxBytes)
  const sizeBytes = Math.max(0, stat.sizeBytes)
  const start = Math.max(0, sizeBytes - maxBytes)
  const buffer = await deps.logs.read({
    end: sizeBytes,
    path: resolved.path,
    runDir: input.runDir,
    start,
  })
  const source: TailLogSource = {
    buffer,
    bufferStartOffset: start,
    events: logEvents(artifacts.events, input.taskId, stream),
    sizeBytes,
    stream,
  }

  return frameFromSelection(
    input.taskId,
    selectTaskLogTail({ request, sources: [source] }),
    stream,
    undefined,
    resolved,
  )
}

function tailRequest(
  input: TailWorkflowInput,
  stream: WorkerOutputStream,
  cursor: TailCursor | undefined,
  offset: number | undefined,
): TailRequest {
  return {
    maxBytes: Math.max(0, input.maxBytes),
    stream,
    ...(cursor === undefined ? {} : { cursor }),
    ...(input.lines === undefined ? {} : { lines: input.lines }),
    ...(offset === undefined ? {} : { offset }),
    ...(input.since === undefined ? {} : { since: input.since }),
  }
}

function hasTask(artifacts: LiveRunArtifacts, taskId: string): boolean {
  return artifacts.normalized.tasks.some((task) => task.id === taskId)
}

function resolveLogPath(
  artifacts: LiveRunArtifacts,
  taskId: string,
  stream: WorkerOutputStream,
): ResolvedLogPath | undefined {
  const resultPath = resultLogPath(artifacts.workerResults.get(taskId), stream)
  if (resultPath !== undefined) return { path: resultPath, source: 'result' }

  const snapshotPath = snapshotLogPath(artifacts, taskId, stream)
  if (snapshotPath !== undefined) return { path: snapshotPath, source: 'snapshot' }

  return eventLogPath(artifacts.events, taskId, stream)
}

function resultLogPath(result: WorkerResult | undefined, stream: WorkerOutputStream): string | undefined {
  if (result === undefined) return undefined
  const metadata: WorkerResultLogMetadata = result
  return stream === 'stdout' ? metadata.stdout_log_path : metadata.stderr_log_path
}

function snapshotLogPath(
  artifacts: LiveRunArtifacts,
  taskId: string,
  stream: WorkerOutputStream,
): string | undefined {
  const snapshot = artifacts.workerSupervisorSnapshots.get(taskId)
  if (snapshot === undefined) return undefined
  return stream === 'stdout' ? snapshot.logs.stdout : snapshot.logs.stderr
}

function eventLogPath(
  events: readonly RunStoreEvent[],
  taskId: string,
  stream: WorkerOutputStream,
): ResolvedLogPath | undefined {
  for (const event of [...events].reverse()) {
    if (event.type !== 'worker_output') continue
    if (event.payload.task_id !== taskId || event.payload.stream !== stream) continue
    if (event.payload.log_path !== undefined) return { path: event.payload.log_path, source: 'event' }
  }
  return undefined
}

function logEvents(
  events: readonly RunStoreEvent[],
  taskId: string,
  stream: WorkerOutputStream,
): readonly TailLogEventRange[] {
  return events.flatMap((event) => {
    if (event.type !== 'worker_output') return []
    if (event.payload.task_id !== taskId || event.payload.stream !== stream) return []
    return [{
      byteCount: event.payload.byte_count,
      occurredAt: event.payload.observed_at ?? '',
      offset: event.payload.offset,
      stream,
    }]
  })
}

function frameFromSelection(
  taskId: string,
  selection: ReturnType<typeof selectTaskLogTail>,
  stream: WorkerOutputStream,
  missingReason?: TailWorkflowMissingReason,
  resolved?: ResolvedLogPath,
): TailWorkflowFrame {
  return {
    chunks: selection.chunks,
    cursor: selection.nextCursor,
    missing: selection.missing,
    rotated: selection.rotated,
    stream,
    taskId,
    truncated: selection.truncated,
    ...(missingReason === undefined ? {} : { missingReason }),
    ...(resolved === undefined ? {} : { logPath: resolved.path, logPathSource: resolved.source }),
  }
}
