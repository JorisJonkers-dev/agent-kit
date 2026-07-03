import { describe, expect, it, vi } from 'vitest'

import type {
  DagExecutorHooks,
  DagSuperviseInput,
  DagVerifyInput,
  GitCommitAllRequest,
  GitReconcileRequest,
  WorkerResult,
} from '../ports/index.js'
import type { Task } from '../shared-kernel/index.js'

import type { DagExecutorExecutionPorts, DagExecutorStateInput } from './dag-executor.js'
import { executeDagExecutorState } from './dag-executor.js'

function task(overrides: Partial<Task> = {}): Task {
  return {
    boundaries: 'Only touch the executor workflow.',
    content_hash: 'sha256:T6',
    depends_on: [],
    difficulty: 'moderate',
    id: 'T6',
    model: 'sonnet',
    objective: 'Execute successful task path.',
    output_format: 'Patch',
    paths: ['council/ts/src/workflows/dag-executor.ts'],
    title: 'Execute successful task path',
    verify: 'npx vitest run src/workflows/dag-executor.execution.test.ts',
    ...overrides,
  }
}

function executionInput(
  requestedTask: Task,
  hooks: DagExecutorHooks,
  execution: DagExecutorExecutionPorts,
): DagExecutorStateInput {
  return {
    agent_pool: {
      assignments: [
        {
          agent_id: 'codex-sonnet',
          model: requestedTask.model,
          task_id: requestedTask.id,
        },
      ],
      available: [{ id: 'codex-sonnet', kind: 'codex', model: requestedTask.model }],
    },
    base_ref: 'main',
    concurrency: { max_parallel_tasks: 1 },
    dry_run: false,
    execution,
    hooks,
    integration_branch: 'integration/dag',
    repoFiles: new Set(requestedTask.paths),
    run_id: 'run-dag',
    tasks: [requestedTask],
  }
}

function workerResult(requestedTask: Task): WorkerResult {
  return {
    status: 'succeeded',
    task_id: requestedTask.id,
  }
}

describe('executeDagExecutorState execution path', () => {
  it('creates an isolated worktree, verifies, gates changed files, commits, reconciles, and cleans up', async () => {
    const requestedTask = task()
    const calls: string[] = []
    const changedFiles = [
      'council/ts/src/workflows/dag-executor.ts',
      'workers/T6/story.md',
    ]
    const commitRequests: GitCommitAllRequest[] = []
    const reconcileRequests: GitReconcileRequest[] = []
    const execution = executionPorts({
      calls,
      changedFiles,
      commitRequests,
      reconcileRequests,
    })
    const hooks: DagExecutorHooks = {
      provision: vi.fn(() => Promise.reject(new Error('legacy provision should not run'))),
      supervise: vi.fn((request: DagSuperviseInput) => {
        calls.push(`supervise:${request.branch}:${request.worktree_path}`)
        return Promise.resolve({
          result: workerResult(requestedTask),
          status: 'succeeded' as const,
        })
      }),
      verify: vi.fn((request: DagVerifyInput) => {
        calls.push(`verify:${request.command}:${request.worktree_path}`)
        return Promise.resolve({
          command: request.command,
          exit_code: 0,
          output: 'all good',
          status: 'passed' as const,
        })
      }),
    }

    const result = await executeDagExecutorState(executionInput(requestedTask, hooks, execution))

    expect(result.status).toBe('succeeded')
    expect(calls).toEqual([
      'createWorktree:/repo/.worktrees/integration:worker/T6:/repo/.worktrees/workers/run-dag/T6',
      'provisionDependencies:/repo:/repo/.worktrees/workers/run-dag/T6',
      'supervise:worker/T6:/repo/.worktrees/workers/run-dag/T6',
      'verify:npx vitest run src/workflows/dag-executor.execution.test.ts:/repo/.worktrees/workers/run-dag/T6',
      'changedFiles:/repo/.worktrees/workers/run-dag/T6',
      'commitAll:/repo/.worktrees/workers/run-dag/T6',
      'reconcileIntegrationBranch:/repo/.worktrees/integration',
      'removeWorktree:/repo/.worktrees/integration:/repo/.worktrees/workers/run-dag/T6',
    ])
    expect(hooks.provision).not.toHaveBeenCalled()
    expect(commitRequests).toEqual([{ message: 'T6 Execute successful task path' }])
    expect(reconcileRequests).toEqual([
      {
        baseBranch: 'main',
        integrationBranch: 'integration/dag',
        sourceBranch: 'worker/T6',
      },
    ])

    const taskResult = onlyTaskResult(result.task_results)
    expect(taskResult).toMatchObject({
      branch: 'worker/T6',
      commit: 'commit-T6',
      files_changed: changedFiles,
      status: 'succeeded',
      task_id: 'T6',
      verify: {
        command: 'npx vitest run src/workflows/dag-executor.execution.test.ts',
        exit_code: 0,
        output: 'all good',
        status: 'passed',
      },
      worktree_path: '/repo/.worktrees/workers/run-dag/T6',
    })
    expect(taskResult.worker_result).toEqual({
      branch: 'worker/T6',
      committed: true,
      files_changed: changedFiles,
      out_of_bounds: [],
      status: 'succeeded',
      task_id: 'T6',
      verify_output: 'all good',
      verify_rc: 0,
      worktree: '/repo/.worktrees/workers/run-dag/T6',
    })
  })

  it('applies the bounds gate before commit and still removes the worktree when files are out of bounds', async () => {
    const requestedTask = task()
    const calls: string[] = []
    const changedFiles = [
      'council/ts/src/workflows/dag-executor.ts',
      'docs/outside.md',
    ]
    const commitRequests: GitCommitAllRequest[] = []
    const reconcileRequests: GitReconcileRequest[] = []
    const execution = executionPorts({
      calls,
      changedFiles,
      commitRequests,
      reconcileRequests,
    })
    const hooks: DagExecutorHooks = {
      provision: vi.fn(() => Promise.reject(new Error('legacy provision should not run'))),
      supervise: vi.fn((request: DagSuperviseInput) => {
        calls.push(`supervise:${request.branch}:${request.worktree_path}`)
        return Promise.resolve({
          result: workerResult(requestedTask),
          status: 'succeeded' as const,
        })
      }),
      verify: vi.fn((request: DagVerifyInput) => {
        calls.push(`verify:${request.command}:${request.worktree_path}`)
        return Promise.resolve({
          command: request.command,
          exit_code: 0,
          output: 'all good',
          status: 'passed' as const,
        })
      }),
    }

    const result = await executeDagExecutorState(executionInput(requestedTask, hooks, execution))

    expect(result.status).toBe('failed')
    expect(calls).toEqual([
      'createWorktree:/repo/.worktrees/integration:worker/T6:/repo/.worktrees/workers/run-dag/T6',
      'provisionDependencies:/repo:/repo/.worktrees/workers/run-dag/T6',
      'supervise:worker/T6:/repo/.worktrees/workers/run-dag/T6',
      'verify:npx vitest run src/workflows/dag-executor.execution.test.ts:/repo/.worktrees/workers/run-dag/T6',
      'changedFiles:/repo/.worktrees/workers/run-dag/T6',
      'removeWorktree:/repo/.worktrees/integration:/repo/.worktrees/workers/run-dag/T6',
    ])
    expect(commitRequests).toEqual([])
    expect(reconcileRequests).toEqual([])
    expect(result.failed_tasks).toEqual([
      { error: 'bounds gate reported out-of-bounds', status: 'failed', task_id: 'T6' },
    ])

    const taskResult = onlyTaskResult(result.task_results)
    expect(taskResult.worker_result).toEqual({
      branch: 'worker/T6',
      committed: false,
      files_changed: changedFiles,
      out_of_bounds: ['docs/outside.md'],
      status: 'out-of-bounds',
      task_id: 'T6',
      verify_output: 'all good',
      verify_rc: 0,
      worktree: '/repo/.worktrees/workers/run-dag/T6',
    })
  })
})

function executionPorts(input: {
  readonly calls: string[]
  readonly changedFiles: readonly string[]
  readonly commitRequests: GitCommitAllRequest[]
  readonly reconcileRequests: GitReconcileRequest[]
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
        return Promise.resolve(input.changedFiles)
      },
      commitAll(cwd, request) {
        input.calls.push(`commitAll:${cwd}`)
        input.commitRequests.push(request)
        return Promise.resolve({
          branch: 'worker/T6',
          commit: 'commit-T6',
          files_changed: input.changedFiles,
          message: request.message,
        })
      },
      createWorktree(cwd, branch, path) {
        input.calls.push(`createWorktree:${cwd}:${branch}:${path}`)
        return Promise.resolve({ branch, path })
      },
      reconcileIntegrationBranch(cwd, request) {
        input.calls.push(`reconcileIntegrationBranch:${cwd}`)
        input.reconcileRequests.push(request)
        return Promise.resolve({
          head: 'integration-head',
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

function onlyTaskResult<T>(items: readonly T[]): T {
  expect(items).toHaveLength(1)
  const item = items[0]
  expect(item).toBeDefined()
  if (item === undefined) {
    throw new Error('missing task result')
  }
  return item
}
