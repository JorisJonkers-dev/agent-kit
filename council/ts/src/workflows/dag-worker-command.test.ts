import { describe, expect, it } from 'vitest'

import type { DagAgent, DagAgentAssignment } from '../ports/index.js'
import type { Task } from '../shared-kernel/index.js'
import {
  resolveDagWorkerCommand,
  workerCommandOutputFile,
  workerCommandPromptFile,
} from './dag-worker-command.js'

const baseTask: Task = {
  boundaries: 'Only touch the allowed workflow and engine barrel paths.',
  content_hash: 'sha256:task',
  context_refs: ['design/native-dag'],
  depends_on: [],
  difficulty: 'moderate',
  engine: { cli: 'codex', label: 'codex:gpt-5', model: 'gpt-5' },
  id: 'T3',
  model: 'sonnet',
  objective: 'Resolve worker commands from DAG tasks.',
  output_format: 'Implementation patch plus verification notes.',
  paths: [
    'council/ts/src/workflows/dag-worker-command.ts',
    'council/ts/src/workflows/dag-worker-command.test.ts',
  ],
  title: 'Resolve worker commands',
  verify: 'npx vitest run src/workflows/dag-worker-command.test.ts',
  acceptance_criteria: [
    'Resolver maps assigned agents to executable engine commands.',
    'Unit tests do not spawn worker processes.',
  ],
  verify_proves: ['command construction is deterministic and isolated from process spawning'],
}

const codexAgent: DagAgent = {
  id: 'codex-1',
  kind: 'codex',
  labels: ['typescript'],
  max_concurrency: 1,
  model: 'sonnet',
}

const codexAssignment: DagAgentAssignment = {
  agent_id: 'codex-1',
  model: 'sonnet',
  reason: 'Codex owns TypeScript implementation tasks.',
  task_id: 'T3',
}

describe('dag worker command resolver', () => {
  it('resolves a codex task command through the engine registry without spawning a process', () => {
    const resolved = resolveDagWorkerCommand({
      agent: codexAgent,
      assignment: codexAssignment,
      cwd: '/repo/worktrees/T3',
      effort: 'high',
      outputFile: 'workers/T3/result.txt',
      promptFile: 'workers/T3/prompt.md',
      repoRoot: '/repo/worktrees/T3',
      task: baseTask,
    })
    const { prompt, ...resolutionWithoutPrompt } = resolved

    expect(resolutionWithoutPrompt).toEqual({
      command: {
        args: [
          '-lc',
          'codex exec -m gpt-5 -c model_reasoning_effort=high --skip-git-repo-check -o workers/T3/result.txt "$(cat workers/T3/prompt.md)"',
        ],
        command: 'sh',
        cwd: '/repo/worktrees/T3',
      },
      engine: {
        name: 'codex',
        model: 'gpt-5',
      },
      outputFile: 'workers/T3/result.txt',
      promptFile: 'workers/T3/prompt.md',
    })
    expect(prompt).toContain('Task T3: Resolve worker commands')
    expect(prompt).toContain('Objective:\nResolve worker commands from DAG tasks.')
    expect(prompt).toContain(
      [
        'Allowed paths:',
        '- council/ts/src/workflows/dag-worker-command.ts',
        '- council/ts/src/workflows/dag-worker-command.test.ts',
      ].join('\n'),
    )
    expect(prompt).toContain(
      'Boundaries:\nOnly touch the allowed workflow and engine barrel paths.',
    )
    expect(prompt).toContain(
      'Verify command:\nnpx vitest run src/workflows/dag-worker-command.test.ts',
    )
    expect(prompt).toContain(
      'Expected output:\nImplementation patch plus verification notes.',
    )
    expect(prompt).toContain(
      'Path constraint:\nAll file paths you modify or report must be repo-root-relative paths from /repo/worktrees/T3.',
    )
    expect(prompt).toContain(
      [
        'Acceptance criteria:',
        '- Resolver maps assigned agents to executable engine commands.',
        '- Unit tests do not spawn worker processes.',
      ].join('\n'),
    )
    expect(prompt).toContain(
      'Verify proves:\n- command construction is deterministic and isolated from process spawning',
    )
  })

  it('maps shared-kernel assigned engine specs to executable commands', () => {
    const resolved = resolveDagWorkerCommand({
      agent: {
        cli: 'codex',
        label: 'codex:gpt-5.5',
        model: 'gpt-5.5',
      },
      repoRoot: '/repo',
      task: withoutTaskEngine(baseTask),
    })

    expect(resolved.engine).toEqual({ name: 'codex', model: 'gpt-5.5' })
    expect(resolved.command).toEqual({
      args: [
        '-lc',
        'codex exec -m gpt-5.5 -c model_reasoning_effort=medium --skip-git-repo-check -o .council/workers/T3/output.json "$(cat .council/workers/T3/prompt.md)"',
      ],
      command: 'sh',
      cwd: '/repo',
    })
  })

  it('falls back from task engine metadata to the assigned agent kind and model', () => {
    const resolved = resolveDagWorkerCommand({
      agent: {
        id: 'claude-1',
        kind: 'claude',
        model: 'haiku',
      },
      assignment: {
        agent_id: 'claude-1',
        model: 'opus',
        task_id: 'T3',
      },
      repoRoot: '/repo',
      task: withoutTaskEngine(baseTask),
    })

    expect(resolved.command).toEqual({
      args: [
        '-lc',
        'COUNCIL_EFFORT=medium claude -p --model opus --output-format json --permission-mode plan < .council/workers/T3/prompt.md > .council/workers/T3/output.json',
      ],
      command: 'sh',
      cwd: '/repo',
    })
    expect(resolved.engine).toEqual({ name: 'claude', model: 'opus' })
    expect(resolved.promptFile).toBe('.council/workers/T3/prompt.md')
    expect(resolved.outputFile).toBe('.council/workers/T3/output.json')
  })

  it('rejects assignments that do not target a registered engine', () => {
    expect(() =>
      resolveDagWorkerCommand({
        agent: {
          id: 'local-1',
          kind: 'local',
          model: 'sonnet',
        },
        assignment: codexAssignment,
        repoRoot: '/repo',
        task: withoutTaskEngine(baseTask),
      }),
    ).toThrow('Unknown engine: local')
  })

  it('derives stable prompt and output paths for a task', () => {
    expect(workerCommandPromptFile('T7')).toBe('.council/workers/T7/prompt.md')
    expect(workerCommandOutputFile('T7')).toBe('.council/workers/T7/output.json')
  })
})

function withoutTaskEngine(task: Task): Task {
  const taskWithoutEngine = { ...task }
  delete taskWithoutEngine.engine
  return taskWithoutEngine
}
