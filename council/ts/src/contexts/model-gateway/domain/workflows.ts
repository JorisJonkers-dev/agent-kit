import { isObject, ModelGatewayError } from './config.js'

/**
 * A workflow's life: the name every delegation of one piece of work carries,
 * and the moment it started and ended. Caps and grants count against it, so a
 * workflow that nobody ends must not stay alive for ever.
 */

/** The version the workflows file is written as. */
export const WORKFLOWS_VERSION = 1

/** How long a workflow lives when nobody ends it. */
export const WORKFLOW_TTL_MS = 24 * 60 * 60 * 1000

/** The characters a workflow name may hold: a letter or digit first, then letters, digits and `._:/-`, at most 80. */
export const WORKFLOW_NAME = /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,79}$/

/**
 * The workflow name `value` names, refused when it is missing or is not a name
 * a workflow may carry. A name reaches the shell command the gateway suggests
 * and the paths a run keeps, so it is held to letters, digits and `._:/-`.
 */
export function requireWorkflowName(value: unknown, missing: string): string {
  if (typeof value !== 'string' || value === '') throw new ModelGatewayError('invalid-input', missing)
  if (!WORKFLOW_NAME.test(value)) {
    throw new ModelGatewayError(
      'invalid-input',
      `workflow ${JSON.stringify(value)} is not a workflow name: a letter or digit, then letters, digits and . _ : / - only, at most 80 characters`,
    )
  }
  return value
}

/** One workflow, as the file holds it. */
export interface WorkflowRecord {
  readonly name: string
  /** When it was first named, as an ISO timestamp. */
  readonly started_at: string
  /** When someone ended it, as an ISO timestamp; absent while it runs. */
  readonly ended_at?: string
}

/** Whether a workflow is still open to runs. */
export type WorkflowState = 'running' | 'ended'

/** What a change to the workflows leaves behind: the workflows to write, and the one it touched. */
export interface WorkflowChange {
  readonly workflows: readonly WorkflowRecord[]
  readonly workflow: WorkflowRecord
}

/** One workflow as a caller reads it: its life, with the 24-hour end worked out. */
export interface WorkflowView {
  readonly name: string
  readonly started_at: string
  /** When it ended: the moment someone ended it, or the moment its 24 hours ran out; null while it runs. */
  readonly ended_at: string | null
  readonly state: WorkflowState
}

/**
 * The workflows a file holds. A file that is not JSON, of another version, or
 * whose entries are not workflows is one error naming what is wrong, rather
 * than a surprise when a run joins a workflow that is not there.
 */
export function parseWorkflowsFile(text: string): readonly WorkflowRecord[] {
  let raw: unknown
  try {
    raw = JSON.parse(text)
  } catch (error) {
    throw invalid(`not JSON: ${messageOf(error)}`)
  }
  if (!isObject(raw)) throw invalid('the workflows file must be a JSON object')
  if (raw.version !== WORKFLOWS_VERSION) throw invalid(`version must be ${String(WORKFLOWS_VERSION)}`)
  if (!Array.isArray(raw.workflows)) throw invalid('workflows must be a list')
  const workflows: readonly unknown[] = raw.workflows
  return workflows.map((value, index) => parseWorkflow(value, index))
}

function parseWorkflow(raw: unknown, index: number): WorkflowRecord {
  const where = `workflows[${String(index)}]`
  if (!isObject(raw)) throw invalid(`${where} must be an object`)
  const name = requireString(raw.name, `${where}.name`)
  const startedAt = requireTimestamp(raw.started_at, `${where}.started_at`)
  if (raw.ended_at === undefined) return { name, started_at: startedAt }
  return { ended_at: requireTimestamp(raw.ended_at, `${where}.ended_at`), name, started_at: startedAt }
}

function requireString(value: unknown, where: string): string {
  if (typeof value !== 'string' || value === '') throw invalid(`${where} must be a non-empty string`)
  return value
}

function requireTimestamp(value: unknown, where: string): string {
  if (typeof value !== 'string' || Number.isNaN(Date.parse(value))) throw invalid(`${where} must be an ISO timestamp`)
  return value
}

function invalid(message: string): ModelGatewayError {
  return new ModelGatewayError('invalid-config', message)
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** The moment a workflow's 24 hours run out. */
function expiryOf(record: WorkflowRecord): string {
  return new Date(Date.parse(record.started_at) + WORKFLOW_TTL_MS).toISOString()
}

/** The moment a workflow stops taking runs: when someone ended it, or when its 24 hours run out, whichever comes first. */
export function workflowEndsAt(record: WorkflowRecord): Date {
  const lifeEnd = Date.parse(expiryOf(record))
  return new Date(record.ended_at === undefined ? lifeEnd : Math.min(lifeEnd, Date.parse(record.ended_at)))
}

/** Whether a workflow is closed to runs: someone ended it, or its 24 hours ran out. */
export function workflowEnded(record: WorkflowRecord, now: Date): boolean {
  if (record.ended_at !== undefined) return true
  return now.getTime() - Date.parse(record.started_at) >= WORKFLOW_TTL_MS
}

/** One workflow as a caller reads it, with the 24-hour end worked out from the clock. */
export function workflowView(record: WorkflowRecord, now: Date): WorkflowView {
  const endedAt = record.ended_at ?? (workflowEnded(record, now) ? expiryOf(record) : null)
  return {
    ended_at: endedAt,
    name: record.name,
    started_at: record.started_at,
    state: endedAt === null ? 'running' : 'ended',
  }
}

/** Every workflow as a caller reads it, in the order the file holds them. */
export function workflowViews(records: readonly WorkflowRecord[], now: Date): readonly WorkflowView[] {
  return records.map((record) => workflowView(record, now))
}

/**
 * The workflow `name` names, refused when it ended: its caps and grants lapsed
 * with it, so a run may not quietly revive it. It starts nothing, so a run can
 * be refused before it reserves anything.
 */
export function requireWorkflowOpen(
  records: readonly WorkflowRecord[],
  name: string,
  now: Date,
): WorkflowRecord | undefined {
  const existing = records.find((record) => record.name === name)
  if (existing === undefined || !workflowEnded(existing, now)) return existing
  const endedAt = existing.ended_at ?? expiryOf(existing)
  throw new ModelGatewayError('workflow-ended', `workflow ${name} ended at ${endedAt}: a run may not join it, so start a new workflow`)
}

/**
 * The workflows with `name` started at `now` if it is new, or as they are when
 * it is already running. A workflow that ended is refused: its caps and grants
 * lapsed with it, so a run may not quietly revive it.
 */
export function startWorkflow(records: readonly WorkflowRecord[], name: string, now: Date): WorkflowChange {
  const existing = requireWorkflowOpen(records, name, now)
  if (existing === undefined) {
    const started: WorkflowRecord = { name, started_at: now.toISOString() }
    return { workflow: started, workflows: [...records, started] }
  }
  return { workflow: existing, workflows: records }
}

/**
 * The workflows with `name` ended at `now`. One that already ended keeps the
 * moment it ended; one this machine never started is an error, so a typo does
 * not pass for a workflow that ran.
 */
export function endWorkflow(records: readonly WorkflowRecord[], name: string, now: Date): WorkflowChange {
  const existing = records.find((record) => record.name === name)
  if (existing === undefined) {
    throw new ModelGatewayError('unknown-workflow', `no workflow named ${name} has started on this machine`)
  }
  if (workflowEnded(existing, now)) return { workflow: existing, workflows: records }
  const ended: WorkflowRecord = { ...existing, ended_at: now.toISOString() }
  return { workflow: ended, workflows: records.map((record) => (record.name === name ? ended : record)) }
}
