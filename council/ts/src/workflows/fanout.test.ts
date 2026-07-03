import { describe, expect, it, vi } from 'vitest'

import type { Task } from '../shared-kernel/index.js'

import { fanoutWorkflow } from './fanout.js'
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

describe('fanoutWorkflow', () => {
  it('returns an execution plan for a passing DAG', async () => {
    const createPullRequest = vi.fn<() => Promise<string>>().mockResolvedValue('https://github.test/pr/1')

    await expect(
      fanoutWorkflow(
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
      ),
    ).resolves.toMatchObject({
      github: 'created',
      prUrl: 'https://github.test/pr/1',
      run: 'run-1',
      waves: [['T1'], ['T2']],
    })
    expect(createPullRequest).toHaveBeenCalledWith('run-1')
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
