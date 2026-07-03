import { describe, expect, it } from 'vitest'

import type {
  DagAgentAssignment,
  DagAgentPool,
  DagExecutorInput,
  DagExecutorPort,
  DagProvisionResult,
  DagTaskResult,
  DagVerifyResult,
  GitDagExecutorPort,
  RunStorePort,
  WorkerResult,
} from './index.js'
import type { Task } from '../shared-kernel/index.js'

const baseTask: Task = {
  id: 'T1',
  title: 'Define executor ports',
  objective: 'Add native DAG executor contracts.',
  output_format: 'Pure TypeScript ports',
  paths: ['council/ts/src/ports/dag-executor.ts'],
  depends_on: [],
  difficulty: 'moderate',
  model: 'sonnet',
  verify: 'npm run typecheck',
  boundaries: 'Ports only',
  acceptance_criteria: ['Exports the executor contract'],
}

const dependentTask: Task = {
  ...baseTask,
  id: 'T2',
  title: 'Verify barrel exports',
  depends_on: ['T1'],
  model: 'haiku',
}

describe('DAG executor ports', () => {
  it('defines the native executor input and machine-readable result contract', async () => {
    const assignment: DagAgentAssignment = {
      agent_id: 'codex-1',
      model: 'sonnet',
      reason: 'Owns strict TypeScript ports.',
      task_id: 'T1',
    }

    const agents: DagAgentPool = {
      available: [
        {
          id: 'codex-1',
          kind: 'codex',
          model: 'sonnet',
          max_concurrency: 1,
          labels: ['typescript'],
        },
        {
          id: 'claude-1',
          kind: 'claude',
          model: 'haiku',
          max_concurrency: 2,
        },
      ],
      assignments: [assignment],
    }

    const input: DagExecutorInput = {
      run_id: 'run-native-dag',
      base_ref: 'main',
      integration_branch: 'integration/native-dag',
      tasks: [baseTask, dependentTask],
      agent_pool: agents,
      concurrency: {
        max_parallel_tasks: 2,
      },
      dry_run: false,
      eval: {
        enabled: true,
        require_clean_boundaries: true,
      },
      hooks: {
        provision: (request) =>
          Promise.resolve({
            assignment: request.assignment,
            branch: `worker/${request.task.id}`,
            status: 'provisioned',
            worktree_path: `/tmp/${request.task.id}`,
          }),
        supervise: (request) =>
          Promise.resolve({
            result: workerResult(request.task.id, 'succeeded'),
            status: 'succeeded',
          }),
        verify: (request) =>
          Promise.resolve({
            command: request.command,
            exit_code: 0,
            output: 'ok',
            status: 'passed',
          }),
      },
    }
    const hookResults: readonly [DagProvisionResult, DagVerifyResult] = [
      {
        status: 'dry-run',
      },
      {
        command: 'npm test',
        exit_code: null,
        status: 'skipped',
      },
    ]

    const executor: DagExecutorPort = {
      async execute(request) {
        const provisioned = await request.hooks.provision({
          assignment,
          base_ref: request.base_ref,
          integration_branch: request.integration_branch,
          run_id: request.run_id,
          task: baseTask,
        })
        const branch = provisioned.branch ?? 'worker/T1'
        const worktreePath = provisioned.worktree_path ?? '/tmp/T1'
        const verified = await request.hooks.verify({
          assignment,
          command: baseTask.verify,
          run_id: request.run_id,
          task: baseTask,
          worktree_path: worktreePath,
        })
        const taskResult: DagTaskResult = {
          assignment,
          branch,
          files_changed: ['council/ts/src/ports/dag-executor.ts'],
          status: 'succeeded',
          task_id: 'T1',
          verify: verified,
          worktree_path: worktreePath,
        }

        return {
          base_ref: request.base_ref,
          dry_run: request.dry_run,
          eval: {
            status: 'passed',
          },
          failed_tasks: [],
          integration_branch: request.integration_branch,
          run_id: request.run_id,
          skipped_tasks: [
            {
              reason: 'dependency-failed',
              status: 'skipped',
              task_id: 'T2',
            },
          ],
          status: 'partial',
          task_results: [taskResult],
        }
      },
    }

    await expect(executor.execute(input)).resolves.toMatchObject({
      base_ref: 'main',
      dry_run: false,
      eval: {
        status: 'passed',
      },
      integration_branch: 'integration/native-dag',
      run_id: 'run-native-dag',
      skipped_tasks: [{ task_id: 'T2' }],
      status: 'partial',
      task_results: [{ task_id: 'T1', verify: { status: 'passed' } }],
    })
    expect(hookResults).toHaveLength(2)
  })

  it('widens Git and run-store capabilities needed by executor adapters', async () => {
    const git: GitDagExecutorPort = {
      changedFiles() {
        return Promise.resolve(['council/ts/src/ports/git.ts'])
      },
      commitAll(_cwd, request) {
        return Promise.resolve({
          branch: 'worker/T1',
          commit: 'abc123',
          files_changed: ['council/ts/src/ports/git.ts'],
          message: request.message,
        })
      },
      createWorktree(_cwd, branch, path) {
        return Promise.resolve({
          branch,
          path,
        })
      },
      currentBranch() {
        return Promise.resolve('worker/T1')
      },
      reconcileIntegrationBranch(_cwd, request) {
        return Promise.resolve({
          head: 'def456',
          integrationBranch: request.integrationBranch,
          sourceBranch: request.sourceBranch,
        })
      },
      removeWorktree() {
        return Promise.resolve(undefined)
      },
      root() {
        return Promise.resolve('/repo')
      },
    }

    const events: string[] = []
    const runStore: Pick<RunStorePort, 'appendWorkerEvent' | 'writeWorkerResult'> = {
      appendWorkerEvent(_runId, event) {
        events.push(event.type)
        return Promise.resolve(undefined)
      },
      writeWorkerResult(_runId, taskId, result) {
        events.push(`${taskId}:${result.status}`)
        return Promise.resolve(undefined)
      },
    }

    const committed = await git.commitAll('/repo', {
      message: 'T1 define executor ports',
    })
    await runStore.writeWorkerResult('run-native-dag', 'T1', workerResult('T1', 'failed'))
    await runStore.appendWorkerEvent('run-native-dag', {
      payload: {
        content_hash: 'sha256:skip',
        finished_at: '2026-07-03T00:00:00.000Z',
        status: 'skipped',
        task_id: 'T2',
        worker_id: 'native-dag:T2',
      },
      type: 'worker_finished',
    })

    expect(committed).toEqual({
      branch: 'worker/T1',
      commit: 'abc123',
      files_changed: ['council/ts/src/ports/git.ts'],
      message: 'T1 define executor ports',
    })
    expect(events).toEqual(['T1:failed', 'worker_finished'])
  })
})

function workerResult(taskId: string, status: WorkerResult['status']): WorkerResult {
  return {
    files_changed: ['council/ts/src/ports/dag-executor.ts'],
    status,
    task_id: taskId,
    verify_rc: status === 'succeeded' ? 0 : 1,
  }
}
