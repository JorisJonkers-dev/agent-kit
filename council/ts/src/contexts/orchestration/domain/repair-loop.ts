import type { Task, TaskId } from '../../../shared-kernel/index.js'
import {
  assertValidWorkerTrace,
  type WorkerTraceEntry,
  type WorkerTraceOutputEntry,
  type WorkerTraceResultEntry,
} from './worker-trace.js'

const DEFAULT_MAX_TAIL_CHARS = 4096
const SUCCESS_STATUSES = new Set(['completed', 'success', 'succeeded', 'ok'])
const NON_RETRYABLE_RESOURCE_STATUSES = new Set(['wall-clock-cap', 'output-cap', 'budget-cap', 'disk-cap'])

export interface RepairLoopState {
  readonly repairAttemptConsumed: boolean
}

export interface RepairLoopWorkerResult {
  readonly status: string
  readonly error?: string
  readonly outOfBounds?: readonly string[]
  readonly verifyRc?: number | null
  readonly verifyOutput?: string
  readonly stdoutTail?: string
  readonly stderrTail?: string
}

export interface RepairTraceSummary {
  readonly taskId: TaskId
  readonly attempts: readonly number[]
  readonly workerIds: readonly string[]
  readonly detections: readonly string[]
  readonly resultStatuses: readonly string[]
  readonly latestResultStatus: string
}

export interface RepairFailureArtifacts {
  readonly verifyOutput: string
  readonly verifyRc: number | null
  readonly workerResultStatus: string
  readonly workerError?: string
  readonly outOfBounds?: readonly string[]
  readonly traceSummary: RepairTraceSummary
  readonly stdoutTail: string
  readonly stderrTail: string
}

export interface RepairPlan {
  readonly taskId: TaskId
  readonly attempt: 1
  readonly verifyCommand: string
  readonly artifacts: RepairFailureArtifacts
}

export type TerminalRepairFailureReason =
  | 'human-review-required'
  | 'out-of-bounds'
  | 'non-retryable-resource-failure'
  | 'repair-attempt-consumed'

export type RepairLoopDecision =
  | {
      readonly kind: 'no-repair'
      readonly reason: 'succeeded'
      readonly state: RepairLoopState
    }
  | {
      readonly kind: 'repair'
      readonly state: RepairLoopState
      readonly plan: RepairPlan
    }
  | {
      readonly kind: 'terminal-failure'
      readonly reason: TerminalRepairFailureReason
      readonly state: RepairLoopState
      readonly artifacts: RepairFailureArtifacts
    }

export interface RepairLoopInput {
  readonly state: RepairLoopState
  readonly task: Pick<Task, 'id' | 'verify' | 'human_review_required'>
  readonly workerResult: RepairLoopWorkerResult
  readonly trace: readonly WorkerTraceEntry[]
  readonly maxTailChars?: number
}

export function createRepairLoopState(): RepairLoopState {
  return { repairAttemptConsumed: false }
}

export function decideRepairLoop(input: RepairLoopInput): RepairLoopDecision {
  assertValidWorkerTrace(input.trace)

  if (isSuccessfulWorkerResult(input.workerResult)) {
    return {
      kind: 'no-repair',
      reason: 'succeeded',
      state: input.state,
    }
  }

  const artifacts = collectFailureArtifacts(input)
  const terminalReason = classifyTerminalFailure(input)

  if (terminalReason !== null) {
    return {
      artifacts,
      kind: 'terminal-failure',
      reason: terminalReason,
      state: input.state,
    }
  }

  return {
    kind: 'repair',
    plan: {
      artifacts,
      attempt: 1,
      taskId: input.task.id,
      verifyCommand: input.task.verify,
    },
    state: { repairAttemptConsumed: true },
  }
}

function classifyTerminalFailure(input: RepairLoopInput): TerminalRepairFailureReason | null {
  if (input.task.human_review_required === true) {
    return 'human-review-required'
  }
  if ((input.workerResult.outOfBounds?.length ?? 0) > 0) {
    return 'out-of-bounds'
  }
  if (hasNonRetryableResourceFailure(input)) {
    return 'non-retryable-resource-failure'
  }
  if (input.state.repairAttemptConsumed) {
    return 'repair-attempt-consumed'
  }

  return null
}

function collectFailureArtifacts(input: RepairLoopInput): RepairFailureArtifacts {
  const maxTailChars = input.maxTailChars ?? DEFAULT_MAX_TAIL_CHARS

  return {
    ...(input.workerResult.outOfBounds === undefined ? {} : { outOfBounds: input.workerResult.outOfBounds }),
    ...(input.workerResult.error === undefined ? {} : { workerError: input.workerResult.error }),
    stderrTail: tailText(input.workerResult.stderrTail ?? collectTraceTail(input.trace, 'stderr'), maxTailChars),
    stdoutTail: tailText(input.workerResult.stdoutTail ?? collectTraceTail(input.trace, 'stdout'), maxTailChars),
    traceSummary: summarizeTrace(input.task.id, input.trace),
    verifyOutput: input.workerResult.verifyOutput ?? '',
    verifyRc: input.workerResult.verifyRc ?? null,
    workerResultStatus: input.workerResult.status,
  }
}

function summarizeTrace(taskId: TaskId, trace: readonly WorkerTraceEntry[]): RepairTraceSummary {
  const taskEntries = trace.filter((entry) => entry.taskId === taskId)
  const resultStatuses = taskEntries
    .filter((entry): entry is WorkerTraceResultEntry => entry.kind === 'result')
    .map(summarizeResult)

  return {
    attempts: uniqueSortedNumbers(taskEntries.map((entry) => entry.attempt)),
    detections: taskEntries
      .filter((entry) => entry.kind === 'detection')
      .map((entry) => entry.status ?? 'unknown'),
    latestResultStatus: resultStatuses.at(-1)?.replace(/^worker_(?:exited|finished):/, '') ?? 'none',
    resultStatuses,
    taskId,
    workerIds: uniqueSortedStrings(taskEntries.map((entry) => entry.workerId)),
  }
}

function summarizeResult(entry: WorkerTraceResultEntry): string {
  if (entry.sourceEventType === 'worker_exited') {
    return `${entry.sourceEventType}:exit-${String(entry.exitCode)}`
  }

  return `${entry.sourceEventType}:${entry.status ?? 'unknown'}`
}

function collectTraceTail(entries: readonly WorkerTraceEntry[], stream: WorkerTraceOutputEntry['stream']): string {
  return entries
    .filter((entry): entry is WorkerTraceOutputEntry => entry.kind === 'output' && entry.stream === stream)
    .map((entry) => entry.tail ?? '')
    .filter((tail) => tail.length > 0)
    .join('\n')
}

function tailText(text: string, maxChars: number): string {
  if (text.length <= maxChars) {
    return text
  }

  return text.slice(-maxChars)
}

function isSuccessfulWorkerResult(workerResult: RepairLoopWorkerResult): boolean {
  return workerResult.verifyRc === 0 && SUCCESS_STATUSES.has(workerResult.status)
}

function hasNonRetryableResourceFailure(input: RepairLoopInput): boolean {
  return input.trace.some(
    (entry) =>
      (entry.kind === 'detection' || entry.kind === 'result') &&
      entry.status !== undefined &&
      NON_RETRYABLE_RESOURCE_STATUSES.has(entry.status),
  )
}

function uniqueSortedNumbers(values: readonly number[]): readonly number[] {
  return [...new Set(values)].sort((left, right) => left - right)
}

function uniqueSortedStrings(values: readonly string[]): readonly string[] {
  return [...new Set(values)].sort()
}
