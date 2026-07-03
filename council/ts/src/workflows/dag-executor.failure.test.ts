import { describe, expect, it, vi } from 'vitest'

import type {
  DagExecutorHooks,
  DagSuperviseInput,
  DagVerifyInput,
  GitCommitAllRequest,
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
    boundaries: 'Only touch workflow failure handling.',
    content_hash: `sha256:${id}`,
    depends_on: [],
    difficulty: 'moderate',
    id,
    model: 'sonnet',
    objective: `Handle ${id}`,
    output_format: 'Patch',
    paths: [`src/${id}.ts`],
    title: `Handle ${id}`,
    verify: `npx vitest run src/${id}.test.ts`,
    ...overrides,
  }
}

function workerResult(taskId: TaskId, status = 'succeeded'): WorkerResult {
  return {
    status,
    task_id: taskId,
  }
}

function hooksFor(input: {
  readonly workerFailures?: ReadonlySet<TaskId>
  readonly verifyFailures?: ReadonlySet<TaskId>
} = {}): DagExecutorHooks {
  return {
    provision: vi.fn(() => Promise.reject(new Error('legacy provision should not run'))),
    supervise: vi.fn((request: DagSuperviseInput) => {
      const failed = input.workerFailures?.has(request.task.id) === true
      const status = failed ? ('failed' as const) : ('succeeded' as const)
      return Promise.resolve({
        result: workerResult(request.task.id, status),
        status,
      })
    }),
    verify: vi.fn((request: DagVerifyInput) => {
      const failed = input.verifyFailures?.has(request.task.id) === true
      const status = failed ? ('failed' as const) : ('passed' as const)
      return Promise.resolve({
        command: request.command,
        exit_code: failed ? 1 : 0,
        output: failed ? `verify failed for ${request.task.id}` : `verify passed for ${request.task.id}`,
        status,
      })
    }),
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
      assignments: tasks.map((requestedTask) => ({
        agent_id: `agent-${requestedTask.id}`,
        model: requestedTask.model,
        task_id: requestedTask.id,
      })),
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

function resultByTask(result: { readonly task_results: readonly { readonly task_id: TaskId }[] }, taskId: TaskId) {
  const taskResult = result.task_results.find((candidate) => candidate.task_id === taskId)
  expect(taskResult).toBeDefined()
  if (taskResult === undefined) {
    throw new Error(`missing task result for ${taskId}`)
  }
  return taskResult
}

describe('executeDagExecutorState failure policy', () => {
  it('does not reconcile worker or verify failures and still cleans up their worktrees', async () => {
    const tasks = [task('T1'), task('T2')]
    const calls: string[] = []
    const execution = executionPorts({ calls })
    const hooks = hooksFor({
      verifyFailures: new Set(['T2']),
      workerFailures: new Set(['T1']),
    })

    const result = await executeDagExecutorState(inputFor(tasks, hooks, execution))

    expect(result.status).toBe('failed')
    expect(calls).not.toContain('changedFiles:/repo/.worktrees/workers/run-dag/T1')
    expect(calls).not.toContain('changedFiles:/repo/.worktrees/workers/run-dag/T2')
    expect(calls.some((call) => call.startsWith('commitAll:'))).toBe(false)
    expect(calls.some((call) => call.startsWith('reconcileIntegrationBranch:'))).toBe(false)
    expect(calls).toContain('removeWorktree:/repo/.worktrees/integration:/repo/.worktrees/workers/run-dag/T1')
    expect(calls).toContain('removeWorktree:/repo/.worktrees/integration:/repo/.worktrees/workers/run-dag/T2')
    expect(result.failed_tasks).toEqual([
      { error: 'worker reported failed', status: 'failed', task_id: 'T1' },
      { error: 'verify failed with exit code 1', status: 'failed', task_id: 'T2' },
    ])
    expect(resultByTask(result, 'T1')).toMatchObject({
      branch: 'worker/T1',
      status: 'failed',
      task_id: 'T1',
      worker_result: {
        branch: 'worker/T1',
        status: 'failed',
        task_id: 'T1',
        worktree: '/repo/.worktrees/workers/run-dag/T1',
      },
      worktree_path: '/repo/.worktrees/workers/run-dag/T1',
    })
    expect(resultByTask(result, 'T2')).toMatchObject({
      branch: 'worker/T2',
      status: 'failed',
      task_id: 'T2',
      verify: { exit_code: 1, status: 'failed' },
      worker_result: {
        branch: 'worker/T2',
        status: 'verify-failed',
        task_id: 'T2',
        verify_output: 'verify failed for T2',
        verify_rc: 1,
        worktree: '/repo/.worktrees/workers/run-dag/T2',
      },
      worktree_path: '/repo/.worktrees/workers/run-dag/T2',
    })
  })

  it('classifies a verified no-op as failed without committing or reconciling', async () => {
    const requestedTask = task('T1')
    const calls: string[] = []
    const commitRequests: GitCommitAllRequest[] = []
    const reconcileRequests: GitReconcileRequest[] = []
    const execution = executionPorts({
      calls,
      changedFilesByTask: new Map([['T1', []]]),
      commitRequests,
      reconcileRequests,
    })

    const result = await executeDagExecutorState(inputFor([requestedTask], hooksFor(), execution))

    expect(result.status).toBe('failed')
    expect(calls).toContain('changedFiles:/repo/.worktrees/workers/run-dag/T1')
    expect(commitRequests).toEqual([])
    expect(reconcileRequests).toEqual([])
    expect(calls).toContain('removeWorktree:/repo/.worktrees/integration:/repo/.worktrees/workers/run-dag/T1')
    expect(result.failed_tasks).toEqual([
      { error: 'no changes to commit', status: 'failed', task_id: 'T1' },
    ])
    expect(resultByTask(result, 'T1')).toMatchObject({
      error: 'no changes to commit',
      status: 'failed',
      task_id: 'T1',
      worker_result: {
        branch: 'worker/T1',
        committed: false,
        files_changed: [],
        out_of_bounds: [],
        status: 'no-op',
        task_id: 'T1',
        verify_output: 'verify passed for T1',
        verify_rc: 0,
        worktree: '/repo/.worktrees/workers/run-dag/T1',
      },
    })
  })

  it('marks only a conflicted merge failed, continues unrelated work, skips dependents, and persists evidence', async () => {
    const tasks = [
      task('T1'),
      task('T2'),
      task('T3', { depends_on: ['T1'] }),
    ]
    const calls: string[] = []
    const reconcileRequests: GitReconcileRequest[] = []
    const runStore = runStoreRecorder()
    const execution = executionPorts({
      calls,
      reconcileFailuresByBranch: new Map([['worker/T1', new Error('merge conflict in src/T1.ts')]]),
      reconcileRequests,
    })

    const result = await executeDagExecutorState(
      inputFor(tasks, hooksFor(), execution, {
        run_store: runStore,
      }),
    )

    expect(result.status).toBe('partial')
    expect(reconcileRequests.map(({ sourceBranch }) => sourceBranch)).toEqual(['worker/T1', 'worker/T2'])
    expect(result.failed_tasks).toEqual([
      { error: 'reconcile failed: merge conflict in src/T1.ts', status: 'failed', task_id: 'T1' },
    ])
    expect(result.skipped_tasks).toEqual([
      { dependency_task_id: 'T1', reason: 'dependency-failed', status: 'skipped', task_id: 'T3' },
    ])
    expect(resultByTask(result, 'T1')).toMatchObject({
      branch: 'worker/T1',
      commit: 'commit-T1',
      error: 'reconcile failed: merge conflict in src/T1.ts',
      files_changed: ['src/T1.ts'],
      status: 'failed',
      worker_result: {
        branch: 'worker/T1',
        committed: true,
        error: 'reconcile failed: merge conflict in src/T1.ts',
        files_changed: ['src/T1.ts'],
        merge: 'conflict',
        status: 'merge-conflict',
        task_id: 'T1',
        worktree: '/repo/.worktrees/workers/run-dag/T1',
      },
      worktree_path: '/repo/.worktrees/workers/run-dag/T1',
    })
    expect(resultByTask(result, 'T2')).toMatchObject({
      branch: 'worker/T2',
      commit: 'commit-T2',
      status: 'succeeded',
      task_id: 'T2',
    })
    expect(resultByTask(result, 'T3')).toMatchObject({
      skipped_reason: 'dependency-failed',
      status: 'skipped',
      task_id: 'T3',
      worker_result: {
        error: 'skipped because dependency T1 failed',
        status: 'skipped',
        task_id: 'T3',
      },
    })
    expect(calls).toContain('removeWorktree:/repo/.worktrees/integration:/repo/.worktrees/workers/run-dag/T1')
    expect(calls).toContain('removeWorktree:/repo/.worktrees/integration:/repo/.worktrees/workers/run-dag/T2')
    expect(calls).not.toContain('createWorktree:/repo/.worktrees/integration:worker/T3:/repo/.worktrees/workers/run-dag/T3')
    expect(runStore.workerResults.map(({ taskId, result }) => `${taskId}:${result.status}`)).toEqual([
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

function executionPorts(input: {
  readonly calls: string[]
  readonly changedFilesByTask?: ReadonlyMap<TaskId, readonly string[]>
  readonly commitRequests?: GitCommitAllRequest[]
  readonly reconcileFailuresByBranch?: ReadonlyMap<string, Error>
  readonly reconcileRequests?: GitReconcileRequest[]
}): DagExecutorExecutionPorts {
  return {
    dependency_provisioner: {
      provision(request) {
        input.calls.push(`provisionDependencies:${request.repoRoot}:${request.worktreePath}`)
        return Promise.resolve({ status: 'copied' })
      },
    },
    git: {
      changedFiles(cwd) {
        input.calls.push(`changedFiles:${cwd}`)
        return Promise.resolve(input.changedFilesByTask?.get(taskIdFromWorktree(cwd)) ?? [defaultChangedFile(cwd)])
      },
      commitAll(cwd, request) {
        input.calls.push(`commitAll:${cwd}`)
        input.commitRequests?.push(request)
        const taskId = taskIdFromWorktree(cwd)
        return Promise.resolve({
          branch: `worker/${taskId}`,
          commit: `commit-${taskId}`,
          files_changed: input.changedFilesByTask?.get(taskId) ?? [defaultChangedFile(cwd)],
          message: request.message,
        })
      },
      createWorktree(cwd, branch, path) {
        input.calls.push(`createWorktree:${cwd}:${branch}:${path}`)
        return Promise.resolve({ branch, path })
      },
      reconcileIntegrationBranch(cwd, request) {
        input.calls.push(`reconcileIntegrationBranch:${cwd}:${request.sourceBranch}`)
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
  if (taskId === undefined || taskId.length === 0) {
    throw new Error(`cannot infer task id from ${worktreePath}`)
  }
  return taskId as TaskId
}

function defaultChangedFile(worktreePath: string): string {
  return `src/${taskIdFromWorktree(worktreePath)}.ts`
}
