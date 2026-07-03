import { describe, expect, it } from 'vitest'

import {
  type AmendmentDelta,
  type AmendmentTaskSnapshot,
  validateAmendmentDelta,
} from './index.js'

const currentTasks: readonly AmendmentTaskSnapshot[] = [
  { id: 'T1', status: 'unstarted' },
  { id: 'T2', status: 'running' },
  { id: 'T3', status: 'landed' },
  { id: 'T4', status: 'completed', has_landed_work: true },
]

describe('validateAmendmentDelta', () => {
  it('accepts add, modify, cancel, guide, and follow_up operations for a newer plan version', () => {
    const delta: AmendmentDelta = {
      plan_version: 2,
      operations: [
        { action: 'add', task: { id: 'T5', title: 'new task' } },
        { action: 'modify', id: 'T1', task: { id: 'T1', title: 'renamed task' } },
        { action: 'cancel', id: 'T1', reason: 'merged into T5' },
        { action: 'guide', id: 'T2', guidance: 'restart with narrower scope' },
        { action: 'follow_up', id: 'T2', prompt: 'explain the failing assertion' },
      ],
    }

    expect(validateAmendmentDelta(delta, { current_plan_version: 1, tasks: currentTasks })).toEqual({
      valid: true,
      errors: [],
      guardrails: [],
      requires_human_checkpoint: false,
      next_plan_version: 2,
    })
  })

  it('rejects non-monotonic plan versions and add operations without unique stable ids', () => {
    const result = validateAmendmentDelta(
      {
        plan_version: 1,
        operations: [
          { action: 'add', task: { id: '' } },
          { action: 'add', task: { id: 'T1' } },
          { action: 'add', task: { id: 'T5' } },
          { action: 'add', task: { id: 'T5' } },
        ],
      },
      { current_plan_version: 1, tasks: currentTasks },
    )

    expect(result.valid).toBe(false)
    expect(result.errors).toEqual([
      {
        code: 'invalid_plan_version',
        message: 'plan_version must be an integer greater than current_plan_version',
      },
      {
        code: 'missing_task_id',
        message: 'added tasks must carry a stable id',
        operation_index: 0,
      },
      {
        code: 'duplicate_task_id',
        message: 'added task id must not duplicate an existing or newly added task',
        operation_index: 1,
        task_id: 'T1',
      },
      {
        code: 'duplicate_task_id',
        message: 'added task id must not duplicate an existing or newly added task',
        operation_index: 3,
        task_id: 'T5',
      },
    ])
  })

  it('rejects unknown task targets before applying target-specific rules', () => {
    const result = validateAmendmentDelta(
      {
        plan_version: 2,
        operations: [{ action: 'modify', id: 'T9', task: { id: 'renamed' } }],
      },
      { current_plan_version: 1, tasks: currentTasks },
    )

    expect(result.errors).toEqual([
      {
        code: 'unknown_task_id',
        message: 'operation references a task id that is not in the current plan',
        operation_index: 0,
        task_id: 'T9',
      },
    ])
  })

  it('enforces only-unstarted restructuring and stable ids on modify', () => {
    const result = validateAmendmentDelta(
      {
        plan_version: 2,
        operations: [
          { action: 'modify', id: 'T2', task: { id: 'renamed' } },
          { action: 'cancel', id: 'T2' },
        ],
      },
      { current_plan_version: 1, tasks: currentTasks },
    )

    expect(result.valid).toBe(false)
    expect(result.errors).toEqual([
      {
        code: 'restructure_started_task',
        message: 'modify and cancel may only restructure unstarted tasks',
        operation_index: 0,
        task_id: 'T2',
      },
      {
        code: 'id_changed_on_modify',
        message: 'modify must preserve the target task id',
        operation_index: 0,
        task_id: 'T2',
      },
      {
        code: 'restructure_started_task',
        message: 'modify and cancel may only restructure unstarted tasks',
        operation_index: 1,
        task_id: 'T2',
      },
    ])
  })

  it('requires human checkpoints for bulk cancels and landed work', () => {
    const result = validateAmendmentDelta(
      {
        plan_version: 2,
        operations: [
          { action: 'cancel', id: 'T1' },
          { action: 'guide', id: 'T3', guidance: 'inspect landed diff first' },
          { action: 'follow_up', id: 'T4', prompt: 'summarize landed changes' },
          { action: 'cancel', id: 'T1' },
          { action: 'cancel', id: 'T1' },
        ],
      },
      { current_plan_version: 1, tasks: currentTasks },
    )

    expect(result.requires_human_checkpoint).toBe(true)
    expect(result.guardrails).toEqual([
      {
        code: 'landed_work',
        message: 'amending a task with landed work requires a human checkpoint',
        operation_index: 1,
        task_id: 'T3',
      },
      {
        code: 'landed_work',
        message: 'amending a task with landed work requires a human checkpoint',
        operation_index: 2,
        task_id: 'T4',
      },
      {
        code: 'bulk_cancel',
        message: 'more than two cancels require a human checkpoint',
      },
    ])
  })
})
