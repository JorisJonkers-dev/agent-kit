import { assertTasksJsonSchema, validateTasks } from '../../../tasks/index.js'
import type {
  Amendment,
  DesignLedger,
  JsonRecord,
  JsonValue,
  ReviewVerdict,
  RoutingVerdict,
  RunState,
  Story,
  Task,
} from '../../../../shared-kernel/index.js'
import type {
  RunStoreEvent,
  WorkerDetectedPayload,
  WorkerExitedPayload,
  WorkerFinishedPayload,
  WorkerLifecycleEvent,
  WorkerOutputPayload,
  WorkerRestartedPayload,
  WorkerStartedPayload,
} from '../../../runstore/index.js'

const WORKER_OUTPUT_TAIL_MAX_CHARS = 4096

export const STORY_FILE = 'story.json'
export const DESIGN_LEDGER_FILE = 'design-ledger.json'
export const WORKERS_DIR = 'workers'
export const RESULT_FILE = 'result.json'

export interface WorkerResult {
  readonly task_id: string
  readonly status: string
  readonly title?: string
  readonly model?: string
  readonly suggested_model?: 'haiku' | 'sonnet' | 'opus'
  readonly branch?: string
  readonly worktree?: string
  readonly committed?: boolean
  readonly summary?: string
  readonly files_changed?: readonly string[]
  readonly out_of_bounds?: readonly string[]
  readonly verify_rc?: number | null
  readonly verify_output?: string
  readonly verdict?: ReviewVerdict | null
  readonly merge?: string
  readonly error?: string
  readonly content_hash?: string
  readonly engine?: unknown
  readonly model_tier?: string
  readonly stdout_tail?: string
  readonly stderr_tail?: string
  readonly stdout_log_path?: string
  readonly stderr_log_path?: string
  readonly stdout_bytes?: number
  readonly stderr_bytes?: number
}

export interface LegacyTaskReport {
  readonly task_id: string
  readonly status?: string
  readonly merge?: string
  readonly model?: string
  readonly files_changed?: readonly string[]
  readonly verify_rc?: number | null
  readonly verifier_satisfied?: boolean
  readonly out_of_bounds?: readonly string[]
  readonly branch?: string
  readonly good?: boolean
}

export interface LegacyRunReport {
  readonly run: string
  readonly integration_branch?: string
  readonly integration_worktree?: string
  readonly waves: readonly (readonly string[])[]
  readonly tasks: readonly LegacyTaskReport[]
}

export function parseJson(text: string): unknown {
  return JSON.parse(text) as unknown
}

export function assertRunState(value: unknown): RunState {
  const record = assertRecord(value, 'state')
  assertAllowed(record, 'state', [
    'stage',
    'intensity',
    'rounds',
    'task_count',
    'spec_id',
    'spec_slug',
    'spec_relpath',
    'agents',
    'integration_branch',
    'engine',
    'model_tier',
    'content_hash',
  ])
  optionalString(record, 'state', 'stage')
  optionalString(record, 'state', 'intensity')
  optionalInteger(record, 'state', 'rounds')
  optionalInteger(record, 'state', 'task_count')
  optionalString(record, 'state', 'spec_id')
  optionalString(record, 'state', 'spec_slug')
  optionalString(record, 'state', 'spec_relpath')
  optionalStringArray(record, 'state', 'agents')
  optionalString(record, 'state', 'integration_branch')
  optionalString(record, 'state', 'model_tier')
  optionalString(record, 'state', 'content_hash')
  return record
}

export function assertTasks(value: unknown): readonly Task[] {
  validateTasks(value)
  assertTasksJsonSchema(value)
  return value as unknown as readonly Task[]
}

export function assertStory(value: unknown): Story {
  const record = assertRecord(value, 'story')
  assertAllowed(record, 'story', [
    'title',
    'status',
    'goal',
    'user_value',
    'context',
    'acceptance_criteria',
    'scope',
    'implementation_notes',
    'tests',
    'definition_of_done',
  ])
  requiredString(record, 'story', 'title')
  requiredString(record, 'story', 'status')
  requiredString(record, 'story', 'goal')
  assertStoryUserValue(record.user_value)
  requiredString(record, 'story', 'context')
  requiredStringArray(record, 'story', 'acceptance_criteria')
  assertStoryScope(record.scope)
  assertStoryImplementationNotes(record.implementation_notes)
  assertStoryTests(record.tests)
  requiredStringArray(record, 'story', 'definition_of_done')
  return record as unknown as Story
}

export function assertDesignLedger(value: unknown): DesignLedger {
  const record = assertRecord(value, 'design ledger')
  assertAllowed(record, 'design ledger', ['entries', 'content_hash'])
  optionalString(record, 'design ledger', 'content_hash')
  if ('entries' in record) requiredArray(record, 'design ledger', 'entries').forEach(assertDesignLedgerEntry)
  return record
}

export function assertReviewVerdict(value: unknown): ReviewVerdict {
  const record = assertRecord(value, 'review verdict')
  assertAllowed(record, 'review verdict', [
    'satisfied',
    'reasons',
    'issues',
    'task_id',
    'reviewer',
    'engine',
    'model_tier',
    'content_hash',
  ])
  requiredBoolean(record, 'review verdict', 'satisfied')
  requiredString(record, 'review verdict', 'reasons')
  requiredStringArray(record, 'review verdict', 'issues')
  optionalString(record, 'review verdict', 'task_id')
  optionalString(record, 'review verdict', 'reviewer')
  optionalString(record, 'review verdict', 'model_tier')
  optionalString(record, 'review verdict', 'content_hash')
  return record as unknown as ReviewVerdict
}

export function assertRoutingVerdict(value: unknown): RoutingVerdict {
  const record = assertRecord(value, 'routing verdict')
  assertAllowed(record, 'routing verdict', [
    'route',
    'reasons',
    'task_id',
    'candidate_routes',
    'engine',
    'model_tier',
    'context_refs',
    'content_hash',
  ])
  requiredString(record, 'routing verdict', 'route')
  requiredString(record, 'routing verdict', 'reasons')
  optionalString(record, 'routing verdict', 'task_id')
  optionalStringArray(record, 'routing verdict', 'candidate_routes')
  optionalStringArray(record, 'routing verdict', 'context_refs')
  optionalString(record, 'routing verdict', 'model_tier')
  optionalString(record, 'routing verdict', 'content_hash')
  return record as unknown as RoutingVerdict
}

export function assertAmendment(value: unknown): Amendment {
  const record = assertRecord(value, 'amendment')
  assertAllowed(record, 'amendment', [
    'id',
    'summary',
    'reason',
    'status',
    'task_refs',
    'supersedes',
    'context_refs',
    'discovered_from',
    'engine',
    'model_tier',
    'content_hash',
  ])
  requiredString(record, 'amendment', 'id')
  requiredString(record, 'amendment', 'summary')
  optionalString(record, 'amendment', 'reason')
  optionalString(record, 'amendment', 'status')
  optionalStringArray(record, 'amendment', 'task_refs')
  optionalStringArray(record, 'amendment', 'supersedes')
  optionalStringArray(record, 'amendment', 'context_refs')
  optionalString(record, 'amendment', 'discovered_from')
  optionalString(record, 'amendment', 'model_tier')
  optionalString(record, 'amendment', 'content_hash')
  return record as unknown as Amendment
}

export function assertWorkerResult(value: unknown, taskId?: string): WorkerResult {
  const record = assertRecord(value, 'worker result')
  assertAllowed(record, 'worker result', [
    'task_id',
    'title',
    'model',
    'suggested_model',
    'engine',
    'model_tier',
    'branch',
    'worktree',
    'committed',
    'summary',
    'files_changed',
    'out_of_bounds',
    'verify_rc',
    'verify_output',
    'verdict',
    'merge',
    'status',
    'error',
    'content_hash',
    'stdout_tail',
    'stderr_tail',
    'stdout_log_path',
    'stderr_log_path',
    'stdout_bytes',
    'stderr_bytes',
  ])
  requiredString(record, 'worker result', 'task_id')
  requiredString(record, 'worker result', 'status')
  if (taskId !== undefined && record.task_id !== taskId) {
    fail(`worker result task_id must match path task id: ${taskId}`)
  }
  optionalString(record, 'worker result', 'title')
  optionalString(record, 'worker result', 'model')
  optionalEnum(record, 'worker result', 'suggested_model', ['haiku', 'sonnet', 'opus'])
  optionalString(record, 'worker result', 'model_tier')
  optionalString(record, 'worker result', 'branch')
  optionalString(record, 'worker result', 'worktree')
  optionalBoolean(record, 'worker result', 'committed')
  optionalString(record, 'worker result', 'summary')
  optionalStringArray(record, 'worker result', 'files_changed')
  optionalStringArray(record, 'worker result', 'out_of_bounds')
  optionalIntegerOrNull(record, 'worker result', 'verify_rc')
  optionalString(record, 'worker result', 'verify_output')
  if (record.verdict !== undefined && record.verdict !== null) assertReviewVerdict(record.verdict)
  optionalString(record, 'worker result', 'merge')
  optionalString(record, 'worker result', 'error')
  optionalString(record, 'worker result', 'content_hash')
  optionalBoundedString(record, 'worker result', 'stdout_tail', WORKER_OUTPUT_TAIL_MAX_CHARS)
  optionalBoundedString(record, 'worker result', 'stderr_tail', WORKER_OUTPUT_TAIL_MAX_CHARS)
  optionalString(record, 'worker result', 'stdout_log_path')
  optionalString(record, 'worker result', 'stderr_log_path')
  optionalNonNegativeInteger(record, 'worker result', 'stdout_bytes')
  optionalNonNegativeInteger(record, 'worker result', 'stderr_bytes')
  return record as unknown as WorkerResult
}

export function assertLegacyReport(value: unknown): LegacyRunReport {
  const record = assertRecord(value, 'legacy report')
  requiredString(record, 'legacy report', 'run')
  optionalString(record, 'legacy report', 'integration_branch')
  optionalString(record, 'legacy report', 'integration_worktree')
  requiredArray(record, 'legacy report', 'waves').forEach((wave) => {
    assertStringArray(wave, 'legacy report wave')
  })
  requiredArray(record, 'legacy report', 'tasks').forEach(assertLegacyTaskReport)
  return record as unknown as LegacyRunReport
}

export function assertRunStoreEvent(value: unknown): RunStoreEvent {
  const record = assertRecord(value, 'run store event')
  if (record.type === 'review_verdict') {
    return { type: 'review_verdict', payload: assertReviewVerdict(record.payload) }
  }
  if (record.type === 'routing_verdict') {
    return { type: 'routing_verdict', payload: assertRoutingVerdict(record.payload) }
  }
  if (record.type === 'amendment') {
    return { type: 'amendment', payload: assertAmendment(record.payload) }
  }
  if (record.type === 'worker_started') {
    return { type: 'worker_started', payload: assertWorkerStarted(record.payload) }
  }
  if (record.type === 'worker_output') {
    return { type: 'worker_output', payload: assertWorkerOutput(record.payload) }
  }
  if (record.type === 'worker_detected') {
    return { type: 'worker_detected', payload: assertWorkerDetected(record.payload) }
  }
  if (record.type === 'worker_restarted') {
    return { type: 'worker_restarted', payload: assertWorkerRestarted(record.payload) }
  }
  if (record.type === 'worker_exited') {
    return { type: 'worker_exited', payload: assertWorkerExited(record.payload) }
  }
  if (record.type === 'worker_finished') {
    return { type: 'worker_finished', payload: assertWorkerFinished(record.payload) }
  }
  fail(`unsupported run store event type: ${formatJsonValue(record.type)}`)
}

export function assertWorkerLifecycleEvent(value: unknown): WorkerLifecycleEvent {
  const event = assertRunStoreEvent(value)
  if (
    event.type === 'worker_started' ||
    event.type === 'worker_output' ||
    event.type === 'worker_detected' ||
    event.type === 'worker_restarted' ||
    event.type === 'worker_exited' ||
    event.type === 'worker_finished'
  ) {
    return event
  }
  fail('worker event type is required')
}

export function assertRecord(value: unknown, label: string): JsonRecord {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) fail(`${label} must be an object`)
  return value as JsonRecord
}

export function copyOptionalString(from: JsonRecord, to: Record<string, unknown>, field: string): void {
  if (typeof from[field] === 'string') to[field] = from[field]
}

export function copyOptionalInteger(from: JsonRecord, to: Record<string, unknown>, field: string): void {
  if (Number.isInteger(from[field])) to[field] = from[field]
}

export function assertPathSegment(label: string, value: string): void {
  if (value.length === 0) fail(`${label} must not be empty`)
  if (value.includes('/') || value.includes('\\') || value.includes('\0')) {
    fail(`${label} must be a single path segment`)
  }
}

export function isErrno(error: unknown, code: string): boolean {
  return error instanceof Error && 'code' in error && error.code === code
}

function assertStoryUserValue(value: unknown): void {
  const record = assertRecord(value, 'story.user_value')
  assertAllowed(record, 'story.user_value', ['actor', 'capability', 'outcome'])
  requiredString(record, 'story.user_value', 'actor')
  requiredString(record, 'story.user_value', 'capability')
  requiredString(record, 'story.user_value', 'outcome')
}

function assertStoryScope(value: unknown): void {
  const record = assertRecord(value, 'story.scope')
  assertAllowed(record, 'story.scope', ['in_scope', 'out_of_scope'])
  requiredStringArray(record, 'story.scope', 'in_scope')
  requiredStringArray(record, 'story.scope', 'out_of_scope')
}

function assertStoryImplementationNotes(value: unknown): void {
  const record = assertRecord(value, 'story.implementation_notes')
  assertAllowed(record, 'story.implementation_notes', [
    'files',
    'patterns',
    'dependencies',
    'data_config_migration',
  ])
  requiredStringArray(record, 'story.implementation_notes', 'files')
  requiredStringArray(record, 'story.implementation_notes', 'patterns')
  requiredStringArray(record, 'story.implementation_notes', 'dependencies')
  requiredStringArray(record, 'story.implementation_notes', 'data_config_migration')
}

function assertStoryTests(value: unknown): void {
  const record = assertRecord(value, 'story.tests')
  assertAllowed(record, 'story.tests', ['unit', 'integration', 'manual_or_workflow'])
  requiredStringArray(record, 'story.tests', 'unit')
  requiredStringArray(record, 'story.tests', 'integration')
  requiredStringArray(record, 'story.tests', 'manual_or_workflow')
}

function assertDesignLedgerEntry(value: unknown): void {
  const record = assertRecord(value, 'design ledger entry')
  assertAllowed(record, 'design ledger entry', [
    'id',
    'decision',
    'rationale',
    'status',
    'task_refs',
    'context_refs',
    'supersedes',
    'content_hash',
  ])
  requiredString(record, 'design ledger entry', 'id')
  requiredString(record, 'design ledger entry', 'decision')
  optionalString(record, 'design ledger entry', 'rationale')
  optionalString(record, 'design ledger entry', 'status')
  optionalStringArray(record, 'design ledger entry', 'task_refs')
  optionalStringArray(record, 'design ledger entry', 'context_refs')
  optionalStringArray(record, 'design ledger entry', 'supersedes')
  optionalString(record, 'design ledger entry', 'content_hash')
}

function assertLegacyTaskReport(value: unknown): void {
  const record = assertRecord(value, 'legacy task report')
  requiredString(record, 'legacy task report', 'task_id')
  optionalString(record, 'legacy task report', 'status')
  optionalString(record, 'legacy task report', 'merge')
  optionalString(record, 'legacy task report', 'model')
  optionalStringArray(record, 'legacy task report', 'files_changed')
  optionalIntegerOrNull(record, 'legacy task report', 'verify_rc')
  optionalBoolean(record, 'legacy task report', 'verifier_satisfied')
  optionalStringArray(record, 'legacy task report', 'out_of_bounds')
  optionalString(record, 'legacy task report', 'branch')
  optionalBoolean(record, 'legacy task report', 'good')
}

function assertWorkerStarted(value: unknown): WorkerStartedPayload {
  const record = assertRecord(value, 'worker started')
  assertAllowed(record, 'worker started', [
    'worker_id',
    'task_id',
    'attempt',
    'pid',
    'command',
    'cwd',
    'started_at',
    'engine',
    'model_tier',
    'content_hash',
  ])
  requiredString(record, 'worker started', 'worker_id')
  optionalString(record, 'worker started', 'task_id')
  optionalNonNegativeInteger(record, 'worker started', 'attempt')
  optionalNonNegativeInteger(record, 'worker started', 'pid')
  optionalStringArray(record, 'worker started', 'command')
  optionalString(record, 'worker started', 'cwd')
  optionalString(record, 'worker started', 'started_at')
  optionalString(record, 'worker started', 'model_tier')
  optionalString(record, 'worker started', 'content_hash')
  return record as unknown as WorkerStartedPayload
}

function assertWorkerOutput(value: unknown): WorkerOutputPayload {
  const record = assertRecord(value, 'worker output')
  assertAllowed(record, 'worker output', [
    'worker_id',
    'task_id',
    'stream',
    'offset',
    'byte_count',
    'tail',
    'tail_bytes',
    'log_path',
    'sha256',
    'content_hash',
  ])
  requiredString(record, 'worker output', 'worker_id')
  optionalString(record, 'worker output', 'task_id')
  requiredEnum(record, 'worker output', 'stream', ['stdout', 'stderr'])
  requiredNonNegativeInteger(record, 'worker output', 'offset')
  requiredNonNegativeInteger(record, 'worker output', 'byte_count')
  optionalBoundedString(record, 'worker output', 'tail', WORKER_OUTPUT_TAIL_MAX_CHARS)
  optionalNonNegativeInteger(record, 'worker output', 'tail_bytes')
  optionalString(record, 'worker output', 'log_path')
  optionalString(record, 'worker output', 'sha256')
  optionalString(record, 'worker output', 'content_hash')
  return record as unknown as WorkerOutputPayload
}

function assertWorkerDetected(value: unknown): WorkerDetectedPayload {
  const record = assertRecord(value, 'worker detected')
  assertAllowed(record, 'worker detected', [
    'worker_id',
    'task_id',
    'pid',
    'status',
    'detected_at',
    'content_hash',
  ])
  requiredString(record, 'worker detected', 'worker_id')
  optionalString(record, 'worker detected', 'task_id')
  optionalNonNegativeInteger(record, 'worker detected', 'pid')
  optionalString(record, 'worker detected', 'status')
  optionalString(record, 'worker detected', 'detected_at')
  optionalString(record, 'worker detected', 'content_hash')
  return record as unknown as WorkerDetectedPayload
}

function assertWorkerRestarted(value: unknown): WorkerRestartedPayload {
  const record = assertRecord(value, 'worker restarted')
  assertAllowed(record, 'worker restarted', [
    'worker_id',
    'task_id',
    'attempt',
    'previous_pid',
    'pid',
    'reason',
    'restarted_at',
    'content_hash',
  ])
  requiredString(record, 'worker restarted', 'worker_id')
  optionalString(record, 'worker restarted', 'task_id')
  requiredNonNegativeInteger(record, 'worker restarted', 'attempt')
  optionalNonNegativeInteger(record, 'worker restarted', 'previous_pid')
  optionalNonNegativeInteger(record, 'worker restarted', 'pid')
  optionalString(record, 'worker restarted', 'reason')
  optionalString(record, 'worker restarted', 'restarted_at')
  optionalString(record, 'worker restarted', 'content_hash')
  return record as unknown as WorkerRestartedPayload
}

function assertWorkerExited(value: unknown): WorkerExitedPayload {
  const record = assertRecord(value, 'worker exited')
  assertAllowed(record, 'worker exited', [
    'worker_id',
    'task_id',
    'pid',
    'exit_code',
    'signal',
    'duration_ms',
    'exited_at',
    'content_hash',
  ])
  requiredString(record, 'worker exited', 'worker_id')
  optionalString(record, 'worker exited', 'task_id')
  optionalNonNegativeInteger(record, 'worker exited', 'pid')
  requiredIntegerOrNull(record, 'worker exited', 'exit_code')
  optionalStringOrNull(record, 'worker exited', 'signal')
  optionalNonNegativeInteger(record, 'worker exited', 'duration_ms')
  optionalString(record, 'worker exited', 'exited_at')
  optionalString(record, 'worker exited', 'content_hash')
  return record as unknown as WorkerExitedPayload
}

function assertWorkerFinished(value: unknown): WorkerFinishedPayload {
  const record = assertRecord(value, 'worker finished')
  assertAllowed(record, 'worker finished', [
    'worker_id',
    'task_id',
    'status',
    'result_path',
    'duration_ms',
    'finished_at',
    'content_hash',
  ])
  requiredString(record, 'worker finished', 'worker_id')
  requiredString(record, 'worker finished', 'task_id')
  requiredString(record, 'worker finished', 'status')
  optionalString(record, 'worker finished', 'result_path')
  optionalNonNegativeInteger(record, 'worker finished', 'duration_ms')
  optionalString(record, 'worker finished', 'finished_at')
  optionalString(record, 'worker finished', 'content_hash')
  return record as unknown as WorkerFinishedPayload
}

function assertAllowed(record: JsonRecord, label: string, allowed: readonly string[]): void {
  const allowedSet = new Set(allowed)
  Object.keys(record).forEach((key) => {
    if (!allowedSet.has(key)) fail(`${label}.${key} is not allowed`)
  })
}

function requiredString(record: JsonRecord, label: string, field: string): string {
  const value = record[field]
  if (typeof value !== 'string') fail(`${label}.${field} must be a string`)
  return value
}

function optionalString(record: JsonRecord, label: string, field: string): void {
  if (record[field] !== undefined && typeof record[field] !== 'string') fail(`${label}.${field} must be a string`)
}

function optionalBoundedString(record: JsonRecord, label: string, field: string, maxChars: number): void {
  optionalString(record, label, field)
  if (typeof record[field] === 'string' && record[field].length > maxChars) {
    fail(`${label}.${field} must be at most ${String(maxChars)} characters`)
  }
}

function optionalStringOrNull(record: JsonRecord, label: string, field: string): void {
  if (record[field] !== undefined && record[field] !== null && typeof record[field] !== 'string') {
    fail(`${label}.${field} must be a string or null`)
  }
}

function requiredBoolean(record: JsonRecord, label: string, field: string): void {
  if (typeof record[field] !== 'boolean') fail(`${label}.${field} must be a boolean`)
}

function optionalBoolean(record: JsonRecord, label: string, field: string): void {
  if (record[field] !== undefined && typeof record[field] !== 'boolean') fail(`${label}.${field} must be a boolean`)
}

function optionalEnum(record: JsonRecord, label: string, field: string, values: readonly string[]): void {
  const value = record[field]
  if (value !== undefined && (typeof value !== 'string' || !values.includes(value))) {
    fail(`${label}.${field} must be one of: ${values.join(', ')}`)
  }
}

function requiredEnum(record: JsonRecord, label: string, field: string, values: readonly string[]): void {
  const value = record[field]
  if (typeof value !== 'string' || !values.includes(value)) fail(`${label}.${field} must be one of: ${values.join(', ')}`)
}

function optionalInteger(record: JsonRecord, label: string, field: string): void {
  if (record[field] !== undefined && !Number.isInteger(record[field])) fail(`${label}.${field} must be an integer`)
}

function requiredNonNegativeInteger(record: JsonRecord, label: string, field: string): void {
  if (!Number.isInteger(record[field]) || Number(record[field]) < 0) {
    fail(`${label}.${field} must be a non-negative integer`)
  }
}

function optionalNonNegativeInteger(record: JsonRecord, label: string, field: string): void {
  if (record[field] !== undefined && (!Number.isInteger(record[field]) || Number(record[field]) < 0)) {
    fail(`${label}.${field} must be a non-negative integer`)
  }
}

function optionalIntegerOrNull(record: JsonRecord, label: string, field: string): void {
  if (record[field] !== undefined && record[field] !== null && !Number.isInteger(record[field])) {
    fail(`${label}.${field} must be an integer or null`)
  }
}

function requiredIntegerOrNull(record: JsonRecord, label: string, field: string): void {
  if (record[field] !== null && !Number.isInteger(record[field])) {
    fail(`${label}.${field} must be an integer or null`)
  }
}

function requiredArray(record: JsonRecord, label: string, field: string): readonly unknown[] {
  const value = record[field]
  if (!Array.isArray(value)) fail(`${label}.${field} must be an array`)
  return value
}

function requiredStringArray(record: JsonRecord, label: string, field: string): void {
  assertStringArray(requiredArray(record, label, field), `${label}.${field}`)
}

function optionalStringArray(record: JsonRecord, label: string, field: string): void {
  if (record[field] !== undefined) assertStringArray(record[field], `${label}.${field}`)
}

function assertStringArray(value: unknown, label: string): void {
  if (!Array.isArray(value) || value.some((item) => typeof item !== 'string')) {
    fail(`${label} must be an array of strings`)
  }
}

function formatJsonValue(value: JsonValue | undefined): string {
  return typeof value === 'string' ? value : JSON.stringify(value)
}

function fail(message: string): never {
  throw new Error(message)
}
