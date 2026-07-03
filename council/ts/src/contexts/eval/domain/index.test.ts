import { describe, expect, it } from 'vitest'

import type { ReviewVerdict, Task } from '../../../shared-kernel/index.js'
import {
  scoreEvalRun,
  type EvalRunInput,
  type EvalWorkerResult,
} from './index.js'

const baseTask: Task = {
  id: 'T1',
  title: 'Eval scorer',
  objective: 'Score normalized worker output',
  output_format: 'Code edits',
  paths: ['src/eval.ts'],
  depends_on: [],
  difficulty: 'moderate',
  model: 'sonnet',
  verify: 'npm test',
  verify_proves: ['unit and integration checks cover the changed behavior'],
  boundaries: 'Only src/eval.ts',
  acceptance_criteria: ['score clean and risky runs'],
}

const satisfiedVerdict: ReviewVerdict = {
  engine: { cli: 'codex', model: 'gpt-5' },
  satisfied: true,
  reasons: 'Checks passed and diff matches the task.',
  issues: [],
  task_id: 'T1',
}

function task(overrides: Partial<Task> = {}): Task {
  return { ...baseTask, ...overrides }
}

function result(overrides: Partial<EvalWorkerResult> = {}): EvalWorkerResult {
  return {
    task_id: 'T1',
    status: 'ok',
    files_changed: ['src/eval.ts'],
    out_of_bounds: [],
    verify_rc: 0,
    verdict: satisfiedVerdict,
    ...overrides,
  }
}

function resultFor(taskId: string, overrides: Partial<EvalWorkerResult> = {}): EvalWorkerResult {
  return result({
    task_id: taskId,
    verdict: { ...satisfiedVerdict, task_id: taskId },
    ...overrides,
  })
}

function score(input: Partial<EvalRunInput> = {}) {
  return scoreEvalRun({
    tasks: [baseTask],
    worker_results: [result()],
    events: [],
    ...input,
  })
}

describe('scoreEvalRun', () => {
  it('scores a clean run with complete status-style summary data', () => {
    const scorecard = score({
      report: {
        waves: [['T1']],
        task_reports: [
          {
            task_id: 'T1',
            status: 'ok',
            files_changed: ['src/eval.ts'],
            verify_rc: 0,
            verifier_satisfied: true,
            out_of_bounds: [],
          },
        ],
      },
    })

    expect(scorecard.total_score).toBe(100)
    expect(scorecard.status).toBe('pass')
    expect(scorecard.findings).toEqual([])
    expect(scorecard.categories.boundary_compliance).toMatchObject({
      score: 100,
      status: 'pass',
      finding_count: 0,
    })
    expect(scorecard.summary).toEqual({
      status: 'pass',
      task_count: 1,
      worker_result_count: 1,
      report_task_count: 1,
      wave_count: 1,
      completed_count: 1,
      missing_worker_result_count: 0,
      failed_verify_count: 0,
      satisfied_verdict_count: 1,
      retry_count: 0,
      no_op_count: 0,
      out_of_bounds_count: 0,
      weak_verify_count: 0,
      lucky_pass_suspicion_count: 0,
    })
  })

  it('penalizes a weak verify pass when the task does not say what verify proves', () => {
    const scorecard = score({
      tasks: [task({ verify_proves: [] })],
    })

    expect(scorecard.categories.verify_relevance).toMatchObject({
      score: 40,
      status: 'fail',
      finding_count: 1,
    })
    expect(scorecard.findings).toContainEqual({
      category: 'verify_relevance',
      code: 'weak-verify-pass',
      severity: 'warning',
      task_id: 'T1',
      message: 'Verify passed without task-level evidence describing what it proves.',
      evidence: ['verify=npm test'],
    })
    expect(scorecard.summary.weak_verify_count).toBe(1)
    expect(scorecard.total_score).toBeLessThan(100)
  })

  it('penalizes retry-heavy lifecycle events without reading run directories', () => {
    const scorecard = score({
      events: [
        { type: 'worker_started', payload: { worker_id: 'worker-T1', task_id: 'T1', attempt: 1 } },
        {
          type: 'worker_restarted',
          payload: { worker_id: 'worker-T1', task_id: 'T1', attempt: 2, reason: 'stalled' },
        },
        {
          type: 'worker_restarted',
          payload: { worker_id: 'worker-T1', task_id: 'T1', attempt: 3, reason: 'stalled' },
        },
        {
          type: 'worker_restarted',
          payload: { worker_id: 'worker-T1', task_id: 'T1', attempt: 4, reason: 'loop' },
        },
      ],
    })

    expect(scorecard.categories.retries).toMatchObject({
      score: 0,
      status: 'fail',
      finding_count: 1,
    })
    expect(scorecard.findings).toContainEqual({
      category: 'retries',
      code: 'retry-heavy-run',
      severity: 'critical',
      message: 'Worker lifecycle required 3 retry attempt(s).',
      evidence: ['retry_count=3'],
    })
    expect(scorecard.summary.retry_count).toBe(3)
  })

  it('flags a no-op worker and lucky-pass suspicion when checks pass without edits', () => {
    const scorecard = score({
      worker_results: [result({ files_changed: [], status: 'no-op' })],
    })

    expect(scorecard.categories.no_op_rate).toMatchObject({
      score: 0,
      status: 'fail',
      finding_count: 1,
    })
    expect(scorecard.categories.lucky_pass_suspicion).toMatchObject({
      score: 0,
      status: 'fail',
      finding_count: 1,
    })
    expect(scorecard.findings).toContainEqual({
      category: 'no_op_rate',
      code: 'no-op-worker',
      severity: 'warning',
      task_id: 'T1',
      message: 'Worker reported success without changing files.',
      evidence: ['status=no-op'],
    })
    expect(scorecard.findings).toContainEqual({
      category: 'lucky_pass_suspicion',
      code: 'lucky-pass-no-op',
      severity: 'warning',
      task_id: 'T1',
      message: 'Verify and reviewer passed even though the worker changed no files.',
      evidence: ['verify_rc=0', 'verdict.satisfied=true'],
    })
    expect(scorecard.summary.no_op_count).toBe(1)
    expect(scorecard.summary.lucky_pass_suspicion_count).toBe(1)
  })

  it('flags derived boundary drift and reported out-of-bounds edits', () => {
    const scorecard = score({
      worker_results: [
        result({
          files_changed: ['src/eval.ts', 'src/other.ts'],
          out_of_bounds: ['src/other.ts'],
        }),
      ],
    })

    expect(scorecard.categories.boundary_compliance).toMatchObject({
      score: 0,
      status: 'fail',
      finding_count: 1,
    })
    expect(scorecard.categories.out_of_bounds_edits).toMatchObject({
      score: 0,
      status: 'fail',
      finding_count: 1,
    })
    expect(scorecard.findings).toContainEqual({
      category: 'boundary_compliance',
      code: 'boundary-drift',
      severity: 'critical',
      task_id: 'T1',
      message: 'Changed files were outside the task path boundary.',
      evidence: ['src/other.ts'],
    })
    expect(scorecard.findings).toContainEqual({
      category: 'out_of_bounds_edits',
      code: 'out-of-bounds-edit',
      severity: 'critical',
      task_id: 'T1',
      message: 'Run data reported out-of-bounds edits.',
      evidence: ['src/other.ts'],
    })
    expect(scorecard.summary.out_of_bounds_count).toBe(1)
  })

  it('flags a satisfied verifier verdict when the verify command failed', () => {
    const scorecard = score({
      worker_results: [result({ verify_rc: 1 })],
    })

    expect(scorecard.categories.lucky_pass_suspicion).toMatchObject({
      score: 0,
      status: 'fail',
      finding_count: 1,
    })
    expect(scorecard.findings).toContainEqual({
      category: 'lucky_pass_suspicion',
      code: 'satisfied-verdict-failed-verify',
      severity: 'critical',
      task_id: 'T1',
      message: 'Verifier was satisfied even though the verify command failed.',
      evidence: ['verify_rc=1', 'verdict.satisfied=true'],
    })
    expect(scorecard.summary.failed_verify_count).toBe(1)
    expect(scorecard.summary.satisfied_verdict_count).toBe(1)
  })

  it('flags a task that has no normalized worker result', () => {
    const scorecard = score({
      worker_results: [],
    })

    expect(scorecard.categories.result_completeness).toMatchObject({
      score: 0,
      status: 'fail',
      finding_count: 1,
    })
    expect(scorecard.findings).toContainEqual({
      category: 'result_completeness',
      code: 'missing-worker-result',
      severity: 'critical',
      task_id: 'T1',
      message: 'Task has no normalized worker result.',
      evidence: ['task_id=T1'],
    })
    expect(scorecard.summary.missing_worker_result_count).toBe(1)
    expect(scorecard.summary.completed_count).toBe(0)
  })

  it('scores a mixed realistic run predictably across every eval category', () => {
    const tasks = [
      task({ id: 'T1', paths: ['src/clean.ts'], title: 'Clean task' }),
      task({ id: 'T2', paths: ['src/noop.ts'], title: 'No-op task' }),
      task({ id: 'T3', paths: ['src/bounds.ts'], title: 'Boundary task' }),
      task({
        id: 'T4',
        paths: ['src/weak.ts'],
        title: 'Weak verify task',
        verify_proves: [],
      }),
      task({ id: 'T5', paths: ['src/failed.ts'], title: 'Failed verify task' }),
      task({ id: 'T6', paths: ['src/missing.ts'], title: 'Missing result task' }),
    ]
    const scorecard = scoreEvalRun({
      events: [
        { type: 'worker_started', payload: { worker_id: 'worker-T3', task_id: 'T3', attempt: 1 } },
        {
          type: 'worker_restarted',
          payload: { worker_id: 'worker-T3', task_id: 'T3', attempt: 2, reason: 'progress-stall' },
        },
        {
          type: 'worker_restarted',
          payload: { worker_id: 'worker-T3', task_id: 'T3', attempt: 3, reason: 'loop' },
        },
      ],
      report: {
        task_reports: tasks.map((runTask) => ({ task_id: runTask.id, status: 'ok' })),
        waves: [['T1', 'T2'], ['T3', 'T4'], ['T5', 'T6']],
      },
      tasks,
      worker_results: [
        resultFor('T1', { files_changed: ['src/clean.ts'] }),
        resultFor('T2', { files_changed: [], status: 'no-op' }),
        resultFor('T3', {
          files_changed: ['src/bounds.ts', 'docs/outside.md'],
          out_of_bounds: ['docs/outside.md'],
        }),
        resultFor('T4', { files_changed: ['src/weak.ts'] }),
        resultFor('T5', { files_changed: ['src/failed.ts'], verify_rc: 2 }),
      ],
    })

    expect(scorecard.status).toBe('warn')
    expect(scorecard.total_score).toBe(83)
    expect(scorecard.categories).toMatchObject({
      boundary_compliance: { finding_count: 1, score: 83, status: 'warn' },
      lucky_pass_suspicion: { finding_count: 2, score: 67, status: 'fail' },
      no_op_rate: { finding_count: 1, score: 83, status: 'warn' },
      out_of_bounds_edits: { finding_count: 1, score: 83, status: 'warn' },
      result_completeness: { finding_count: 1, score: 83, status: 'warn' },
      retries: { finding_count: 1, score: 83, status: 'warn' },
      verify_relevance: { finding_count: 1, score: 90, status: 'warn' },
    })
    expect(scorecard.summary).toMatchObject({
      completed_count: 5,
      failed_verify_count: 1,
      lucky_pass_suspicion_count: 2,
      missing_worker_result_count: 1,
      no_op_count: 1,
      out_of_bounds_count: 1,
      report_task_count: 6,
      retry_count: 2,
      satisfied_verdict_count: 5,
      task_count: 6,
      wave_count: 3,
      weak_verify_count: 1,
      worker_result_count: 5,
    })
  })
})
