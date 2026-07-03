import { describe, expect, it } from 'vitest'

import { CouncilApp } from './council-app.js'
import type {
  SuperviseRunStore,
  SuperviseWorkerSupervisor,
  SuperviseWorkerSupervisorDependencies,
  SuperviseWorkerSupervisorResult,
  SuperviseWorkerSupervisorSession,
  SuperviseWorkerSupervisorSnapshot,
  SuperviseWorkerSupervisorStartRequest,
} from './council-app.js'
import type {
  WorktreeDependencyProvisioningRequest,
  WorktreeDependencyProvisioningResult,
  WorktreeDependencyProvisionerPort,
} from '../adapters/worktree-provisioning/index.js'
import type { RunStoreEvent, WorkerLifecycleEvent } from '../contexts/runstore/index.js'
import type { ProcessCommand, ProcessPort, ProcessResult } from '../ports/index.js'
import type { Task } from '../shared-kernel/index.js'
import type { RunSummary } from '../workflows/index.js'

describe('CouncilApp execute contract smoke', () => {
  it('executes native fanout through fake ports and attaches eval output for the run', async () => {
    const processPort = new FakeExecuteProcess()
    const provisioner = new FakeWorktreeDependencyProvisioner()
    const runStore = new FakeRunStore()
    const supervisors: FakeWorkerSupervisor[] = []
    const runStoreRoots: string[] = []
    const statusRequests: { readonly runDir: string }[] = []
    const promptWrites: { readonly path: string; readonly text: string }[] = []
    const app = new CouncilApp({
      createRunStore(root) {
        runStoreRoots.push(root)
        return runStore
      },
      createWorkerSupervisor(dependencies) {
        const supervisor = new FakeWorkerSupervisor(dependencies, completedSupervisorResult('T12'))
        supervisors.push(supervisor)
        return supervisor
      },
      integrationWorktreePath: '/repo',
      nowIso: () => '2026-07-03T14:00:00.000Z',
      process: processPort,
      repoRoot: '/repo',
      status(input) {
        statusRequests.push(input)
        return Promise.resolve(executeRunSummary())
      },
      worktreeDependencyProvisioner: provisioner,
      worktreeRoot: '/repo/.worktrees/workers/run-execute',
      writeText(path, text) {
        promptWrites.push({ path, text })
        return Promise.resolve()
      },
    })

    const plan = await app.fanout({
      baseRef: 'origin/integration',
      concurrency: { max_parallel_tasks: 1 },
      dryRun: false,
      eval: {
        command: 'council eval --run /runs/run-execute',
        enabled: true,
        metadata: { source: 'execute-smoke' },
        require_clean_boundaries: true,
      },
      execute: true,
      github: false,
      integrationBranch: 'integration/execute',
      repoFiles: new Set(['src/execute-contract.ts']),
      runDir: '/runs/run-execute',
    })

    expect(plan.execution).toMatchObject({
      base_ref: 'origin/integration',
      dry_run: false,
      eval: {
        command: 'council eval --run /runs/run-execute',
        exit_code: 0,
        metadata: {
          finding_count: 0,
          score: 100,
          status: 'pass',
        },
        output: 'pass score=100 findings=0',
        status: 'passed',
      },
      integration_branch: 'integration/execute',
      run_id: 'run-execute',
      status: 'succeeded',
      task_results: [
        {
          branch: 'worker/T12',
          commit: 'commit-execute',
          files_changed: ['src/execute-contract.ts'],
          status: 'succeeded',
          task_id: 'T12',
          verify: {
            command: 'npx vitest run src/composition/council-app.execute.test.ts',
            exit_code: 0,
            output: 'verified execute contract\n',
            status: 'passed',
          },
          worktree_path: '/repo/.worktrees/workers/run-execute/T12',
        },
      ],
    })
    expect(statusRequests).toEqual([{ runDir: '/runs/run-execute' }, { runDir: '/runs/run-execute' }])
    expect(runStore.readEventRunIds).toEqual(['run-execute'])
    expect(runStoreRoots).toEqual(['/runs', '/runs', '/runs'])
    expect(provisioner.requests).toEqual([
      {
        repoRoot: '/repo',
        worktreePath: '/repo/.worktrees/workers/run-execute/T12',
      },
    ])
    expect(promptWrites).toEqual([
      {
        path: '/repo/.worktrees/workers/run-execute/T12/.council/workers/T12/prompt.md',
        text: expect.stringContaining('Task T12: Harden execute documentation') as string,
      },
    ])
    expect(supervisors).toHaveLength(1)
    expect(supervisors[0]?.startRequests).toEqual([
      {
        args: [
          '-lc',
          'codex exec -m gpt-5 -c model_reasoning_effort=medium --skip-git-repo-check -o .council/workers/T12/output.json "$(cat .council/workers/T12/prompt.md)"',
        ],
        command: 'sh',
        id: 'T12',
        mcpProfile: 'code-intel',
        modelTier: 'sonnet',
        worktree: '/repo/.worktrees/workers/run-execute/T12',
      },
    ])
    expect(processPort.commands).toEqual([
      gitCommand('/repo', ['show-ref', '--verify', '--quiet', 'refs/heads/worker/T12']),
      gitCommand('/repo', ['worktree', 'add', '-b', 'worker/T12', '/repo/.worktrees/workers/run-execute/T12', 'HEAD']),
      {
        args: ['-lc', 'npx vitest run src/composition/council-app.execute.test.ts'],
        command: 'sh',
        cwd: '/repo/.worktrees/workers/run-execute/T12',
      },
      gitCommand('/repo/.worktrees/workers/run-execute/T12', ['status', '--porcelain=v1']),
      gitCommand('/repo/.worktrees/workers/run-execute/T12', ['status', '--porcelain=v1']),
      gitCommand('/repo/.worktrees/workers/run-execute/T12', ['add', '-A']),
      gitCommand('/repo/.worktrees/workers/run-execute/T12', ['commit', '-m', 'T12 Harden execute documentation']),
      gitCommand('/repo/.worktrees/workers/run-execute/T12', ['rev-parse', 'HEAD']),
      gitCommand('/repo/.worktrees/workers/run-execute/T12', ['branch', '--show-current']),
      gitCommand('/repo', ['show-ref', '--verify', '--quiet', 'refs/heads/integration/execute']),
      gitCommand('/repo', ['checkout', '-B', 'integration/execute', 'origin/integration']),
      gitCommand('/repo', ['merge', '--no-ff', '--no-edit', 'worker/T12']),
      gitCommand('/repo', ['rev-parse', 'HEAD']),
      gitCommand('/repo', ['worktree', 'remove', '--force', '/repo/.worktrees/workers/run-execute/T12']),
    ])
    expect(processPort.commands.some((command) => command.args.includes('--show-toplevel'))).toBe(false)
    expect(processPort.commands.some(spawnsCodexOrClaude)).toBe(false)
    expect(runStore.results).toHaveLength(2)
    expect(runStore.events.map(({ event }) => event.type)).toEqual([
      'worker_started',
      'worker_finished',
      'worker_finished',
    ])
  })
})

class FakeExecuteProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []

  exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    if (command.command === 'git') {
      return Promise.resolve(this.git(command))
    }
    if (
      command.command === 'sh' &&
      command.cwd === '/repo/.worktrees/workers/run-execute/T12' &&
      command.args.join(' ') === '-lc npx vitest run src/composition/council-app.execute.test.ts'
    ) {
      return Promise.resolve(processResult(0, 'verified execute contract\n'))
    }
    return Promise.resolve(processResult(127, '', `unexpected command ${JSON.stringify(command)}`))
  }

  private git(command: ProcessCommand): ProcessResult {
    const args = command.args
    if (args[0] === 'show-ref') {
      return processResult(1)
    }
    if (args[0] === 'status') {
      return processResult(0, ' M src/execute-contract.ts\n')
    }
    if (args[0] === 'rev-parse') {
      return processResult(0, 'commit-execute\n')
    }
    if (args[0] === 'branch') {
      return processResult(0, 'worker/T12\n')
    }
    return processResult(0)
  }
}

class FakeWorktreeDependencyProvisioner implements WorktreeDependencyProvisionerPort {
  readonly requests: WorktreeDependencyProvisioningRequest[] = []

  provision(request: WorktreeDependencyProvisioningRequest): Promise<WorktreeDependencyProvisioningResult> {
    this.requests.push(request)
    return Promise.resolve({ reason: 'disabled', status: 'skipped', strategy: 'none' })
  }
}

class FakeRunStore implements SuperviseRunStore {
  readonly events: { readonly event: WorkerLifecycleEvent; readonly runId: string }[] = []
  readonly readEventRunIds: string[] = []
  readonly results: {
    readonly result: Parameters<SuperviseRunStore['writeWorkerResult']>[2]
    readonly runId: string
    readonly taskId: string
  }[] = []

  appendWorkerEvent(runId: string, event: WorkerLifecycleEvent): Promise<void> {
    this.events.push({ event, runId })
    return Promise.resolve()
  }

  readEvents(runId: string): Promise<readonly RunStoreEvent[]> {
    this.readEventRunIds.push(runId)
    return Promise.resolve(this.events.map(({ event }) => event))
  }

  readWorkerSupervisorSnapshot(): Promise<SuperviseWorkerSupervisorSnapshot> {
    const error = new Error('missing snapshot') as Error & { code: string }
    error.code = 'ENOENT'
    return Promise.reject(error)
  }

  writeWorkerResult(
    runId: string,
    taskId: string,
    result: Parameters<SuperviseRunStore['writeWorkerResult']>[2],
  ): Promise<void> {
    this.results.push({ result, runId, taskId })
    return Promise.resolve()
  }

  writeWorkerSupervisorSnapshot(): Promise<void> {
    return Promise.resolve()
  }
}

class FakeWorkerSupervisor implements SuperviseWorkerSupervisor {
  readonly startRequests: SuperviseWorkerSupervisorStartRequest[] = []
  private readonly dependencies: SuperviseWorkerSupervisorDependencies
  private readonly supervisorResult: SuperviseWorkerSupervisorResult

  constructor(
    dependencies: SuperviseWorkerSupervisorDependencies,
    supervisorResult: SuperviseWorkerSupervisorResult,
  ) {
    this.dependencies = dependencies
    this.supervisorResult = supervisorResult
  }

  reattach(): SuperviseWorkerSupervisorSession {
    throw new Error('reattach should not be used by the execute smoke')
  }

  start(request: SuperviseWorkerSupervisorStartRequest): SuperviseWorkerSupervisorSession {
    this.startRequests.push(request)
    this.dependencies.onEvent?.({
      attemptId: 1,
      pid: 12012,
      restart: 1,
      restartCount: 0,
      taskId: request.id,
      type: 'started',
      ...(request.modelTier === undefined ? {} : { modelTier: request.modelTier }),
    })
    return {
      inject: () => Promise.reject(new Error('inject should not be used by the execute smoke')),
      result: Promise.resolve(this.supervisorResult),
      stop: () => Promise.resolve(),
    }
  }
}

function executeRunSummary(): RunSummary {
  return {
    run: 'run-execute',
    state: { stage: 'fanout' },
    tasks: [executeTask()],
    waves: [['T12']],
    workerResults: [
      {
        files_changed: ['src/execute-contract.ts'],
        status: 'ok',
        task_id: 'T12',
        verdict: {
          engine: { cli: 'codex', model: 'gpt-5' },
          issues: [],
          reasons: 'execute contract smoke completed',
          satisfied: true,
          task_id: 'T12',
        },
        verify_rc: 0,
      },
    ],
  }
}

function executeTask(): Task {
  return {
    acceptance_criteria: ['Composed execution succeeds through fake app ports.'],
    attachment: {
      activeSkills: ['typescript'],
      mcpProfile: 'code-intel',
    },
    boundaries: 'Only touch src/execute-contract.ts.',
    depends_on: [],
    difficulty: 'moderate',
    engine: { cli: 'codex', model: 'gpt-5' },
    id: 'T12',
    model: 'sonnet',
    objective: 'Smoke test the native execute app contract.',
    output_format: 'Code edits',
    paths: ['src/execute-contract.ts'],
    title: 'Harden execute documentation',
    verify: 'npx vitest run src/composition/council-app.execute.test.ts',
    verify_proves: ['The app-level execute smoke uses fake ports and covers eval attachment.'],
  }
}

function completedSupervisorResult(taskId: string): SuperviseWorkerSupervisorResult {
  return {
    exitCode: 0,
    id: taskId,
    modelTier: 'sonnet',
    restarts: 0,
    signal: null,
    status: 'completed',
    stderr: '',
    stderrBytes: 0,
    stderrLogPath: `workers/${taskId}/logs/stderr.log`,
    stdout: 'worker completed\n',
    stdoutBytes: 17,
    stdoutLogPath: `workers/${taskId}/logs/stdout.log`,
  }
}

function gitCommand(cwd: string, args: readonly string[]): ProcessCommand {
  return {
    args,
    command: 'git',
    cwd,
  }
}

function processResult(exitCode: number, stdout = '', stderr = ''): ProcessResult {
  return {
    exitCode,
    stderr,
    stdout,
  }
}

function spawnsCodexOrClaude(command: ProcessCommand): boolean {
  const commandLine = [command.command, ...command.args].join(' ')
  return commandLine.includes('codex') || commandLine.includes('claude')
}
