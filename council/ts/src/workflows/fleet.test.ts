import { describe, expect, it, vi } from 'vitest'

import type { DagExecutorHooks, DagExecutorInput, DagExecutorResult } from '../ports/index.js'
import type { Task } from '../shared-kernel/index.js'

import { PreFanoutGateError } from './fanout.js'
import { fleetWorkflow } from './fleet.js'

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

function tasksJson(tasks: readonly Task[]): string {
  return JSON.stringify(tasks)
}

function hooks(): DagExecutorHooks {
  return {
    provision: () => Promise.reject(new Error('not used by workflow tests')),
    supervise: () => Promise.reject(new Error('not used by workflow tests')),
    verify: () => Promise.reject(new Error('not used by workflow tests')),
  }
}

function executionResult(input: DagExecutorInput): DagExecutorResult {
  return {
    base_ref: input.base_ref,
    dry_run: input.dry_run,
    failed_tasks: [],
    integration_branch: input.integration_branch,
    run_id: input.run_id,
    skipped_tasks: [],
    status: input.dry_run ? 'dry-run' : 'succeeded',
    task_results: input.tasks.map((plannedTask) => {
      const assignment = input.agent_pool.assignments.find((candidate) => candidate.task_id === plannedTask.id)
      return {
        ...(assignment !== undefined ? { assignment } : {}),
        status: input.dry_run ? 'skipped' : 'succeeded',
        task_id: plannedTask.id,
      }
    }),
  }
}

describe('fleetWorkflow', () => {
  it('returns an execution plan for a passing DAG', async () => {
    const createPullRequest = vi.fn<() => Promise<string>>().mockResolvedValue('https://github.test/pr/2')

    await expect(
      fleetWorkflow(
        {
          agents: 'codex:gpt-5*2',
          dryRun: false,
          github: true,
          repoFiles: new Set(['src/a.ts', 'src/b.ts']),
          tasksPath: 'runs/run-2.json',
        },
        {
          createPullRequest,
          readText: () =>
            Promise.resolve(tasksJson([
              task({ id: 'T1', objective: 'change a', paths: ['src/a.ts'], verify: 'npm test' }),
              task({
                depends_on: ['T1'],
                id: 'T2',
                objective: 'change b',
                paths: ['src/a.ts', 'src/b.ts'],
                verify: 'npm run lint',
              }),
            ])),
        },
      ),
    ).resolves.toMatchObject({
      agents: { T1: 'codex:gpt-5', T2: 'codex:gpt-5' },
      github: 'created',
      prUrl: 'https://github.test/pr/2',
      run: 'run-2',
      waves: [['T1'], ['T2']],
    })
    expect(createPullRequest).toHaveBeenCalledWith('run-2')
  })

  it('executes a gated DAG with parsed fleet agent assignments', async () => {
    const tasks = [
      task({ id: 'T1', model: 'sonnet', objective: 'change a', paths: ['src/a.ts'], verify: 'npm test' }),
      task({
        id: 'T2',
        model: 'opus',
        objective: 'change b',
        paths: ['src/b.ts'],
        verify: 'npm run typecheck',
      }),
      task({ id: 'T3', objective: 'change c', paths: ['src/c.ts'], verify: 'npm run lint' }),
    ] as const
    const executeDag = vi.fn((input: DagExecutorInput) => Promise.resolve(executionResult(input)))

    const plan = await fleetWorkflow(
      {
        agents: 'codex:gpt-5*2,claude:haiku',
        baseRef: 'main',
        concurrency: { max_parallel_tasks: 3, per_agent: { 'codex:gpt-5#1': 1 } },
        dryRun: false,
        execute: true,
        github: false,
        hooks: hooks(),
        integrationBranch: 'council/run-2/integration',
        repoFiles: new Set(['src/a.ts', 'src/b.ts', 'src/c.ts']),
        tasksPath: 'runs/run-2.json',
      },
      {
        createPullRequest: vi.fn<() => Promise<string>>(),
        executeDag,
        readText: () => Promise.resolve(tasksJson(tasks)),
      },
    )

    expect(plan).toMatchObject({
      agents: { T1: 'codex:gpt-5', T2: 'codex:gpt-5', T3: 'claude:haiku' },
      execution: {
        dry_run: false,
        integration_branch: 'council/run-2/integration',
        run_id: 'run-2',
        status: 'succeeded',
      },
      github: 'disabled',
      run: 'run-2',
      waves: [['T1', 'T2', 'T3']],
    })
    expect(executeDag).toHaveBeenCalledTimes(1)
    expect(executeDag.mock.calls[0]?.[0]).toMatchObject({
      agent_pool: {
        assignments: [
          {
            agent_id: 'codex:gpt-5#1',
            metadata: { cli: 'codex', label: 'codex:gpt-5', model: 'gpt-5' },
            model: 'sonnet',
            task_id: 'T1',
          },
          {
            agent_id: 'codex:gpt-5#2',
            metadata: { cli: 'codex', label: 'codex:gpt-5', model: 'gpt-5' },
            model: 'opus',
            task_id: 'T2',
          },
          {
            agent_id: 'claude:haiku#3',
            metadata: { cli: 'claude', label: 'claude:haiku', model: 'haiku' },
            model: 'haiku',
            task_id: 'T3',
          },
        ],
        available: [
          { id: 'codex:gpt-5#1', kind: 'codex', metadata: { model: 'gpt-5' } },
          { id: 'codex:gpt-5#2', kind: 'codex', metadata: { model: 'gpt-5' } },
          { id: 'claude:haiku#3', kind: 'claude', metadata: { model: 'haiku' } },
        ],
      },
      base_ref: 'main',
      concurrency: { max_parallel_tasks: 3, per_agent: { 'codex:gpt-5#1': 1 } },
      dry_run: false,
      integration_branch: 'council/run-2/integration',
      run_id: 'run-2',
      tasks,
    })
  })

  it('surfaces pre-fanout gate errors before executing the DAG', async () => {
    const executeDag = vi.fn((input: DagExecutorInput) => Promise.resolve(executionResult(input)))

    await expect(
      fleetWorkflow(
        {
          agents: 'claude:haiku',
          baseRef: 'main',
          concurrency: { max_parallel_tasks: 1 },
          dryRun: false,
          execute: true,
          github: false,
          hooks: hooks(),
          integrationBranch: 'council/run-2/integration',
          repoFiles: new Set(['src/collide.ts']),
          tasksPath: 'runs/run-2.json',
        },
        {
          createPullRequest: vi.fn<() => Promise<string>>(),
          executeDag,
          readText: () =>
            Promise.resolve(tasksJson([
              task({ id: 'T1', objective: 'first collision', paths: ['src/collide.ts'], verify: 'npm test' }),
              task({ id: 'T2', objective: 'second collision', paths: ['src/collide.ts'], verify: 'npm test' }),
            ])),
        },
      ),
    ).rejects.toBeInstanceOf(PreFanoutGateError)
    expect(executeDag).not.toHaveBeenCalled()
  })

  it('validates loaded tasks before assigning agents or creating GitHub output', async () => {
    const createPullRequest = vi.fn<() => Promise<string>>()

    await expect(
      fleetWorkflow(
        {
          agents: '',
          dryRun: false,
          github: true,
          repoFiles: new Set(['src/destructive.ts']),
          tasksPath: 'runs/run-2.json',
        },
        {
          createPullRequest,
          readText: () =>
            Promise.resolve(tasksJson([
              task({
                id: 'T1',
                objective: 'destructive verify',
                paths: ['src/destructive.ts'],
                verify: 'rm -rf dist',
              }),
            ])),
        },
      ),
    ).rejects.toThrow('task T1 verify command contains a destructive shell command')
    expect(createPullRequest).not.toHaveBeenCalled()
  })

  it('rejects same-wave overlap before assigning agents or creating GitHub output', async () => {
    const createPullRequest = vi.fn<() => Promise<string>>()

    await expect(
      fleetWorkflow(
        {
          agents: '',
          dryRun: false,
          github: true,
          repoFiles: new Set(['council/ts/src/contexts/graph/adapters/process/session.ts']),
          tasksPath: 'runs/run-2.json',
        },
        {
          createPullRequest,
          readText: () =>
            Promise.resolve(tasksJson([
              task({
                id: 'T1',
                objective: 'first session change',
                paths: ['council/ts/src/contexts/graph/adapters/process/session.ts'],
                verify: 'npm test',
              }),
              task({
                id: 'T2',
                objective: 'second session change',
                paths: ['council/ts/src/contexts/graph/adapters/process/session.ts'],
                verify: 'npm test',
              }),
            ])),
        },
      ),
    ).rejects.toMatchObject({
      violations: [
        {
          kind: 'same-wave-path-overlap',
          otherPath: 'council/ts/src/contexts/graph/adapters/process/session.ts',
          otherTaskId: 'T2',
          path: 'council/ts/src/contexts/graph/adapters/process/session.ts',
          taskId: 'T1',
          wave: 0,
        },
      ],
    })
    expect(createPullRequest).not.toHaveBeenCalled()
  })

  it('surfaces missing declared file findings before returning an execution plan', async () => {
    await expect(
      fleetWorkflow(
        {
          agents: 'claude:sonnet',
          dryRun: true,
          github: false,
          repoFiles: new Set(['src/present.ts']),
          tasksPath: 'runs/run-2.json',
        },
        {
          createPullRequest: vi.fn<() => Promise<string>>(),
          readText: () =>
            Promise.resolve(tasksJson([
              task({ id: 'T1', objective: 'missing file', paths: ['src/missing.ts'], verify: 'npm test' }),
            ])),
        },
      ),
    ).rejects.toThrow('task T1 declares path src/missing.ts that is absent from the repo file set')
  })

  it('surfaces weak verify and absolute path findings before returning an execution plan', async () => {
    await expect(
      fleetWorkflow(
        {
          agents: 'claude:sonnet',
          dryRun: true,
          github: false,
          repoFiles: new Set(['src/weak.ts']),
          tasksPath: 'runs/run-2.json',
        },
        {
          createPullRequest: vi.fn<() => Promise<string>>(),
          readText: () =>
            Promise.resolve(tasksJson([
              task({ id: 'T1', objective: 'weak verify', paths: ['src/weak.ts'], verify: 'echo ok' }),
              task({ id: 'T2', objective: 'absolute path', paths: ['/tmp/outside.ts'], verify: 'npm test' }),
            ])),
        },
      ),
    ).rejects.toMatchObject({
      violations: [
        {
          kind: 'non-proving-verify',
          taskId: 'T1',
          verify: 'echo ok',
        },
        {
          kind: 'absolute-task-path',
          path: '/tmp/outside.ts',
          taskId: 'T2',
        },
      ],
    })
  })
})
