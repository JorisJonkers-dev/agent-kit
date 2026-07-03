import { describe, expect, it, vi } from 'vitest'

import type { DagExecutorHooks, DagExecutorInput, DagExecutorResult } from '../ports/index.js'
import type { Task } from '../shared-kernel/index.js'

import { fanoutWorkflow, PreFanoutGateError } from './fanout.js'
import type { RunSummary } from './status.js'

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

function summary(tasks: readonly Task[]): RunSummary {
  return {
    run: 'run-1',
    state: {},
    tasks,
    waves: [],
    workerResults: [],
  }
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

describe('fanoutWorkflow', () => {
  it('returns an execution plan for a passing DAG', async () => {
    const createPullRequest = vi.fn<() => Promise<string>>().mockResolvedValue('https://github.test/pr/1')

    const plan = await fanoutWorkflow(
      {
        dryRun: false,
        github: true,
        repoFiles: new Set(['src/a.ts', 'src/b.ts']),
        runDir: 'runs/run-1',
      },
      {
        createPullRequest,
        status: () =>
          Promise.resolve(summary([
            task({ id: 'T1', objective: 'change a', paths: ['src/a.ts'], verify: 'npm test' }),
            task({
              depends_on: ['T1'],
              id: 'T2',
              objective: 'change b',
              paths: ['src/a.ts', 'src/b.ts'],
              verify: 'npm run typecheck',
            }),
          ])),
      },
    )

    expect(plan).toMatchObject({
      github: 'created',
      prUrl: 'https://github.test/pr/1',
      run: 'run-1',
      waves: [['T1'], ['T2']],
    })
    expect(plan).not.toHaveProperty('execution')
    expect(createPullRequest).toHaveBeenCalledWith('run-1')
  })

  it('executes a gated DAG from status with dry-run semantics when execute is true', async () => {
    const tasks = [
      task({ engine: { cli: 'codex', model: 'gpt-5' }, id: 'T1', objective: 'change a', paths: ['src/a.ts'], verify: 'npm test' }),
      task({
        depends_on: ['T1'],
        id: 'T2',
        objective: 'change b',
        paths: ['src/a.ts', 'src/b.ts'],
        verify: 'npm run typecheck',
      }),
    ] as const
    const executeDag = vi.fn((input: DagExecutorInput) => Promise.resolve(executionResult(input)))

    const plan = await fanoutWorkflow(
      {
        baseRef: 'main',
        concurrency: { max_parallel_tasks: 2 },
        dryRun: true,
        execute: true,
        github: false,
        hooks: hooks(),
        integrationBranch: 'council/run-1/integration',
        repoFiles: new Set(['src/a.ts', 'src/b.ts']),
        runDir: 'runs/run-1',
      },
      {
        createPullRequest: vi.fn<() => Promise<string>>(),
        executeDag,
        status: () => Promise.resolve(summary(tasks)),
      },
    )

    expect(plan).toMatchObject({
      execution: {
        dry_run: true,
        integration_branch: 'council/run-1/integration',
        run_id: 'run-1',
        status: 'dry-run',
      },
      github: 'disabled',
      run: 'run-1',
      waves: [['T1'], ['T2']],
    })
    expect(executeDag).toHaveBeenCalledTimes(1)
    expect(executeDag.mock.calls[0]?.[0]).toMatchObject({
      agent_pool: {
        assignments: [
          { agent_id: 'task:T1', model: 'haiku', task_id: 'T1' },
          { agent_id: 'task:T2', model: 'haiku', task_id: 'T2' },
        ],
      },
      base_ref: 'main',
      concurrency: { max_parallel_tasks: 2 },
      dry_run: true,
      integration_branch: 'council/run-1/integration',
      run_id: 'run-1',
      tasks,
    })
  })

  it('requires an executeDag dependency when execute is true', async () => {
    await expect(
      fanoutWorkflow(
        {
          baseRef: 'main',
          concurrency: { max_parallel_tasks: 1 },
          dryRun: true,
          execute: true,
          github: false,
          hooks: hooks(),
          integrationBranch: 'council/run-1/integration',
          repoFiles: new Set(['src/a.ts']),
          runDir: 'runs/run-1',
        },
        {
          createPullRequest: vi.fn<() => Promise<string>>(),
          status: () =>
            Promise.resolve(summary([
              task({ id: 'T1', objective: 'change a', paths: ['src/a.ts'], verify: 'npm test' }),
            ])),
        },
      ),
    ).rejects.toThrow('executeDag dependency is required when execute=true')
  })

  it('fails fast on same-wave overlap before creating a pull request', async () => {
    const createPullRequest = vi.fn<() => Promise<string>>()

    await expect(
      fanoutWorkflow(
        {
          dryRun: false,
          github: true,
          repoFiles: new Set(['council/ts/src/contexts/graph/adapters/process/session.ts']),
          runDir: 'runs/run-1',
        },
        {
          createPullRequest,
          status: () =>
            Promise.resolve(summary([
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

  it('surfaces pre-fanout gate errors before executing the DAG', async () => {
    const executeDag = vi.fn((input: DagExecutorInput) => Promise.resolve(executionResult(input)))

    await expect(
      fanoutWorkflow(
        {
          baseRef: 'main',
          concurrency: { max_parallel_tasks: 2 },
          dryRun: false,
          execute: true,
          github: false,
          hooks: hooks(),
          integrationBranch: 'council/run-1/integration',
          repoFiles: new Set(['src/collide.ts']),
          runDir: 'runs/run-1',
        },
        {
          createPullRequest: vi.fn<() => Promise<string>>(),
          executeDag,
          status: () =>
            Promise.resolve(summary([
              task({ id: 'T1', objective: 'first collision', paths: ['src/collide.ts'], verify: 'npm test' }),
              task({ id: 'T2', objective: 'second collision', paths: ['src/collide.ts'], verify: 'npm test' }),
            ])),
        },
      ),
    ).rejects.toBeInstanceOf(PreFanoutGateError)
    expect(executeDag).not.toHaveBeenCalled()
  })

  it('rejects missing injected repo files before creating a pull request', async () => {
    const createPullRequest = vi.fn<() => Promise<string>>()

    await expect(
      fanoutWorkflow(
        {
          dryRun: false,
          github: true,
          repoFiles: new Set(['src/present.ts']),
          runDir: 'runs/run-1',
        },
        {
          createPullRequest,
          status: () =>
            Promise.resolve(summary([
              task({ id: 'T1', objective: 'missing file', paths: ['src/missing.ts'], verify: 'npm test' }),
            ])),
        },
      ),
    ).rejects.toThrow('task T1 declares path src/missing.ts that is absent from the repo file set')
    expect(createPullRequest).not.toHaveBeenCalled()
  })

  it('surfaces weak verify findings before creating a pull request', async () => {
    const createPullRequest = vi.fn<() => Promise<string>>()

    await expect(
      fanoutWorkflow(
        {
          dryRun: false,
          github: true,
          repoFiles: new Set(['src/weak.ts']),
          runDir: 'runs/run-1',
        },
        {
          createPullRequest,
          status: () =>
            Promise.resolve(summary([
              task({ id: 'T1', objective: 'weak verify', paths: ['src/weak.ts'], verify: 'echo ok' }),
            ])),
        },
      ),
    ).rejects.toThrow('task T1 verify command does not prove the task result')
    expect(createPullRequest).not.toHaveBeenCalled()
  })

  it('surfaces destructive verify findings before returning a dry-run plan', async () => {
    await expect(
      fanoutWorkflow(
        {
          dryRun: true,
          github: true,
          repoFiles: new Set(['src/destructive.ts']),
          runDir: 'runs/run-1',
        },
        {
          createPullRequest: vi.fn<() => Promise<string>>(),
          status: () =>
            Promise.resolve(summary([
              task({
                id: 'T1',
                objective: 'destructive verify',
                paths: ['src/destructive.ts'],
                verify: 'git reset --hard',
              }),
            ])),
        },
      ),
    ).rejects.toThrow('task T1 verify command contains a destructive shell command')
  })

  it('surfaces absolute path findings before returning a dry-run plan', async () => {
    await expect(
      fanoutWorkflow(
        {
          dryRun: true,
          github: true,
          repoFiles: new Set(['src/present.ts']),
          runDir: 'runs/run-1',
        },
        {
          createPullRequest: vi.fn<() => Promise<string>>(),
          status: () =>
            Promise.resolve(summary([
              task({ id: 'T1', objective: 'absolute path', paths: ['/tmp/outside.ts'], verify: 'npm test' }),
            ])),
        },
      ),
    ).rejects.toThrow('task T1 declares absolute path /tmp/outside.ts')
  })
})
