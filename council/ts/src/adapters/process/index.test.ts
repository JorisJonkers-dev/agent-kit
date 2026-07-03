import { describe, expect, it } from 'vitest'

import type {
  DagVerifyInput,
  ProcessCommand,
  ProcessPort,
  ProcessResult,
} from '../../ports/index.js'
import { ProcessVerificationAdapter } from './index.js'

type Handler = (command: ProcessCommand) => ProcessResult | Promise<ProcessResult>

class RecordingProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []
  private readonly handler: Handler

  constructor(handler: Handler) {
    this.handler = handler
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    return this.handler(command)
  }
}

function result(exitCode: number, stdout = '', stderr = ''): ProcessResult {
  return {
    exitCode,
    stderr,
    stdout,
  }
}

function verifyInput(command: string): DagVerifyInput {
  return {
    assignment: {
      agent_id: 'native',
      model: 'haiku',
      task_id: 'T1',
    },
    command,
    run_id: 'run-native',
    task: {
      boundaries: 'stay scoped',
      depends_on: [],
      difficulty: 'moderate',
      id: 'T1',
      model: 'haiku',
      objective: 'verify process commands',
      output_format: 'code',
      paths: ['src/adapters/process/index.ts'],
      title: 'Process adapter',
      verify: command,
    },
    worktree_path: '/repo/worktrees/T1',
  }
}

describe('ProcessVerificationAdapter', () => {
  it('runs a passing verification command through the injected process port', async () => {
    const process = new RecordingProcess(() => result(0, 'ok\n', 'warn\n'))
    const adapter = new ProcessVerificationAdapter(process, {
      env: { CI: '1' },
      shell: 'bash',
      shellArgs: ['-lc'],
      timeoutMs: 120_000,
    })

    await expect(adapter.verify(verifyInput('npm test'))).resolves.toEqual({
      command: 'npm test',
      exit_code: 0,
      output: 'ok\nwarn\n',
      status: 'passed',
    })
    expect(process.commands).toEqual([
      {
        args: ['-lc', 'npm test'],
        command: 'bash',
        cwd: '/repo/worktrees/T1',
        env: { CI: '1' },
        timeoutMs: 120_000,
      },
    ])
  })

  it('reports failing verification commands without throwing', async () => {
    const process = new RecordingProcess(() => result(2, '', 'tests failed\n'))
    const adapter = new ProcessVerificationAdapter(process)

    await expect(adapter.verify(verifyInput(' npm run typecheck '))).resolves.toEqual({
      command: 'npm run typecheck',
      exit_code: 2,
      output: 'tests failed\n',
      status: 'failed',
    })
    expect(process.commands).toEqual([
      {
        args: ['-lc', 'npm run typecheck'],
        command: 'sh',
        cwd: '/repo/worktrees/T1',
      },
    ])
  })

  it('skips empty verification commands without invoking the process port', async () => {
    const process = new RecordingProcess(() => result(0, 'unexpected\n'))
    const adapter = new ProcessVerificationAdapter(process)

    await expect(adapter.verify(verifyInput('  '))).resolves.toEqual({
      command: '',
      exit_code: null,
      output: '',
      status: 'skipped',
    })
    expect(process.commands).toEqual([])
  })
})
