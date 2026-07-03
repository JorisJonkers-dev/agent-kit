import type {
  Amendment,
  EngineDef,
  ReviewVerdict,
  RoutingVerdict,
  RunState,
  Task,
} from '../../../shared-kernel/index.js'

export const RUNSTORE_STATE_FILE = 'state.json'
export const RUNSTORE_TASKS_FILE = 'tasks.json'
export const RUNSTORE_EVENTS_FILE = 'events.jsonl'
export const RUNSTORE_EVENTS_LOCK_FILE = 'events.jsonl.lock'

export type AtomicJsonTarget = typeof RUNSTORE_STATE_FILE | typeof RUNSTORE_TASKS_FILE

export type AtomicJsonValue<TTarget extends AtomicJsonTarget> = TTarget extends typeof RUNSTORE_STATE_FILE
  ? RunState
  : readonly Task[]

export type WorkerOutputStream = 'stdout' | 'stderr'

export interface WorkerStartedPayload {
  readonly worker_id: string
  readonly task_id?: string
  readonly attempt?: number
  readonly pid?: number
  readonly command?: readonly string[]
  readonly cwd?: string
  readonly started_at?: string
  readonly engine?: EngineDef
  readonly model_tier?: string
  readonly content_hash?: string
}

export interface WorkerOutputPayload {
  readonly worker_id: string
  readonly task_id?: string
  readonly stream: WorkerOutputStream
  readonly offset: number
  readonly byte_count: number
  readonly tail?: string
  readonly tail_bytes?: number
  readonly log_path?: string
  readonly observed_at?: string
  readonly sha256?: string
  readonly content_hash?: string
}

export interface WorkerDetectedPayload {
  readonly worker_id: string
  readonly task_id?: string
  readonly pid?: number
  readonly status?: string
  readonly detected_at?: string
  readonly content_hash?: string
}

export interface WorkerRestartedPayload {
  readonly worker_id: string
  readonly task_id?: string
  readonly attempt: number
  readonly previous_pid?: number
  readonly pid?: number
  readonly reason?: string
  readonly restarted_at?: string
  readonly content_hash?: string
}

export interface WorkerExitedPayload {
  readonly worker_id: string
  readonly task_id?: string
  readonly pid?: number
  readonly exit_code: number | null
  readonly signal?: string | null
  readonly duration_ms?: number
  readonly exited_at?: string
  readonly content_hash?: string
}

export interface WorkerFinishedPayload {
  readonly worker_id: string
  readonly task_id: string
  readonly status: string
  readonly result_path?: string
  readonly duration_ms?: number
  readonly finished_at?: string
  readonly content_hash?: string
}

export type WorkerLifecycleEvent =
  | {
      readonly type: 'worker_started'
      readonly payload: WorkerStartedPayload
    }
  | {
      readonly type: 'worker_output'
      readonly payload: WorkerOutputPayload
    }
  | {
      readonly type: 'worker_detected'
      readonly payload: WorkerDetectedPayload
    }
  | {
      readonly type: 'worker_restarted'
      readonly payload: WorkerRestartedPayload
    }
  | {
      readonly type: 'worker_exited'
      readonly payload: WorkerExitedPayload
    }
  | {
      readonly type: 'worker_finished'
      readonly payload: WorkerFinishedPayload
    }

export type RunStoreEvent =
  | {
      readonly type: 'review_verdict'
      readonly payload: ReviewVerdict
    }
  | {
      readonly type: 'routing_verdict'
      readonly payload: RoutingVerdict
    }
  | {
      readonly type: 'amendment'
      readonly payload: Amendment
    }
  | WorkerLifecycleEvent

export type AtomicJsonWriteStep =
  | {
      readonly kind: 'write-temp-file'
      readonly path: string
      readonly bytes: string
    }
  | {
      readonly kind: 'sync-file'
      readonly path: string
    }
  | {
      readonly kind: 'rename-file'
      readonly fromPath: string
      readonly toPath: string
    }
  | {
      readonly kind: 'sync-directory'
      readonly path: string
    }

export interface AtomicJsonWritePlan<TTarget extends AtomicJsonTarget = AtomicJsonTarget> {
  readonly kind: 'atomic-json-write'
  readonly runId: string
  readonly target: TTarget
  readonly finalPath: string
  readonly tempPath: string
  readonly bytes: string
  readonly steps: readonly AtomicJsonWriteStep[]
}

export type EventAppendStep =
  | {
      readonly kind: 'acquire-lock'
      readonly path: string
    }
  | {
      readonly kind: 'append-file'
      readonly path: string
      readonly bytes: string
    }
  | {
      readonly kind: 'sync-file'
      readonly path: string
    }
  | {
      readonly kind: 'release-lock'
      readonly path: string
    }

export interface EventAppendPlan {
  readonly kind: 'locked-event-append'
  readonly runId: string
  readonly eventPath: string
  readonly lockPath: string
  readonly events: readonly RunStoreEvent[]
  readonly bytes: string
  readonly steps: readonly EventAppendStep[]
}

export interface PlanAtomicJsonWriteInput<TTarget extends AtomicJsonTarget> {
  readonly runId: string
  readonly target: TTarget
  readonly tempId: string
  readonly value: AtomicJsonValue<TTarget>
}

export interface PlanEventsAppendInput {
  readonly runId: string
  readonly events: readonly RunStoreEvent[]
}

export function reviewVerdictEvent(payload: ReviewVerdict): RunStoreEvent {
  return {
    type: 'review_verdict',
    payload,
  }
}

export function routingVerdictEvent(payload: RoutingVerdict): RunStoreEvent {
  return {
    type: 'routing_verdict',
    payload,
  }
}

export function amendmentEvent(payload: Amendment): RunStoreEvent {
  return {
    type: 'amendment',
    payload,
  }
}

export function workerStartedEvent(payload: WorkerStartedPayload): WorkerLifecycleEvent {
  return {
    type: 'worker_started',
    payload,
  }
}

export function workerOutputEvent(payload: WorkerOutputPayload): WorkerLifecycleEvent {
  return {
    type: 'worker_output',
    payload,
  }
}

export function workerDetectedEvent(payload: WorkerDetectedPayload): WorkerLifecycleEvent {
  return {
    type: 'worker_detected',
    payload,
  }
}

export function workerRestartedEvent(payload: WorkerRestartedPayload): WorkerLifecycleEvent {
  return {
    type: 'worker_restarted',
    payload,
  }
}

export function workerExitedEvent(payload: WorkerExitedPayload): WorkerLifecycleEvent {
  return {
    type: 'worker_exited',
    payload,
  }
}

export function workerFinishedEvent(payload: WorkerFinishedPayload): WorkerLifecycleEvent {
  return {
    type: 'worker_finished',
    payload,
  }
}

export function planStateWrite(
  runId: string,
  state: RunState,
  tempId: string,
): AtomicJsonWritePlan<typeof RUNSTORE_STATE_FILE> {
  return planAtomicJsonWrite({
    runId,
    target: RUNSTORE_STATE_FILE,
    tempId,
    value: state,
  })
}

export function planTasksWrite(
  runId: string,
  tasks: readonly Task[],
  tempId: string,
): AtomicJsonWritePlan<typeof RUNSTORE_TASKS_FILE> {
  return planAtomicJsonWrite({
    runId,
    target: RUNSTORE_TASKS_FILE,
    tempId,
    value: tasks,
  })
}

export function planAtomicJsonWrite<TTarget extends AtomicJsonTarget>(
  input: PlanAtomicJsonWriteInput<TTarget>,
): AtomicJsonWritePlan<TTarget> {
  assertPathSegment('runId', input.runId)
  assertPathSegment('tempId', input.tempId)

  const finalPath = runStorePath(input.runId, input.target)
  const tempPath = runStorePath(input.runId, `.${input.target}.${input.tempId}.tmp`)
  const bytes = serializeJson(input.value, 2)

  return {
    kind: 'atomic-json-write',
    runId: input.runId,
    target: input.target,
    finalPath,
    tempPath,
    bytes,
    steps: [
      {
        kind: 'write-temp-file',
        path: tempPath,
        bytes,
      },
      {
        kind: 'sync-file',
        path: tempPath,
      },
      {
        kind: 'rename-file',
        fromPath: tempPath,
        toPath: finalPath,
      },
      {
        kind: 'sync-directory',
        path: input.runId,
      },
    ],
  }
}

export function planEventAppend(runId: string, event: RunStoreEvent): EventAppendPlan {
  return planEventsAppend({
    runId,
    events: [event],
  })
}

export function planEventsAppend(input: PlanEventsAppendInput): EventAppendPlan {
  assertPathSegment('runId', input.runId)

  if (input.events.length === 0) {
    throw new Error('events must not be empty')
  }

  const eventPath = runStorePath(input.runId, RUNSTORE_EVENTS_FILE)
  const lockPath = runStorePath(input.runId, RUNSTORE_EVENTS_LOCK_FILE)
  const bytes = input.events.map((event) => serializeJson(event, 0)).join('')

  return {
    kind: 'locked-event-append',
    runId: input.runId,
    eventPath,
    lockPath,
    events: input.events,
    bytes,
    steps: [
      {
        kind: 'acquire-lock',
        path: lockPath,
      },
      {
        kind: 'append-file',
        path: eventPath,
        bytes,
      },
      {
        kind: 'sync-file',
        path: eventPath,
      },
      {
        kind: 'release-lock',
        path: lockPath,
      },
    ],
  }
}

function runStorePath(runId: string, fileName: string): string {
  return `${runId}/${fileName}`
}

function serializeJson(value: unknown, space: 0 | 2): string {
  const serialized: unknown = JSON.stringify(value, null, space)

  if (typeof serialized !== 'string') {
    throw new Error('runstore values must be JSON serializable')
  }

  return `${serialized}\n`
}

function assertPathSegment(label: string, value: string): void {
  if (value.length === 0) {
    throw new Error(`${label} must not be empty`)
  }

  if (value.includes('/') || value.includes('\\') || value.includes('\0')) {
    throw new Error(`${label} must be a single path segment`)
  }
}
