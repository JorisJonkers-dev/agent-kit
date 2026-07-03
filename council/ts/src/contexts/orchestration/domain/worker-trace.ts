import type {
  WorkerLifecycleEvent,
  WorkerOutputStream,
  WorkerStartedPayload,
} from '../../runstore/index.js'

export type WorkerTraceAttemptSource = 'worker_started' | 'worker_restarted'
export type WorkerTraceResultSource = 'worker_exited' | 'worker_finished'

export interface WorkerTraceAttemptEntry {
  readonly kind: 'attempt'
  readonly taskId: string
  readonly workerId: string
  readonly attempt: number
  readonly sourceEventType: WorkerTraceAttemptSource
  readonly pid?: number
  readonly previousPid?: number
  readonly command?: readonly string[]
  readonly cwd?: string
  readonly occurredAt?: string
  readonly engine?: NonNullable<WorkerStartedPayload['engine']>
  readonly modelTier?: string
  readonly reason?: string
  readonly contentHash?: string
}

export interface WorkerTraceOutputEntry {
  readonly kind: 'output'
  readonly taskId: string
  readonly workerId: string
  readonly attempt: number
  readonly stream: WorkerOutputStream
  readonly offset: number
  readonly byteCount: number
  readonly tail?: string
  readonly tailBytes?: number
  readonly logPath?: string
  readonly occurredAt?: string
  readonly sha256?: string
  readonly contentHash?: string
}

export interface WorkerTraceDetectionEntry {
  readonly kind: 'detection'
  readonly taskId: string
  readonly workerId: string
  readonly attempt: number
  readonly pid?: number
  readonly status?: string
  readonly occurredAt?: string
  readonly contentHash?: string
}

export interface WorkerTraceResultEntry {
  readonly kind: 'result'
  readonly taskId: string
  readonly workerId: string
  readonly attempt: number
  readonly sourceEventType: WorkerTraceResultSource
  readonly pid?: number
  readonly exitCode?: number | null
  readonly signal?: string | null
  readonly durationMs?: number
  readonly status?: string
  readonly resultPath?: string
  readonly occurredAt?: string
  readonly contentHash?: string
}

export type WorkerTraceEntry =
  | WorkerTraceAttemptEntry
  | WorkerTraceOutputEntry
  | WorkerTraceDetectionEntry
  | WorkerTraceResultEntry

interface WorkerTraceProjectionState {
  readonly currentAttemptByWorker: Map<string, number>
}

export function projectWorkerTrace(events: readonly WorkerLifecycleEvent[]): readonly WorkerTraceEntry[] {
  return appendWorkerTraceEvents([], events)
}

export function appendWorkerTraceEvents(
  existing: readonly WorkerTraceEntry[],
  events: readonly WorkerLifecycleEvent[],
): readonly WorkerTraceEntry[] {
  assertValidWorkerTrace(existing)
  const state = createProjectionState(existing)
  const additions = events.map((event, index) => projectWorkerTraceEvent(event, index, state))
  const next = [...existing, ...additions]
  validateWorkerTraceAppend(existing, next)
  return next
}

export function validateWorkerTraceAppend(
  previous: readonly WorkerTraceEntry[],
  next: readonly WorkerTraceEntry[],
): void {
  if (next.length < previous.length) {
    throw new Error('worker trace append removed prior entries')
  }
  for (const [index, entry] of previous.entries()) {
    if (JSON.stringify(entry) !== JSON.stringify(next[index])) {
      throw new Error(`worker trace append mutates prior entry at index ${String(index)}`)
    }
  }
  assertValidWorkerTrace(next)
}

export function assertValidWorkerTrace(entries: readonly WorkerTraceEntry[]): void {
  const outputOffsets = new Map<string, number>()
  const terminalEvents = new Set<string>()
  for (const [index, entry] of entries.entries()) {
    if (entry.taskId.trim().length === 0) {
      throw new Error(`worker trace entry at index ${String(index)} is missing taskId`)
    }
    if (entry.kind === 'output') {
      const key = outputKey(entry)
      const previousOffset = outputOffsets.get(key)
      if (previousOffset !== undefined && entry.offset < previousOffset) {
        throw new Error(
          `worker trace output offset decreased for ${entry.taskId} attempt ${String(entry.attempt)} ${entry.stream}`,
        )
      }
      outputOffsets.set(key, entry.offset)
    }
    if (entry.kind === 'result') {
      const key = terminalKey(entry)
      if (terminalEvents.has(key)) {
        throw new Error(
          `worker trace has duplicate ${entry.sourceEventType} terminal event for ${entry.taskId} attempt ${String(entry.attempt)}`,
        )
      }
      terminalEvents.add(key)
    }
  }
}

function createProjectionState(entries: readonly WorkerTraceEntry[]): WorkerTraceProjectionState {
  const currentAttemptByWorker = new Map<string, number>()
  for (const entry of entries) {
    currentAttemptByWorker.set(workerKey(entry.taskId, entry.workerId), entry.attempt)
  }
  return { currentAttemptByWorker }
}

function projectWorkerTraceEvent(
  event: WorkerLifecycleEvent,
  index: number,
  state: WorkerTraceProjectionState,
): WorkerTraceEntry {
  if (event.type === 'worker_started') {
    const taskId = requireTaskId(event.type, event.payload, index)
    const attempt = event.payload.attempt ?? 1
    state.currentAttemptByWorker.set(workerKey(taskId, event.payload.worker_id), attempt)
    return {
      attempt,
      kind: 'attempt',
      sourceEventType: event.type,
      taskId,
      workerId: event.payload.worker_id,
      ...(event.payload.pid === undefined ? {} : { pid: event.payload.pid }),
      ...(event.payload.command === undefined ? {} : { command: event.payload.command }),
      ...(event.payload.cwd === undefined ? {} : { cwd: event.payload.cwd }),
      ...(event.payload.started_at === undefined ? {} : { occurredAt: event.payload.started_at }),
      ...(event.payload.engine === undefined ? {} : { engine: event.payload.engine }),
      ...(event.payload.model_tier === undefined ? {} : { modelTier: event.payload.model_tier }),
      ...(event.payload.content_hash === undefined ? {} : { contentHash: event.payload.content_hash }),
    }
  }
  if (event.type === 'worker_restarted') {
    const taskId = requireTaskId(event.type, event.payload, index)
    state.currentAttemptByWorker.set(workerKey(taskId, event.payload.worker_id), event.payload.attempt)
    return {
      attempt: event.payload.attempt,
      kind: 'attempt',
      sourceEventType: event.type,
      taskId,
      workerId: event.payload.worker_id,
      ...(event.payload.previous_pid === undefined ? {} : { previousPid: event.payload.previous_pid }),
      ...(event.payload.pid === undefined ? {} : { pid: event.payload.pid }),
      ...(event.payload.reason === undefined ? {} : { reason: event.payload.reason }),
      ...(event.payload.restarted_at === undefined ? {} : { occurredAt: event.payload.restarted_at }),
      ...(event.payload.content_hash === undefined ? {} : { contentHash: event.payload.content_hash }),
    }
  }
  if (event.type === 'worker_output') {
    const taskId = requireTaskId(event.type, event.payload, index)
    const attempt = requireActiveAttempt(event.type, taskId, event.payload.worker_id, index, state)
    return {
      attempt,
      byteCount: event.payload.byte_count,
      kind: 'output',
      offset: event.payload.offset,
      stream: event.payload.stream,
      taskId,
      workerId: event.payload.worker_id,
      ...(event.payload.tail === undefined ? {} : { tail: event.payload.tail }),
      ...(event.payload.tail_bytes === undefined ? {} : { tailBytes: event.payload.tail_bytes }),
      ...(event.payload.log_path === undefined ? {} : { logPath: event.payload.log_path }),
      ...(event.payload.observed_at === undefined ? {} : { occurredAt: event.payload.observed_at }),
      ...(event.payload.sha256 === undefined ? {} : { sha256: event.payload.sha256 }),
      ...(event.payload.content_hash === undefined ? {} : { contentHash: event.payload.content_hash }),
    }
  }
  if (event.type === 'worker_detected') {
    const taskId = requireTaskId(event.type, event.payload, index)
    const attempt = requireActiveAttempt(event.type, taskId, event.payload.worker_id, index, state)
    return {
      attempt,
      kind: 'detection',
      taskId,
      workerId: event.payload.worker_id,
      ...(event.payload.pid === undefined ? {} : { pid: event.payload.pid }),
      ...(event.payload.status === undefined ? {} : { status: event.payload.status }),
      ...(event.payload.detected_at === undefined ? {} : { occurredAt: event.payload.detected_at }),
      ...(event.payload.content_hash === undefined ? {} : { contentHash: event.payload.content_hash }),
    }
  }
  if (event.type === 'worker_exited') {
    const taskId = requireTaskId(event.type, event.payload, index)
    const attempt = requireActiveAttempt(event.type, taskId, event.payload.worker_id, index, state)
    return {
      attempt,
      exitCode: event.payload.exit_code,
      kind: 'result',
      sourceEventType: event.type,
      taskId,
      workerId: event.payload.worker_id,
      ...(event.payload.pid === undefined ? {} : { pid: event.payload.pid }),
      ...(event.payload.signal === undefined ? {} : { signal: event.payload.signal }),
      ...(event.payload.duration_ms === undefined ? {} : { durationMs: event.payload.duration_ms }),
      ...(event.payload.exited_at === undefined ? {} : { occurredAt: event.payload.exited_at }),
      ...(event.payload.content_hash === undefined ? {} : { contentHash: event.payload.content_hash }),
    }
  }
  const taskId = requireTaskId(event.type, event.payload, index)
  const attempt = requireActiveAttempt(event.type, taskId, event.payload.worker_id, index, state)
  return {
    attempt,
    kind: 'result',
    sourceEventType: event.type,
    status: event.payload.status,
    taskId,
    workerId: event.payload.worker_id,
    ...(event.payload.result_path === undefined ? {} : { resultPath: event.payload.result_path }),
    ...(event.payload.duration_ms === undefined ? {} : { durationMs: event.payload.duration_ms }),
    ...(event.payload.finished_at === undefined ? {} : { occurredAt: event.payload.finished_at }),
    ...(event.payload.content_hash === undefined ? {} : { contentHash: event.payload.content_hash }),
  }
}

function requireTaskId(
  eventType: WorkerLifecycleEvent['type'],
  payload: { readonly task_id?: string },
  index: number,
): string {
  if (payload.task_id === undefined || payload.task_id.trim().length === 0) {
    throw new Error(`worker trace event ${eventType} at index ${String(index)} is missing task_id`)
  }
  return payload.task_id
}

function requireActiveAttempt(
  eventType: WorkerLifecycleEvent['type'],
  taskId: string,
  workerId: string,
  index: number,
  state: WorkerTraceProjectionState,
): number {
  const attempt = state.currentAttemptByWorker.get(workerKey(taskId, workerId))
  if (attempt === undefined) {
    throw new Error(`worker trace event ${eventType} at index ${String(index)} has no active attempt for ${taskId}`)
  }
  return attempt
}

function workerKey(taskId: string, workerId: string): string {
  return `${taskId}\u0000${workerId}`
}

function outputKey(entry: WorkerTraceOutputEntry): string {
  return `${workerKey(entry.taskId, entry.workerId)}\u0000${String(entry.attempt)}\u0000${entry.stream}`
}

function terminalKey(entry: WorkerTraceResultEntry): string {
  return `${workerKey(entry.taskId, entry.workerId)}\u0000${String(entry.attempt)}\u0000${entry.sourceEventType}`
}
