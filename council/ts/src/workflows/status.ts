import { planWaves } from '../contexts/graph/index.js'
import type { RunStoreEvent } from '../contexts/runstore/index.js'
import type { LegacyRunNormalizerPort, LegacyRunReport, WorkerResult } from '../ports/index.js'
import type { RunState, Task, TaskId } from '../shared-kernel/index.js'

export interface RunSummary {
  readonly report?: LegacyRunReport
  readonly run: string
  readonly state: RunState
  readonly tasks: readonly Task[]
  readonly waves: readonly (readonly string[])[]
  readonly workerResults: readonly WorkerResult[]
}

export type RunViewTaskState =
  | 'blocked'
  | 'budget-cap'
  | 'dead-snapshot'
  | 'detected'
  | 'disk-cap'
  | 'exited'
  | 'failed'
  | 'pending'
  | 'restarting'
  | 'running'
  | 'stale-snapshot'
  | 'stalled'
  | 'stopped'
  | 'succeeded'

export type RunViewSupervisorSnapshotStatus =
  | 'budget-cap'
  | 'completed'
  | 'detected'
  | 'disk-cap'
  | 'exited'
  | 'failed'
  | 'restarting'
  | 'running'
  | 'stalled'
  | 'stopped'

export interface RunViewSupervisorSnapshot {
  readonly task_id: string
  readonly attempt_id: number
  readonly restart_count: number
  readonly model_tier?: string
  readonly pid?: number
  readonly status: RunViewSupervisorSnapshotStatus
  readonly watchdog?: {
    readonly pending_detection?: {
      readonly kind?: unknown
    }
  }
}

export interface RunViewClock {
  now(): Date
}

export interface ProjectRunViewInput {
  readonly clock: RunViewClock
  readonly events?: readonly RunStoreEvent[]
  readonly summary: RunSummary
  readonly supervisorSnapshots?: readonly RunViewSupervisorSnapshot[]
}

export interface RunTaskView {
  readonly attempt: number
  readonly blockedBy: readonly TaskId[]
  readonly dependenciesSatisfied: boolean
  readonly durationMs: number
  readonly lastDetection: string | null
  readonly modelTier: string | null
  readonly pid: number | null
  readonly restarts: number
  readonly startedAt: string | null
  readonly state: RunViewTaskState
  readonly taskId: TaskId
  readonly terminalStatus: string | null
  readonly title: string
  readonly updatedAt: string | null
  readonly wave: number | null
  readonly workerId: string | null
}

export interface RunViewRollup {
  readonly countsByState: Readonly<Record<string, number>>
  readonly criticalPath: readonly TaskId[]
  readonly elapsedMs: number
  readonly readySet: readonly TaskId[]
  readonly startedAt: string | null
  readonly updatedAt: string | null
}

export interface RunView {
  readonly rollup: RunViewRollup
  readonly run: string
  readonly state: RunState
  readonly tasks: readonly RunTaskView[]
  readonly waves: readonly (readonly string[])[]
}

export async function statusWorkflow(
  input: { readonly runDir: string },
  deps: LegacyRunNormalizerPort,
): Promise<RunSummary> {
  const normalized = await deps.normalizeRunDir(input.runDir)
  return {
    ...(normalized.report ? { report: normalized.report } : {}),
    run: normalized.runId,
    state: normalized.state,
    tasks: normalized.tasks,
    waves: normalized.report?.waves ?? planWaves(normalized.tasks),
    workerResults: [...normalized.workerResults.values()],
  }
}

type EventField =
  | 'attempt'
  | 'duration'
  | 'lastDetection'
  | 'modelTier'
  | 'pid'
  | 'restarts'
  | 'startedAt'
  | 'state'
  | 'updatedAt'
  | 'workerId'

interface MutableTaskProjection {
  readonly eventFields: Set<EventField>
  readonly order: number
  readonly task: Task
  attempt: number
  durationMs: number | null
  lastDetection: string | null
  modelTier: string | null
  pid: number | null
  restarts: number
  startedAt: string | null
  state: RunViewTaskState
  terminalStatus: string | null
  updatedAt: string | null
  wave: number | null
  workerId: string | null
}

const SNAPSHOT_STATE: Readonly<Record<RunViewSupervisorSnapshotStatus, RunViewTaskState>> = {
  'budget-cap': 'budget-cap',
  completed: 'succeeded',
  detected: 'detected',
  'disk-cap': 'disk-cap',
  exited: 'exited',
  failed: 'failed',
  restarting: 'restarting',
  running: 'running',
  stalled: 'stalled',
  stopped: 'stopped',
}

const TERMINAL_WORKER_STATES: Readonly<Record<string, RunViewTaskState>> = {
  'budget-cap': 'budget-cap',
  completed: 'succeeded',
  'dead-snapshot': 'dead-snapshot',
  'disk-cap': 'disk-cap',
  failed: 'failed',
  'no-op': 'succeeded',
  ok: 'succeeded',
  passed: 'succeeded',
  stale: 'stale-snapshot',
  'stale-snapshot': 'stale-snapshot',
  stalled: 'stalled',
  stopped: 'stopped',
  succeeded: 'succeeded',
}

export function projectRunView(input: ProjectRunViewInput): RunView {
  const nowMs = input.clock.now().getTime()
  const waveByTaskId = indexWaves(input.summary.waves)
  const projections = input.summary.tasks.map((task, order) =>
    initialProjection(task, order, waveByTaskId.get(task.id) ?? null),
  )
  const byTaskId = new Map(projections.map((projection) => [projection.task.id, projection]))
  const workerTaskIds = new Map<string, TaskId>()

  for (const event of input.events ?? []) {
    applyRunStoreEvent(event, byTaskId, workerTaskIds)
  }
  applyWorkerResults(input.summary.workerResults, byTaskId)
  applySupervisorSnapshots(input.supervisorSnapshots ?? [], byTaskId)

  const tasks = projections.map((projection) => taskViewFor(projection, byTaskId, nowMs))
  const rollup = rollupFor(tasks, projections, nowMs)
  return {
    rollup,
    run: input.summary.run,
    state: input.summary.state,
    tasks,
    waves: input.summary.waves.map((wave) => [...wave]),
  }
}

function initialProjection(task: Task, order: number, wave: number | null): MutableTaskProjection {
  return {
    attempt: 0,
    durationMs: null,
    eventFields: new Set<EventField>(),
    lastDetection: null,
    modelTier: null,
    order,
    pid: null,
    restarts: 0,
    startedAt: null,
    state: 'pending',
    task,
    terminalStatus: null,
    updatedAt: null,
    wave,
    workerId: null,
  }
}

function indexWaves(waves: readonly (readonly string[])[]): ReadonlyMap<string, number> {
  const index = new Map<string, number>()
  waves.forEach((wave, waveIndex) => {
    wave.forEach((taskId) => {
      if (!index.has(taskId)) {
        index.set(taskId, waveIndex)
      }
    })
  })
  return index
}

function applyRunStoreEvent(
  event: RunStoreEvent,
  byTaskId: ReadonlyMap<TaskId, MutableTaskProjection>,
  workerTaskIds: Map<string, TaskId>,
): void {
  if (event.type === 'worker_started') {
    const projection = projectionForEvent(event.payload.task_id, event.payload.worker_id, byTaskId, workerTaskIds)
    if (projection === undefined) return
    applyWorkerIdentity(projection, event.payload.worker_id)
    projection.state = 'running'
    projection.eventFields.add('state')
    if (event.payload.attempt !== undefined) {
      projection.attempt = event.payload.attempt
      projection.restarts = Math.max(0, event.payload.attempt - 1)
      projection.eventFields.add('attempt')
      projection.eventFields.add('restarts')
    }
    if (event.payload.model_tier !== undefined) {
      projection.modelTier = event.payload.model_tier
      projection.eventFields.add('modelTier')
    }
    applyPid(projection, event.payload.pid)
    applyStartedAt(projection, event.payload.started_at)
    return
  }

  if (event.type === 'worker_output') {
    const projection = projectionForEvent(event.payload.task_id, event.payload.worker_id, byTaskId, workerTaskIds)
    if (projection === undefined) return
    applyWorkerIdentity(projection, event.payload.worker_id)
    projection.state = 'running'
    projection.eventFields.add('state')
    return
  }

  if (event.type === 'worker_detected') {
    const projection = projectionForEvent(event.payload.task_id, event.payload.worker_id, byTaskId, workerTaskIds)
    if (projection === undefined) return
    applyWorkerIdentity(projection, event.payload.worker_id)
    projection.state = 'detected'
    projection.eventFields.add('state')
    applyPid(projection, event.payload.pid)
    applyDetection(projection, event.payload.status)
    applyUpdatedAt(projection, event.payload.detected_at)
    return
  }

  if (event.type === 'worker_restarted') {
    const projection = projectionForEvent(event.payload.task_id, event.payload.worker_id, byTaskId, workerTaskIds)
    if (projection === undefined) return
    applyWorkerIdentity(projection, event.payload.worker_id)
    projection.attempt = event.payload.attempt
    projection.restarts = Math.max(0, event.payload.attempt - 1)
    projection.state = 'restarting'
    projection.eventFields.add('attempt')
    projection.eventFields.add('restarts')
    projection.eventFields.add('state')
    applyPid(projection, event.payload.pid)
    applyDetection(projection, event.payload.reason)
    applyUpdatedAt(projection, event.payload.restarted_at)
    return
  }

  if (event.type === 'worker_exited') {
    const projection = projectionForEvent(event.payload.task_id, event.payload.worker_id, byTaskId, workerTaskIds)
    if (projection === undefined) return
    applyWorkerIdentity(projection, event.payload.worker_id)
    projection.state = 'exited'
    projection.eventFields.add('state')
    applyPid(projection, event.payload.pid)
    applyDuration(projection, event.payload.duration_ms)
    applyUpdatedAt(projection, event.payload.exited_at)
    return
  }

  if (event.type === 'worker_finished') {
    const projection = byTaskId.get(event.payload.task_id as TaskId)
    if (projection === undefined) return
    applyWorkerIdentity(projection, event.payload.worker_id)
    projection.state = stateFromWorkerStatus(event.payload.status)
    projection.terminalStatus = event.payload.status
    projection.eventFields.add('state')
    applyDuration(projection, event.payload.duration_ms)
    applyUpdatedAt(projection, event.payload.finished_at)
  }
}

function projectionForEvent(
  taskId: string | undefined,
  workerId: string,
  byTaskId: ReadonlyMap<TaskId, MutableTaskProjection>,
  workerTaskIds: Map<string, TaskId>,
): MutableTaskProjection | undefined {
  const knownTaskId = taskId as TaskId | undefined
  if (knownTaskId !== undefined && byTaskId.has(knownTaskId)) {
    workerTaskIds.set(workerId, knownTaskId)
    return byTaskId.get(knownTaskId)
  }
  const mappedTaskId = workerTaskIds.get(workerId)
  return mappedTaskId === undefined ? undefined : byTaskId.get(mappedTaskId)
}

function applyWorkerIdentity(projection: MutableTaskProjection, workerId: string): void {
  projection.workerId = workerId
  projection.eventFields.add('workerId')
}

function applyPid(projection: MutableTaskProjection, pid: number | undefined): void {
  if (pid !== undefined) {
    projection.pid = pid
    projection.eventFields.add('pid')
  }
}

function applyStartedAt(projection: MutableTaskProjection, startedAt: string | undefined): void {
  if (startedAt !== undefined) {
    projection.startedAt = projection.startedAt ?? startedAt
    projection.updatedAt = startedAt
    projection.eventFields.add('startedAt')
    projection.eventFields.add('updatedAt')
  }
}

function applyUpdatedAt(projection: MutableTaskProjection, updatedAt: string | undefined): void {
  if (updatedAt !== undefined) {
    projection.updatedAt = updatedAt
    projection.eventFields.add('updatedAt')
  }
}

function applyDetection(projection: MutableTaskProjection, detection: string | undefined): void {
  if (detection !== undefined) {
    projection.lastDetection = detection
    projection.eventFields.add('lastDetection')
  }
}

function applyDuration(projection: MutableTaskProjection, durationMs: number | undefined): void {
  if (durationMs !== undefined) {
    projection.durationMs = durationMs
    projection.eventFields.add('duration')
  }
}

function applyWorkerResults(
  workerResults: readonly WorkerResult[],
  byTaskId: ReadonlyMap<TaskId, MutableTaskProjection>,
): void {
  workerResults.forEach((result) => {
    const projection = byTaskId.get(result.task_id as TaskId)
    if (projection === undefined) return
    projection.state = stateFromWorkerStatus(result.status)
    projection.terminalStatus = result.status
    if (!projection.eventFields.has('modelTier') && result.model_tier !== undefined) {
      projection.modelTier = result.model_tier
    }
  })
}

function applySupervisorSnapshots(
  snapshots: readonly RunViewSupervisorSnapshot[],
  byTaskId: ReadonlyMap<TaskId, MutableTaskProjection>,
): void {
  snapshots.forEach((snapshot) => {
    const projection = byTaskId.get(snapshot.task_id as TaskId)
    if (projection === undefined) return
    if (!projection.eventFields.has('state') && projection.terminalStatus === null) {
      projection.state = SNAPSHOT_STATE[snapshot.status]
    }
    if (!projection.eventFields.has('attempt')) {
      projection.attempt = snapshot.attempt_id
    }
    if (!projection.eventFields.has('restarts')) {
      projection.restarts = snapshot.restart_count
    }
    if (!projection.eventFields.has('modelTier') && projection.modelTier === null) {
      projection.modelTier = snapshot.model_tier ?? null
    }
    if (!projection.eventFields.has('pid') && projection.pid === null) {
      projection.pid = snapshot.pid ?? null
    }
    if (!projection.eventFields.has('lastDetection') && projection.lastDetection === null) {
      projection.lastDetection = detectionKind(snapshot.watchdog?.pending_detection)
    }
  })
}

function detectionKind(detection: { readonly kind?: unknown } | undefined): string | null {
  return typeof detection?.kind === 'string' ? detection.kind : null
}

function stateFromWorkerStatus(status: string): RunViewTaskState {
  return TERMINAL_WORKER_STATES[status] ?? 'failed'
}

function taskViewFor(
  projection: MutableTaskProjection,
  byTaskId: ReadonlyMap<TaskId, MutableTaskProjection>,
  nowMs: number,
): RunTaskView {
  const blockedBy = projection.task.depends_on.filter(
    (dependency) => byTaskId.get(dependency)?.state !== 'succeeded',
  )
  const dependenciesSatisfied = blockedBy.length === 0
  const state = projection.state === 'pending' && !dependenciesSatisfied ? 'blocked' : projection.state
  return {
    attempt: projection.attempt,
    blockedBy,
    dependenciesSatisfied,
    durationMs: durationMsFor(projection, state, nowMs),
    lastDetection: projection.lastDetection,
    modelTier: projection.modelTier,
    pid: projection.pid,
    restarts: projection.restarts,
    startedAt: projection.startedAt,
    state,
    taskId: projection.task.id,
    terminalStatus: projection.terminalStatus,
    title: projection.task.title,
    updatedAt: projection.updatedAt,
    wave: projection.wave,
    workerId: projection.workerId,
  }
}

function durationMsFor(
  projection: MutableTaskProjection,
  state: RunViewTaskState,
  nowMs: number,
): number {
  if (projection.durationMs !== null) {
    return projection.durationMs
  }
  const startedAtMs = timestampMs(projection.startedAt)
  if (startedAtMs === null) {
    return 0
  }
  const updatedAtMs = isActiveState(state) ? nowMs : timestampMs(projection.updatedAt) ?? nowMs
  return Math.max(0, updatedAtMs - startedAtMs)
}

function isActiveState(state: RunViewTaskState): boolean {
  return state === 'detected' || state === 'restarting' || state === 'running'
}

function rollupFor(
  tasks: readonly RunTaskView[],
  projections: readonly MutableTaskProjection[],
  nowMs: number,
): RunViewRollup {
  const startedAt = earliestTimestamp(tasks.map((task) => task.startedAt))
  const updatedAt = latestTimestamp(tasks.map((task) => task.updatedAt))
  return {
    countsByState: countsByState(tasks),
    criticalPath: criticalPath(tasks, projections),
    elapsedMs: elapsedMsFor(tasks, startedAt, updatedAt, nowMs),
    readySet: readySet(tasks, projections),
    startedAt,
    updatedAt,
  }
}

function countsByState(tasks: readonly RunTaskView[]): Readonly<Record<string, number>> {
  return tasks.reduce<Record<string, number>>((counts, task) => {
    counts[task.state] = (counts[task.state] ?? 0) + 1
    return counts
  }, {})
}

function readySet(
  tasks: readonly RunTaskView[],
  projections: readonly MutableTaskProjection[],
): readonly TaskId[] {
  const viewByTaskId = new Map(tasks.map((task) => [task.taskId, task]))
  const lengths = remainingPathLengths(viewByTaskId, projections)
  return tasks
    .filter((task) => task.state === 'pending' && task.dependenciesSatisfied)
    .sort((left, right) => comparePathPriority(left.taskId, right.taskId, lengths, projections))
    .map((task) => task.taskId)
}

function criticalPath(
  tasks: readonly RunTaskView[],
  projections: readonly MutableTaskProjection[],
): readonly TaskId[] {
  const viewByTaskId = new Map(tasks.map((task) => [task.taskId, task]))
  const lengths = remainingPathLengths(viewByTaskId, projections)
  const candidates = tasks
    .filter((task) => task.state !== 'succeeded')
    .filter((task) => nonSucceededDependencies(task, viewByTaskId).length === 0)
  const start = [...candidates].sort((left, right) =>
    comparePathPriority(left.taskId, right.taskId, lengths, projections),
  )[0]
  return start === undefined ? [] : pathFrom(start.taskId, viewByTaskId, lengths, projections)
}

function pathFrom(
  taskId: TaskId,
  viewByTaskId: ReadonlyMap<TaskId, RunTaskView>,
  lengths: ReadonlyMap<TaskId, number>,
  projections: readonly MutableTaskProjection[],
): readonly TaskId[] {
  const dependent = dependentsOf(taskId, projections)
    .filter((dependentId) => viewByTaskId.get(dependentId)?.state !== 'succeeded')
    .sort((left, right) => comparePathPriority(left, right, lengths, projections))[0]
  return dependent === undefined ? [taskId] : [taskId, ...pathFrom(dependent, viewByTaskId, lengths, projections)]
}

function remainingPathLengths(
  viewByTaskId: ReadonlyMap<TaskId, RunTaskView>,
  projections: readonly MutableTaskProjection[],
): ReadonlyMap<TaskId, number> {
  const lengths = new Map<TaskId, number>()
  projections.forEach((projection) => {
    lengths.set(projection.task.id, remainingPathLength(projection.task.id, viewByTaskId, projections, lengths))
  })
  return lengths
}

function remainingPathLength(
  taskId: TaskId,
  viewByTaskId: ReadonlyMap<TaskId, RunTaskView>,
  projections: readonly MutableTaskProjection[],
  lengths: Map<TaskId, number>,
): number {
  const cached = lengths.get(taskId)
  if (cached !== undefined) {
    return cached
  }
  const openDependents = dependentsOf(taskId, projections).filter(
    (dependentId) => viewByTaskId.get(dependentId)?.state !== 'succeeded',
  )
  const length =
    1 +
    openDependents.reduce(
      (longest, dependentId) =>
        Math.max(longest, remainingPathLength(dependentId, viewByTaskId, projections, lengths)),
      0,
    )
  lengths.set(taskId, length)
  return length
}

function dependentsOf(taskId: TaskId, projections: readonly MutableTaskProjection[]): readonly TaskId[] {
  return projections
    .filter((projection) => projection.task.depends_on.includes(taskId))
    .map((projection) => projection.task.id)
}

function nonSucceededDependencies(
  task: RunTaskView,
  viewByTaskId: ReadonlyMap<TaskId, RunTaskView>,
): readonly TaskId[] {
  return task.blockedBy.filter((dependency) => viewByTaskId.has(dependency))
}

function comparePathPriority(
  left: TaskId,
  right: TaskId,
  lengths: ReadonlyMap<TaskId, number>,
  projections: readonly MutableTaskProjection[],
): number {
  const lengthDelta = (lengths.get(right) ?? 0) - (lengths.get(left) ?? 0)
  if (lengthDelta !== 0) {
    return lengthDelta
  }
  const orderDelta = orderFor(left, projections) - orderFor(right, projections)
  return orderDelta === 0 ? left.localeCompare(right) : orderDelta
}

function orderFor(taskId: TaskId, projections: readonly MutableTaskProjection[]): number {
  return projections.find((projection) => projection.task.id === taskId)?.order ?? Number.MAX_SAFE_INTEGER
}

function elapsedMsFor(
  tasks: readonly RunTaskView[],
  startedAt: string | null,
  updatedAt: string | null,
  nowMs: number,
): number {
  const startedAtMs = timestampMs(startedAt)
  if (startedAtMs === null) {
    return 0
  }
  const endAtMs = tasks.some((task) => isActiveState(task.state))
    ? nowMs
    : timestampMs(updatedAt) ?? nowMs
  return Math.max(0, endAtMs - startedAtMs)
}

function earliestTimestamp(values: readonly (string | null)[]): string | null {
  return sortedTimestamps(values)[0] ?? null
}

function latestTimestamp(values: readonly (string | null)[]): string | null {
  return sortedTimestamps(values).at(-1) ?? null
}

function sortedTimestamps(values: readonly (string | null)[]): readonly string[] {
  return values
    .filter((value): value is string => value !== null)
    .sort((left, right) => (timestampMs(left) ?? 0) - (timestampMs(right) ?? 0))
}

function timestampMs(value: string | null): number | null {
  if (value === null) {
    return null
  }
  const parsed = Date.parse(value)
  return Number.isFinite(parsed) ? parsed : null
}
