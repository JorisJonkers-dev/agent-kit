import { describe, expect, it, vi } from 'vitest'

import type { Task } from '../shared-kernel/index.js'

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
