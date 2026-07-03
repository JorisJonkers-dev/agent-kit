import { describe, expect, it, vi } from 'vitest'

import type {
  DagAgentAssignment,
  DagExecutorHooks,
  DagSuperviseInput,
  DagVerifyInput,
  GitReconcileRequest,
  RunStorePort,
  WorkerResult,
} from '../ports/index.js'
import type { Task, TaskId } from '../shared-kernel/index.js'

import type { DagExecutorExecutionPorts, DagExecutorStateInput } from './dag-executor.js'
import { executeDagExecutorState } from './dag-executor.js'

interface WorkerResultWrite {
  readonly result: WorkerResult
  readonly runId: string
  readonly taskId: string
}

interface WorkerEventWrite {
  readonly event: Parameters<RunStorePort['appendWorkerEvent']>[1]
  readonly runId: string
}

function task(id: TaskId, overrides: Partial<Task> = {}): Task {
  return {
    boundaries: 'Only touch executor result shaping.',
    content_hash: `sha256:${id}`,
    depends_on: [],
    difficulty: 'moderate',
    id,
    model: 'haiku',
    objective: `Finalize ${id}`,
    output_format: 'patch',
    paths: [`src/${id}.ts`],
    title: `Finalize ${id}`,
    verify: `npx vitest run src/${id}.test.ts`,
    ...overrides,
  }
}

function assignmentFor(requestedTask: Task): DagAgentAssignment {
  return {
    agent_id: `agent-${requestedTask.id}`,
    model: requestedTask.model,
    reason: `assigned ${requestedTask.id}`,
    task_id: requestedTask.id,
  }
}

function workerResult(taskId: TaskId, status = 'succeeded'): WorkerResult {
  return {
    files_changed: [`src/${taskId}.ts`],
    status,
    task_id: taskId,
    verify_rc: status === 'succeeded' ? 0 : 1,
  }
}

function hooksFor(): DagExecutorHooks {
  return {
    provision: vi.fn(() => Promise.reject(new Error('legacy provision should not run'))),
    supervise: vi.fn((request: DagSuperviseInput) =>
      Promise.resolve({ result: workerResult(request.task.id), status: 'succeeded' as const }),
    ),
    verify: vi.fn((request: DagVerifyInput) =>
      Promise.resolve({
        command: request.command,
        exit_code: 0,
        output: `verified ${request.task.id}`,
        status: 'passed' as const,
      }),
    ),
  }
}

function inputFor(
  tasks: readonly Task[],
  hooks: DagExecutorHooks,
  execution: DagExecutorExecutionPorts,
  overrides: Partial<DagExecutorStateInput> = {},
): DagExecutorStateInput {
  return {
    agent_pool: {
      assignments: tasks.map(assignmentFor),
      available: tasks.map((requestedTask) => ({
        id: `agent-${requestedTask.id}`,
        kind: 'codex',
        model: requestedTask.model,
      })),
    },
    base_ref: 'main',
    concurrency: { max_parallel_tasks: 2 },
    dry_run: false,
    execution,
    hooks,
    integration_branch: 'integration/dag',
    repoFiles: new Set(tasks.flatMap((requestedTask) => requestedTask.paths)),
    run_id: 'run-dag',
    tasks,
    ...overrides,
  }
}

function executionPorts(input: {
  readonly calls: string[]
  readonly reconcileFailuresByBranch?: ReadonlyMap<string, Error>
  readonly reconcileRequests?: GitReconcileRequest[]
}): DagExecutorExecutionPorts {
  return {
    dependency_provisioner: {
      provision(request) {
        input.calls.push(`provision:${request.repoRoot}:${request.worktreePath}`)
        return Promise.resolve({ status: 'copied' })
      },
    },
    git: {
      changedFiles(cwd) {
        input.calls.push(`changedFiles:${cwd}`)
        return Promise.resolve([`src/${taskIdFromWorktree(cwd)}.ts`])
      },
      commitAll(cwd) {
        input.calls.push(`commitAll:${cwd}`)
        const taskId = taskIdFromWorktree(cwd)
        return Promise.resolve({
          branch: `worker/${taskId}`,
          commit: `commit-${taskId}`,
          files_changed: [`src/${taskId}.ts`],
          message: `${taskId} Finalize ${taskId}`,
        })
      },
      createWorktree(cwd, branch, path) {
        input.calls.push(`createWorktree:${cwd}:${branch}:${path}`)
        return Promise.resolve({ branch, path })
      },
      reconcileIntegrationBranch(cwd, request) {
        input.calls.push(`reconcile:${cwd}:${request.sourceBranch}`)
        input.reconcileRequests?.push(request)
        const failure = input.reconcileFailuresByBranch?.get(request.sourceBranch)
        if (failure !== undefined) {
          return Promise.reject(failure)
        }
        return Promise.resolve({
          head: `integration-${request.sourceBranch}`,
          integrationBranch: request.integrationBranch,
          sourceBranch: request.sourceBranch,
        })
      },
      removeWorktree(cwd, path) {
        input.calls.push(`removeWorktree:${cwd}:${path}`)
        return Promise.resolve(undefined)
      },
    },
    integration_worktree_path: '/repo/.worktrees/integration',
    repo_root: '/repo',
    worktree_root: '/repo/.worktrees/workers/run-dag',
  }
}

function runStoreRecorder(): Pick<RunStorePort, 'appendWorkerEvent' | 'writeWorkerResult'> & {
  readonly workerEvents: WorkerEventWrite[]
  readonly workerResults: WorkerResultWrite[]
} {
  const workerEvents: WorkerEventWrite[] = []
  const workerResults: WorkerResultWrite[] = []
  return {
    appendWorkerEvent(runId, event) {
      workerEvents.push({ event, runId })
      return Promise.resolve(undefined)
    },
    workerEvents,
    workerResults,
    writeWorkerResult(runId, taskId, result) {
      workerResults.push({ result, runId, taskId })
      return Promise.resolve(undefined)
    },
  }
}

function taskIdFromWorktree(worktreePath: string): TaskId {
  const taskId = worktreePath.split('/').pop()
  expect(taskId).toBeDefined()
  if (taskId === undefined || taskId.length === 0) {
    throw new Error(`cannot infer task id from ${worktreePath}`)
  }
  return taskId as TaskId
}

describe('executeDagExecutorState result shape', () => {
  it('returns a deterministic dry-run plan without invoking executor side-effect ports', async () => {
    const tasks = [
      task('T1'),
      task('T2', { depends_on: ['T1'] }),
    ]
    const calls: string[] = []
    const hooks = hooksFor()

    const result = await executeDagExecutorState(
      inputFor(tasks, hooks, executionPorts({ calls }), {
        dry_run: true,
      }),
    )

    expect(result.status).toBe('dry-run')
    expect(result.pre_fanout_gate).toMatchObject({
      ok: true,
      violations: [],
      waves: [['T1'], ['T2']],
    })
    expect(result.waves).toEqual([['T1'], ['T2']])
    expect(result.assignments).toEqual(tasks.map(assignmentFor))
    expect(result.planned_branches).toEqual([
      { branch: 'worker/T1', task_id: 'T1', worktree_path: '/repo/.worktrees/workers/run-dag/T1' },
      { branch: 'worker/T2', task_id: 'T2', worktree_path: '/repo/.worktrees/workers/run-dag/T2' },
    ])
    expect(result.status_counts).toEqual({
      blocked: 0,
      failed: 0,
      pending: 0,
      running: 0,
      skipped: 2,
      succeeded: 0,
      total: 2,
    })
    expect(result.human_summary).toBe('dry-run: 0 succeeded, 0 failed, 2 skipped, 0 blocked, 0 pending, 0 running')
    expect(result.task_results).toEqual([
      { skipped_reason: 'dry-run', status: 'skipped', task_id: 'T1' },
      { skipped_reason: 'dry-run', status: 'skipped', task_id: 'T2' },
    ])
    expect(calls).toEqual([])
    expect(hooks.provision).not.toHaveBeenCalled()
    expect(hooks.supervise).not.toHaveBeenCalled()
    expect(hooks.verify).not.toHaveBeenCalled()
  })

  it('includes stable machine-readable result JSON for successful execution', async () => {
    const requestedTask = task('T1')
    const calls: string[] = []

    const result = await executeDagExecutorState(inputFor([requestedTask], hooksFor(), executionPorts({ calls })))

    expect(result.status).toBe('succeeded')
    expect(
      JSON.parse(
        JSON.stringify({
          assignments: result.assignments,
          base_ref: result.base_ref,
          human_summary: result.human_summary,
          integration_branch: result.integration_branch,
          planned_branches: result.planned_branches,
          run_id: result.run_id,
          status: result.status,
          status_counts: result.status_counts,
          task_results: result.task_results,
          waves: result.waves,
        }),
      ),
    ).toEqual({
      assignments: [
        {
          agent_id: 'agent-T1',
          model: 'haiku',
          reason: 'assigned T1',
          task_id: 'T1',
        },
      ],
      base_ref: 'main',
      human_summary: 'succeeded: 1 succeeded, 0 failed, 0 skipped, 0 blocked, 0 pending, 0 running',
      integration_branch: 'integration/dag',
      planned_branches: [{ branch: 'worker/T1', task_id: 'T1', worktree_path: '/repo/.worktrees/workers/run-dag/T1' }],
      run_id: 'run-dag',
      status: 'succeeded',
      status_counts: {
        blocked: 0,
        failed: 0,
        pending: 0,
        running: 0,
        skipped: 0,
        succeeded: 1,
        total: 1,
      },
      task_results: [
        {
          assignment: {
            agent_id: 'agent-T1',
            model: 'haiku',
            reason: 'assigned T1',
            task_id: 'T1',
          },
          branch: 'worker/T1',
          commit: 'commit-T1',
          files_changed: ['src/T1.ts'],
          status: 'succeeded',
          task_id: 'T1',
          verify: {
            command: 'npx vitest run src/T1.test.ts',
            exit_code: 0,
            output: 'verified T1',
            status: 'passed',
          },
          worker_result: {
            branch: 'worker/T1',
            committed: true,
            files_changed: ['src/T1.ts'],
            out_of_bounds: [],
            status: 'succeeded',
            task_id: 'T1',
            verify_output: 'verified T1',
            verify_rc: 0,
            worktree: '/repo/.worktrees/workers/run-dag/T1',
          },
          worktree_path: '/repo/.worktrees/workers/run-dag/T1',
        },
      ],
      waves: [['T1']],
    })
  })

  it('persists merge-failed and dependency-skipped terminal worker artifacts and events', async () => {
    const tasks = [
      task('T1'),
      task('T2'),
      task('T3', { depends_on: ['T1'] }),
    ]
    const calls: string[] = []
    const runStore = runStoreRecorder()

    const result = await executeDagExecutorState(
      inputFor(tasks, hooksFor(), executionPorts({
        calls,
        reconcileFailuresByBranch: new Map([['worker/T1', new Error('merge conflict in src/T1.ts')]]),
      }), {
        run_store: runStore,
      }),
    )

    expect(result.status).toBe('partial')
    expect(result.status_counts).toEqual({
      blocked: 0,
      failed: 1,
      pending: 0,
      running: 0,
      skipped: 1,
      succeeded: 1,
      total: 3,
    })
    expect(result.human_summary).toBe('partial: 1 succeeded, 1 failed, 1 skipped, 0 blocked, 0 pending, 0 running')
    expect(result.task_results).toMatchObject([
      {
        branch: 'worker/T1',
        commit: 'commit-T1',
        error: 'reconcile failed: merge conflict in src/T1.ts',
        status: 'failed',
        task_id: 'T1',
        worker_result: {
          error: 'reconcile failed: merge conflict in src/T1.ts',
          merge: 'conflict',
          status: 'merge-conflict',
        },
      },
      { branch: 'worker/T2', commit: 'commit-T2', status: 'succeeded', task_id: 'T2' },
      {
        skipped_reason: 'dependency-failed',
        status: 'skipped',
        task_id: 'T3',
        worker_result: {
          error: 'skipped because dependency T1 failed',
          status: 'skipped',
          task_id: 'T3',
        },
      },
    ])
    expect(runStore.workerResults.map(({ taskId, result: worker }) => `${taskId}:${worker.status}`)).toEqual([
      'T1:merge-conflict',
      'T2:succeeded',
      'T3:skipped',
    ])
    expect(runStore.workerEvents.map(({ event }) => event)).toEqual([
      {
        payload: {
          result_path: 'workers/T1/result.json',
          status: 'merge-conflict',
          task_id: 'T1',
          worker_id: 'native-dag:T1',
        },
        type: 'worker_finished',
      },
      {
        payload: {
          result_path: 'workers/T2/result.json',
          status: 'succeeded',
          task_id: 'T2',
          worker_id: 'native-dag:T2',
        },
        type: 'worker_finished',
      },
      {
        payload: {
          result_path: 'workers/T3/result.json',
          status: 'skipped',
          task_id: 'T3',
          worker_id: 'native-dag:T3',
        },
        type: 'worker_finished',
      },
    ])
  })
})
