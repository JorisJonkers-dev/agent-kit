export type AmendmentDeltaAction = 'add' | 'modify' | 'cancel' | 'guide' | 'follow_up'

export type AmendmentTaskStatus =
  | 'unstarted'
  | 'running'
  | 'restarting'
  | 'completed'
  | 'landed'
  | 'cancelled'

export interface AmendmentTaskRecord {
  readonly id: string
  readonly has_landed_work?: boolean
  readonly [key: string]: unknown
}

export interface AmendmentTaskSnapshot {
  readonly id: string
  readonly status: AmendmentTaskStatus
  readonly has_landed_work?: boolean
}

export interface AddTaskDelta {
  readonly action: 'add'
  readonly task: AmendmentTaskRecord
}

export interface ModifyTaskDelta {
  readonly action: 'modify'
  readonly id: string
  readonly task: AmendmentTaskRecord
}

export interface CancelTaskDelta {
  readonly action: 'cancel'
  readonly id: string
  readonly reason?: string
}

export interface GuideTaskDelta {
  readonly action: 'guide'
  readonly id: string
  readonly guidance: string
}

export interface FollowUpTaskDelta {
  readonly action: 'follow_up'
  readonly id: string
  readonly prompt: string
}

export type AmendmentDeltaOperation =
  | AddTaskDelta
  | ModifyTaskDelta
  | CancelTaskDelta
  | GuideTaskDelta
  | FollowUpTaskDelta

export interface AmendmentDelta {
  readonly plan_version: number
  readonly operations: readonly AmendmentDeltaOperation[]
}

export interface AmendmentValidationContext {
  readonly current_plan_version: number
  readonly tasks: readonly AmendmentTaskSnapshot[]
}

export type AmendmentValidationIssueCode =
  | 'duplicate_task_id'
  | 'id_changed_on_modify'
  | 'invalid_plan_version'
  | 'missing_task_id'
  | 'restructure_started_task'
  | 'unknown_task_id'

export interface AmendmentValidationIssue {
  readonly code: AmendmentValidationIssueCode
  readonly message: string
  readonly operation_index?: number
  readonly task_id?: string
}

export type AmendmentGuardrailCode = 'bulk_cancel' | 'landed_work'

export interface AmendmentGuardrail {
  readonly code: AmendmentGuardrailCode
  readonly message: string
  readonly operation_index?: number
  readonly task_id?: string
}

export interface AmendmentValidationResult {
  readonly valid: boolean
  readonly errors: readonly AmendmentValidationIssue[]
  readonly guardrails: readonly AmendmentGuardrail[]
  readonly requires_human_checkpoint: boolean
  readonly next_plan_version: number
}

const restructuringActions = new Set<AmendmentDeltaAction>(['modify', 'cancel'])

export function validateAmendmentDelta(
  delta: AmendmentDelta,
  context: AmendmentValidationContext,
): AmendmentValidationResult {
  const taskById = new Map(context.tasks.map((task) => [task.id, task]))
  const errors: AmendmentValidationIssue[] = []
  const guardrails: AmendmentGuardrail[] = []
  const addedIds = new Set<string>()
  let cancelCount = 0

  if (!Number.isInteger(delta.plan_version) || delta.plan_version <= context.current_plan_version) {
    errors.push({
      code: 'invalid_plan_version',
      message: 'plan_version must be an integer greater than current_plan_version',
    })
  }

  delta.operations.forEach((operation, operationIndex) => {
    if (operation.action === 'add') {
      validateAddOperation(operation, operationIndex, taskById, addedIds, errors)
      return
    }

    const target = taskById.get(operation.id)
    if (target === undefined) {
      errors.push({
        code: 'unknown_task_id',
        message: 'operation references a task id that is not in the current plan',
        operation_index: operationIndex,
        task_id: operation.id,
      })
      return
    }

    if (operation.action === 'cancel') {
      cancelCount += 1
    }

    validateLandedWorkGuardrail(target, operationIndex, guardrails)
    validateRestructuringInvariant(operation, target, operationIndex, errors)
    validateStableModifyId(operation, operationIndex, errors)
  })

  if (cancelCount > 2) {
    guardrails.push({
      code: 'bulk_cancel',
      message: 'more than two cancels require a human checkpoint',
    })
  }

  return {
    valid: errors.length === 0,
    errors,
    guardrails,
    requires_human_checkpoint: guardrails.length > 0,
    next_plan_version: delta.plan_version,
  }
}

function validateAddOperation(
  operation: AddTaskDelta,
  operationIndex: number,
  taskById: ReadonlyMap<string, AmendmentTaskSnapshot>,
  addedIds: Set<string>,
  errors: AmendmentValidationIssue[],
): void {
  if (operation.task.id.length === 0) {
    errors.push({
      code: 'missing_task_id',
      message: 'added tasks must carry a stable id',
      operation_index: operationIndex,
    })
    return
  }

  if (taskById.has(operation.task.id) || addedIds.has(operation.task.id)) {
    errors.push({
      code: 'duplicate_task_id',
      message: 'added task id must not duplicate an existing or newly added task',
      operation_index: operationIndex,
      task_id: operation.task.id,
    })
    return
  }

  addedIds.add(operation.task.id)
}

function validateLandedWorkGuardrail(
  task: AmendmentTaskSnapshot,
  operationIndex: number,
  guardrails: AmendmentGuardrail[],
): void {
  if (task.status !== 'landed' && task.has_landed_work !== true) {
    return
  }

  guardrails.push({
    code: 'landed_work',
    message: 'amending a task with landed work requires a human checkpoint',
    operation_index: operationIndex,
    task_id: task.id,
  })
}

function validateRestructuringInvariant(
  operation: Exclude<AmendmentDeltaOperation, AddTaskDelta>,
  task: AmendmentTaskSnapshot,
  operationIndex: number,
  errors: AmendmentValidationIssue[],
): void {
  if (!restructuringActions.has(operation.action) || task.status === 'unstarted') {
    return
  }

  errors.push({
    code: 'restructure_started_task',
    message: 'modify and cancel may only restructure unstarted tasks',
    operation_index: operationIndex,
    task_id: task.id,
  })
}

function validateStableModifyId(
  operation: Exclude<AmendmentDeltaOperation, AddTaskDelta>,
  operationIndex: number,
  errors: AmendmentValidationIssue[],
): void {
  if (operation.action !== 'modify' || operation.task.id === operation.id) {
    return
  }

  errors.push({
    code: 'id_changed_on_modify',
    message: 'modify must preserve the target task id',
    operation_index: operationIndex,
    task_id: operation.id,
  })
}
