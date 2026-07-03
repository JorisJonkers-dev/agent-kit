import { describe, expect, it } from 'vitest'

import type { RunStoreEvent } from '../contexts/runstore/index.js'
import type { LegacyRunReport, WorkerResult } from '../ports/index.js'
import type { ReviewVerdict, RunState, Task } from '../shared-kernel/index.js'
import { evalWorkflow, type EvalWorkflowDeps } from './eval.js'
import type { RunSummary } from './status.js'

const baseState: RunState = {
  stage: 'fanout',
}

const baseTask: Task = {
  id: 'T1',
  title: 'Eval workflow',
  objective: 'Score normalized run artifacts',
  output_format: 'TypeScript workflow',
  paths: ['council/ts/src/workflows/eval.ts'],
  depends_on: [],
  difficulty: 'moderate',
  model: 'sonnet',
  verify: 'npx vitest run src/workflows/eval.test.ts',
  verify_proves: ['workflow orchestration passes normalized run artifacts to the eval domain'],
  boundaries: 'Only eval workflow files',
  acceptance_criteria: ['score normalized worker results'],
}

const satisfiedVerdict: ReviewVerdict = {
  engine: { cli: 'codex', model: 'gpt-5' },
  issues: [],
  reasons: 'complete',
  satisfied: true,
  task_id: 'T1',
}

function task(overrides: Partial<Task> = {}): Task {
  return { ...baseTask, ...overrides }
}

function workerResult(overrides: Partial<WorkerResult> = {}): WorkerResult {
  return {
    files_changed: ['council/ts/src/workflows/eval.ts'],
    out_of_bounds: [],
    status: 'ok',
    task_id: 'T1',
    verdict: satisfiedVerdict,
    verify_rc: 0,
    ...overrides,
  }
}

function report(overrides: Partial<LegacyRunReport> = {}): LegacyRunReport {
  return {
    run: 'run-a',
    tasks: [
      {
        files_changed: ['council/ts/src/workflows/eval.ts'],
        out_of_bounds: [],
        status: 'ok',
        task_id: 'T1',
        verifier_satisfied: true,
        verify_rc: 0,
      },
    ],
    waves: [['T1']],
    ...overrides,
  }
}

function runSummary(overrides: Partial<RunSummary> = {}): RunSummary {
  return {
    run: 'run-a',
    state: baseState,
    tasks: [baseTask],
    waves: [['T1']],
    workerResults: [workerResult()],
    ...overrides,
  }
}

function depsFor(summary: RunSummary, events: readonly RunStoreEvent[] = []): EvalWorkflowDeps {
  return {
    readEvents(runId) {
      expect(runId).toBe(summary.run)
      return Promise.resolve(events)
    },
    status(input) {
      expect(input).toEqual({ runDir: '/runs/run-a' })
      return Promise.resolve(summary)
    },
  }
}

describe('evalWorkflow', () => {
  it('scores injected normalized worker results without filesystem coupling', async () => {
    const result = await evalWorkflow(
      { runDir: '/runs/run-a' },
      depsFor(
        runSummary({
          workerResults: [workerResult({ files_changed: [], status: 'no-op' })],
        }),
      ),
    )

    expect(result.run).toBe('run-a')
    expect(result.score).toBe(75)
    expect(result.status).toBe('warn')
    expect(result.categories.no_op_rate).toMatchObject({
      finding_count: 1,
      score: 0,
      status: 'fail',
    })
    expect(result.findings).toContainEqual({
      category: 'no_op_rate',
      code: 'no-op-worker',
      evidence: ['status=no-op'],
      message: 'Worker reported success without changing files.',
      severity: 'warning',
      task_id: 'T1',
    })
    expect(result.summary).toMatchObject({
      completed_count: 1,
      critical_finding_count: 0,
      failed_verify_count: 0,
      finding_count: 2,
      lucky_pass_suspicion_count: 1,
      no_op_count: 1,
      retry_count: 0,
      satisfied_verdict_count: 1,
      status: 'warn',
      task_count: 1,
      weak_verify_count: 0,
      warning_finding_count: 2,
      worker_result_count: 1,
    })
  })

  it('passes legacy report waves and task reports into the eval domain', async () => {
    const secondTask = task({
      id: 'T2',
      paths: ['council/ts/src/workflows/eval.test.ts'],
      title: 'Eval workflow tests',
    })
    const result = await evalWorkflow(
      { runDir: '/runs/run-a' },
      depsFor(
        runSummary({
          report: report({
            tasks: [
              {
                files_changed: ['council/ts/src/workflows/eval.ts'],
                status: 'ok',
                task_id: 'T1',
                verifier_satisfied: true,
                verify_rc: 0,
              },
              {
                files_changed: ['council/ts/src/workflows/eval.test.ts'],
                status: 'ok',
                task_id: 'T2',
                verifier_satisfied: true,
                verify_rc: 0,
              },
            ],
            waves: [['T1'], ['T2']],
          }),
          tasks: [baseTask, secondTask],
          waves: [['T1'], ['T2']],
          workerResults: [
            workerResult(),
            workerResult({
              files_changed: ['council/ts/src/workflows/eval.test.ts'],
              task_id: 'T2',
              verdict: { ...satisfiedVerdict, task_id: 'T2' },
            }),
          ],
        }),
      ),
    )

    expect(result.score).toBe(100)
    expect(result.summary).toMatchObject({
      report_task_count: 2,
      wave_count: 2,
      worker_result_count: 2,
    })
    expect(result.findings).toEqual([])
  })

  it('passes worker lifecycle events from runstore into retry scoring', async () => {
    const result = await evalWorkflow(
      { runDir: '/runs/run-a' },
      depsFor(runSummary(), [
        {
          payload: {
            engine: { cli: 'codex', model: 'gpt-5' },
            issues: [],
            reasons: 'unrelated event',
            satisfied: true,
            task_id: 'T1',
          },
          type: 'review_verdict',
        },
        {
          payload: { attempt: 1, task_id: 'T1', worker_id: 'worker-T1' },
          type: 'worker_started',
        },
        {
          payload: { attempt: 2, reason: 'stalled', task_id: 'T1', worker_id: 'worker-T1' },
          type: 'worker_restarted',
        },
        {
          payload: { attempt: 3, reason: 'loop', task_id: 'T1', worker_id: 'worker-T1' },
          type: 'worker_restarted',
        },
      ]),
    )

    expect(result.categories.retries).toMatchObject({
      finding_count: 1,
      score: 0,
      status: 'fail',
    })
    expect(result.findings).toContainEqual({
      category: 'retries',
      code: 'retry-heavy-run',
      evidence: ['retry_count=2'],
      message: 'Worker lifecycle required 2 retry attempt(s).',
      severity: 'critical',
    })
    expect(result.summary).toMatchObject({
      critical_finding_count: 1,
      finding_count: 1,
      retry_count: 2,
      warning_finding_count: 0,
    })
  })
})
