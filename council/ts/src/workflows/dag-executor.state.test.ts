import { describe, expect, it, vi } from 'vitest'

import type {
  DagAgentAssignment,
  DagExecutorHooks,
  DagExecutorInput,
  DagTaskResult,
  WorkerResult,
} from '../ports/index.js'
import type { Task, TaskId } from '../shared-kernel/index.js'

import { executeDagExecutorState } from './dag-executor.js'

function task(input: Partial<Task> & Pick<Task, 'id' | 'objective' | 'paths' | 'verify'>): Task {
  const { content_hash, id, objective, paths, verify, ...overrides } = input
  return {
    boundaries: 'same package',
    content_hash: content_hash ?? id,
    depends_on: [],
    difficulty: 'moderate',
    id,
    model: 'haiku',
    objective,
    output_format: 'patch',
    paths,
    title: objective,
    verify,
    ...overrides,
  }
}

function assignment(taskId: TaskId): DagAgentAssignment {
  return {
    agent_id: `agent-${taskId}`,
    model: 'haiku',
    task_id: taskId,
  }
}

function workerResult(taskId: TaskId, status: string): WorkerResult {
  return {
    files_changed: [`src/${taskId}.ts`],
    status,
    task_id: taskId,
    verify_rc: status === 'succeeded' ? 0 : 1,
  }
}

function baseInput(
  tasks: readonly Task[],
  hooks: DagExecutorHooks,
  overrides: Partial<Omit<DagExecutorInput, 'hooks' | 'tasks'>> = {},
): DagExecutorInput {
  return {
    agent_pool: {
      assignments: tasks.map(({ id }) => assignment(id)),
      available: [{ id: 'agent-haiku', kind: 'codex', model: 'haiku' }],
    },
    base_ref: 'main',
    concurrency: {
      max_parallel_tasks: 2,
    },
    dry_run: false,
    integration_branch: 'integration/dag',
    run_id: 'run-dag',
    tasks,
    ...overrides,
    hooks,
  }
}

function repoFilesFor(tasks: readonly Task[]): ReadonlySet<string> {
  return new Set(tasks.flatMap(({ paths }) => paths))
}

function resultByTask(results: readonly DagTaskResult[], taskId: TaskId): DagTaskResult {
  const result = results.find(({ task_id }) => task_id === taskId)
  expect(result).toBeDefined()
  if (result === undefined) {
    throw new Error(`missing result for ${taskId}`)
  }
  return result
}

describe('executeDagExecutorState', () => {
  it('returns pre-fanout blocking findings before dispatching or mutating ports', async () => {
    const calls: string[] = []
    const tasks = [
      task({ id: 'T1', objective: 'first shared edit', paths: ['src/shared.ts'], verify: 'npm test' }),
      task({ id: 'T2', objective: 'second shared edit', paths: ['src/shared.ts'], verify: 'npm test' }),
    ]
    const hooks: DagExecutorHooks = {
      provision: vi.fn(() => {
        calls.push('provision')
        return Promise.resolve({ status: 'provisioned' as const })
      }),
      supervise: vi.fn(() => {
        calls.push('supervise')
        return Promise.resolve({ result: workerResult('T1', 'succeeded'), status: 'succeeded' as const })
      }),
      verify: vi.fn(() => {
        calls.push('verify')
        return Promise.resolve({ command: 'npm test', exit_code: 0, status: 'passed' as const })
      }),
    }

    const result = await executeDagExecutorState({
      ...baseInput(tasks, hooks),
      repoFiles: repoFilesFor(tasks),
    })

    expect(result.status).toBe('failed')
    expect(result.pre_fanout_gate).toMatchObject({
      ok: false,
      violations: [{ kind: 'same-wave-path-overlap', taskId: 'T1' }],
      waves: [['T1', 'T2']],
    })
    expect(result.state.dispatched).toEqual([])
    expect(result.state.task_states).toEqual([
      { blocked_by: [], graph_state: 'pending', status: 'pending', task_id: 'T1' },
      { blocked_by: [], graph_state: 'pending', status: 'pending', task_id: 'T2' },
    ])
    expect(result.task_results).toEqual([])
    expect(calls).toEqual([])
    expect(hooks.provision).not.toHaveBeenCalled()
    expect(hooks.supervise).not.toHaveBeenCalled()
    expect(hooks.verify).not.toHaveBeenCalled()
  })

  it('dispatches ready tasks with the concurrency cap, critical path ordering, and stable equal-priority order', async () => {
    const tasks = [
      task({ id: 'T1', objective: 'long root', paths: ['src/T1.ts'], verify: 'npm test' }),
      task({ id: 'T2', objective: 'first short root', paths: ['src/T2.ts'], verify: 'npm test' }),
      task({ depends_on: ['T1'], id: 'T3', objective: 'middle', paths: ['src/T3.ts'], verify: 'npm test' }),
      task({ depends_on: ['T3'], id: 'T4', objective: 'leaf', paths: ['src/T4.ts'], verify: 'npm test' }),
      task({ id: 'T5', objective: 'second short root', paths: ['src/T5.ts'], verify: 'npm test' }),
    ]
    const supervised: TaskId[] = []
    const verified: TaskId[] = []
    let active = 0
    let maxActive = 0
    const hooks: DagExecutorHooks = {
      provision: ({ assignment: taskAssignment, task: requested }) =>
        Promise.resolve({
          assignment: taskAssignment,
          branch: `worker/${requested.id}`,
          status: 'provisioned',
          worktree_path: `/tmp/${requested.id}`,
        }),
      async supervise({ task: requested }) {
        active += 1
        maxActive = Math.max(maxActive, active)
        supervised.push(requested.id)
        await Promise.resolve()
        active -= 1
        return { result: workerResult(requested.id, 'succeeded'), status: 'succeeded' }
      },
      verify: ({ command, task: requested }) => {
        verified.push(requested.id)
        return Promise.resolve({ command, exit_code: 0, status: 'passed' })
      },
    }

    const result = await executeDagExecutorState({
      ...baseInput(tasks, hooks),
      repoFiles: repoFilesFor(tasks),
    })

    expect(result.status).toBe('succeeded')
    expect(result.pre_fanout_gate.ok).toBe(true)
    expect(maxActive).toBe(2)
    expect(supervised).toEqual(['T1', 'T2', 'T3', 'T5', 'T4'])
    expect(verified).toEqual(supervised)
    expect(result.state.dispatched).toEqual(supervised)
    expect(result.state.task_states).toEqual([
      { blocked_by: [], graph_state: 'closed', status: 'succeeded', task_id: 'T1' },
      { blocked_by: [], graph_state: 'closed', status: 'succeeded', task_id: 'T2' },
      { blocked_by: [], graph_state: 'closed', status: 'succeeded', task_id: 'T3' },
      { blocked_by: [], graph_state: 'closed', status: 'succeeded', task_id: 'T4' },
      { blocked_by: [], graph_state: 'closed', status: 'succeeded', task_id: 'T5' },
    ])
    expect(result.task_results.map(({ task_id }) => task_id)).toEqual(supervised)
  })

  it('records failed and blocked transitions while skipping dependents via graph propagation', async () => {
    const tasks = [
      task({ id: 'T1', objective: 'independent root', paths: ['src/T1.ts'], verify: 'npm test' }),
      task({ id: 'T2', objective: 'failing root', paths: ['src/T2.ts'], verify: 'npm test' }),
      task({ depends_on: ['T2'], id: 'T3', objective: 'direct dependent', paths: ['src/T3.ts'], verify: 'npm test' }),
      task({ depends_on: ['T3'], id: 'T4', objective: 'transitive dependent', paths: ['src/T4.ts'], verify: 'npm test' }),
      task({ depends_on: ['T1'], id: 'T5', objective: 'still runnable', paths: ['src/T5.ts'], verify: 'npm test' }),
    ]
    const supervised: TaskId[] = []
    const verified: TaskId[] = []
    const hooks: DagExecutorHooks = {
      provision: ({ task: requested }) =>
        Promise.resolve({
          branch: `worker/${requested.id}`,
          status: 'provisioned',
          worktree_path: `/tmp/${requested.id}`,
        }),
      supervise: ({ task: requested }) => {
        supervised.push(requested.id)
        const status = requested.id === 'T2' ? 'failed' : 'succeeded'
        return Promise.resolve({ result: workerResult(requested.id, status), status })
      },
      verify: ({ command, task: requested }) => {
        verified.push(requested.id)
        return Promise.resolve({ command, exit_code: 0, status: 'passed' })
      },
    }

    const result = await executeDagExecutorState({
      ...baseInput(tasks, hooks),
      repoFiles: repoFilesFor(tasks),
    })

    expect(result.status).toBe('partial')
    expect(supervised).toEqual(['T2', 'T1', 'T5'])
    expect(verified).toEqual(['T1', 'T5'])
    expect(result.failed_tasks).toEqual([{ error: 'worker reported failed', status: 'failed', task_id: 'T2' }])
    expect(result.skipped_tasks).toEqual([
      { dependency_task_id: 'T2', reason: 'dependency-failed', status: 'skipped', task_id: 'T3' },
      { dependency_task_id: 'T3', reason: 'dependency-skipped', status: 'skipped', task_id: 'T4' },
    ])
    expect(resultByTask(result.task_results, 'T2')).toMatchObject({
      error: 'worker reported failed',
      status: 'failed',
      task_id: 'T2',
    })
    expect(resultByTask(result.task_results, 'T3')).toMatchObject({
      skipped_reason: 'dependency-failed',
      status: 'skipped',
      task_id: 'T3',
    })
    expect(result.state.task_states).toEqual([
      { blocked_by: [], graph_state: 'closed', status: 'succeeded', task_id: 'T1' },
      { blocked_by: [], graph_state: 'stalled', status: 'failed', task_id: 'T2' },
      { blocked_by: ['T2'], graph_state: 'blocked', status: 'skipped', task_id: 'T3' },
      { blocked_by: ['T3'], graph_state: 'blocked', status: 'skipped', task_id: 'T4' },
      { blocked_by: [], graph_state: 'closed', status: 'succeeded', task_id: 'T5' },
    ])
  })

  it('records provision and verify failures as failed executor state without direct side effects', async () => {
    const tasks = [
      task({ id: 'T1', objective: 'cannot provision', paths: ['src/T1.ts'], verify: 'npm test' }),
      task({ id: 'T2', objective: 'cannot verify', paths: ['src/T2.ts'], verify: 'npm test' }),
    ]
    const hooks: DagExecutorHooks = {
      provision: ({ task: requested }) =>
        Promise.resolve(
          requested.id === 'T1'
            ? { error: 'worktree failed', status: 'failed' }
            : {
                branch: `worker/${requested.id}`,
                status: 'provisioned',
                worktree_path: `/tmp/${requested.id}`,
              },
        ),
      supervise: ({ task: requested }) =>
        Promise.resolve({ result: workerResult(requested.id, 'succeeded'), status: 'succeeded' }),
      verify: ({ command }) =>
        Promise.resolve({ command, exit_code: 1, output: 'type error', status: 'failed' }),
    }

    const result = await executeDagExecutorState({
      ...baseInput(tasks, hooks),
      repoFiles: repoFilesFor(tasks),
    })

    expect(result.status).toBe('failed')
    expect(result.failed_tasks).toEqual([
      { error: 'worktree failed', status: 'failed', task_id: 'T1' },
      { error: 'verify failed with exit code 1', status: 'failed', task_id: 'T2' },
    ])
    expect(result.state.task_states).toEqual([
      { blocked_by: [], graph_state: 'stalled', status: 'failed', task_id: 'T1' },
      { blocked_by: [], graph_state: 'stalled', status: 'failed', task_id: 'T2' },
    ])
  })

  it('returns a dry-run state without calling executor ports', async () => {
    const tasks = [
      task({ id: 'T1', objective: 'dry root', paths: ['src/T1.ts'], verify: 'npm test' }),
      task({ depends_on: ['T1'], id: 'T2', objective: 'dry child', paths: ['src/T2.ts'], verify: 'npm test' }),
    ]
    const hooks: DagExecutorHooks = {
      provision: vi.fn(() => Promise.resolve({ status: 'provisioned' as const })),
      supervise: vi.fn(() =>
        Promise.resolve({ result: workerResult('T1', 'succeeded'), status: 'succeeded' as const }),
      ),
      verify: vi.fn(() => Promise.resolve({ command: 'npm test', exit_code: 0, status: 'passed' as const })),
    }

    const result = await executeDagExecutorState({
      ...baseInput(tasks, hooks, { dry_run: true }),
      repoFiles: repoFilesFor(tasks),
    })

    expect(result.status).toBe('dry-run')
    expect(result.task_results).toEqual([
      { skipped_reason: 'dry-run', status: 'skipped', task_id: 'T1' },
      { skipped_reason: 'dry-run', status: 'skipped', task_id: 'T2' },
    ])
    expect(result.skipped_tasks).toEqual([
      { reason: 'dry-run', status: 'skipped', task_id: 'T1' },
      { reason: 'dry-run', status: 'skipped', task_id: 'T2' },
    ])
    expect(result.state.dispatched).toEqual([])
    expect(result.state.task_states).toEqual([
      { blocked_by: [], graph_state: 'pending', status: 'skipped', task_id: 'T1' },
      { blocked_by: [], graph_state: 'pending', status: 'skipped', task_id: 'T2' },
    ])
    expect(hooks.provision).not.toHaveBeenCalled()
    expect(hooks.supervise).not.toHaveBeenCalled()
    expect(hooks.verify).not.toHaveBeenCalled()
  })
})
