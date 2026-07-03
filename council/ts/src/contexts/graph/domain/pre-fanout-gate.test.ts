import { describe, expect, it } from 'vitest'

import {
  applyPreFanoutGate,
  createTaskGraph,
  type GraphTaskDraft,
  type PreFanoutGateViolationKind,
} from './index.js'

function task(
  overrides: Partial<GraphTaskDraft> & Pick<GraphTaskDraft, 'id' | 'objective'>,
): GraphTaskDraft {
  return {
    boundaries: 'same package',
    difficulty: 'moderate',
    model: 'haiku',
    output_format: 'patch',
    paths: ['src/default.ts'],
    title: overrides.objective,
    verify: 'npm test',
    ...overrides,
  }
}

function repoFiles(...paths: readonly string[]): ReadonlySet<string> {
  return new Set(paths)
}

describe('pre-fanout gate', () => {
  it('rejects the P0-style parallel merge-conflict hazard within one ready wave', () => {
    const graph = createTaskGraph([
      task({
        id: 'T1',
        objective: 'wire fanout supervision',
        paths: ['council/ts/src/contexts/graph/adapters/process/session.ts'],
      }),
      task({
        id: 'T2',
        objective: 'emit session diagnostics',
        paths: ['council/ts/src/contexts/graph/adapters/process/session.ts'],
      }),
    ])

    expect(
      applyPreFanoutGate({
        graph,
        repoFiles: repoFiles('council/ts/src/contexts/graph/adapters/process/session.ts'),
      }),
    ).toEqual({
      ok: false,
      violations: [
        {
          kind: 'same-wave-path-overlap',
          message:
            'tasks T1 and T2 both declare council/ts/src/contexts/graph/adapters/process/session.ts in ready wave 0',
          otherPath: 'council/ts/src/contexts/graph/adapters/process/session.ts',
          otherTaskId: 'T2',
          path: 'council/ts/src/contexts/graph/adapters/process/session.ts',
          taskId: 'T1',
          wave: 0,
        },
      ],
      waves: [['T1', 'T2']],
    })
  })

  it('allows overlapping paths when dependencies serialize the tasks into separate waves', () => {
    const graph = createTaskGraph([
      task({ id: 'T1', objective: 'add shared helper', paths: ['src/shared.ts'] }),
      task({
        depends_on: ['T1'],
        id: 'T2',
        objective: 'use shared helper',
        paths: ['src/shared.ts'],
      }),
    ])

    expect(applyPreFanoutGate({ graph, repoFiles: repoFiles('src/shared.ts') })).toEqual({
      ok: true,
      violations: [],
      waves: [['T1'], ['T2']],
    })
  })

  it('rejects empty, placeholder, non-proving, and destructive verify commands', () => {
    const graph = createTaskGraph([
      task({ id: 'T1', objective: 'empty verify', paths: ['src/empty.ts'], verify: '  ' }),
      task({
        id: 'T2',
        objective: 'placeholder verify',
        paths: ['src/placeholder.ts'],
        verify: '<verify>',
      }),
      task({
        id: 'T3',
        objective: 'non-proving verify',
        paths: ['src/echo.ts'],
        verify: 'echo ok',
      }),
      task({
        id: 'T4',
        objective: 'destructive verify',
        paths: ['src/destructive.ts'],
        verify: 'rm -rf dist',
      }),
    ])

    const result = applyPreFanoutGate({
      graph,
      repoFiles: repoFiles(
        'src/empty.ts',
        'src/placeholder.ts',
        'src/echo.ts',
        'src/destructive.ts',
      ),
    })

    expect(result.ok).toBe(false)
    expect(result.waves).toEqual([['T1', 'T2', 'T3', 'T4']])
    expect(result.violations.map((violation) => violation.kind)).toEqual([
      'empty-verify',
      'placeholder-verify',
      'non-proving-verify',
      'destructive-command',
    ] satisfies readonly PreFanoutGateViolationKind[])
    expect(result.violations.map((violation) => violation.taskId)).toEqual(['T1', 'T2', 'T3', 'T4'])
    expect(result.violations.map((violation) => violation.verify)).toEqual([
      '  ',
      '<verify>',
      'echo ok',
      'rm -rf dist',
    ])
  })

  it('rejects absolute paths and declared files missing from the injected repo-file set', () => {
    const graph = createTaskGraph([
      task({
        id: 'T1',
        objective: 'bad paths',
        paths: [
          '/tmp/outside.ts',
          String.raw`C:\repo\outside.ts`,
          'src/missing.ts',
          'src/present.ts',
        ],
      }),
    ])

    expect(applyPreFanoutGate({ graph, repoFiles: repoFiles('src/present.ts') })).toEqual({
      ok: false,
      violations: [
        {
          kind: 'absolute-task-path',
          message: 'task T1 declares absolute path /tmp/outside.ts',
          path: '/tmp/outside.ts',
          taskId: 'T1',
        },
        {
          kind: 'absolute-task-path',
          message: String.raw`task T1 declares absolute path C:\repo\outside.ts`,
          path: String.raw`C:\repo\outside.ts`,
          taskId: 'T1',
        },
        {
          kind: 'missing-task-path',
          message: 'task T1 declares path src/missing.ts that is absent from the repo file set',
          path: 'src/missing.ts',
          taskId: 'T1',
        },
      ],
      waves: [['T1']],
    })
  })

  it('accepts compound verify commands when one segment proves the task result', () => {
    const graph = createTaskGraph([
      task({
        id: 'T1',
        objective: 'compound verify',
        paths: ['./src/compound.ts', 'src/compound.ts'],
        verify: 'cd council/ts && npm run typecheck && npm run lint -- --max-warnings=0',
      }),
      task({
        id: 'T2',
        objective: 'file assertion verify',
        paths: ['src/assertion.ts'],
        verify: 'test -f src/assertion.ts',
      }),
    ])

    expect(
      applyPreFanoutGate({ graph, repoFiles: repoFiles('src/compound.ts', 'src/assertion.ts') }),
    ).toMatchObject({
      ok: true,
      violations: [],
      waves: [['T1', 'T2']],
    })
  })
})
