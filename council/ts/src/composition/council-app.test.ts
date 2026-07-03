import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it, vi } from 'vitest'

import { CouncilApp, NodeProcessAdapter } from './council-app.js'
import type {
  CouncilTailTickerInput,
  CouncilTailTickerPort,
  SuperviseInput,
  SuperviseRunStore,
  SuperviseWorkerSupervisor,
  SuperviseWorkerSupervisorDependencies,
  SuperviseWorkerSupervisorEvent,
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
import {
  appendWorkerTraceEvents,
  createRepairLoopState,
  decideRepairLoop,
  projectWorkerTrace,
  validateWorkerTraceAppend,
} from '../contexts/orchestration/index.js'
import {
  workerOutputEvent,
  workerStartedEvent,
  type RunStoreEvent,
  type WorkerLifecycleEvent,
} from '../contexts/runstore/index.js'
import type {
  ClockPort,
  DagExecutorInput,
  DagExecutorResult,
  LiveRunArtifacts,
  ProcessCommand,
  ProcessPort,
  ProcessResult,
  WorkerResult,
} from '../ports/index.js'
import type { Task } from '../shared-kernel/index.js'
import { projectRunView, type RunSummary, type StatusWatchTickerPort } from '../workflows/index.js'
import type {
  TailWorkflowFrame,
  TailWorkflowLogReadInput,
  TailWorkflowLogReaderPort,
  TailWorkflowLogStatInput,
} from '../workflows/index.js'

const tempRoots: string[] = []

afterEach(async () => {
  await Promise.all(tempRoots.splice(0).map((root) => rm(root, { force: true, recursive: true })))
})

describe('CouncilApp.recommend', () => {
  it('returns lens recommendations without touching IO adapters', async () => {
    const app = new CouncilApp({
      readText: () => Promise.reject(new Error('readText should not be called')),
      writeText: () => Promise.reject(new Error('writeText should not be called')),
    })

    const recommendation = await app.recommend({
      profile: {
        kind: 'api',
        risk: 'high',
        signals: ['timeout budget'],
        size: 'medium',
      },
    })

    expect(recommendation.lenses.length).toBeGreaterThan(0)
    expect(recommendation.workerCount).toBe(recommendation.lenses.length)
  })
})

describe('CouncilApp.triage', () => {
  it('runs the triage gate and emits triage.json through the injected writer', async () => {
    const writes: { readonly path: string; readonly text: string }[] = []
    const app = new CouncilApp({
      writeText(path, text) {
        writes.push({ path, text })
        return Promise.resolve()
      },
    })

    const payload = await app.triage({
      runDir: '/runs/run-a',
      signals: ['shared files'],
      triage: {
        clarity: 'clear',
        kind: 'feature',
        landscape: 'brownfield',
        parallelism: 'high',
        risk: 'medium',
        size: 'medium',
      },
    })

    expect(payload).toMatchObject({
      council_worthy: true,
      route: 'program',
      topology: 'parallel',
    })
    expect(writes).toEqual([
      {
        path: '/runs/run-a/triage.json',
        text: `${JSON.stringify(payload, null, 2)}\n`,
      },
    ])
  })
})

describe('CouncilApp.eval', () => {
  it('scores run artifacts through the injected status and runstore seams', async () => {
    const store = new RecordingRunStore({
      runEvents: [
        { payload: { attempt: 1, task_id: 'T1', worker_id: 'worker-T1' }, type: 'worker_started' },
        {
          payload: { attempt: 2, reason: 'progress-stall', task_id: 'T1', worker_id: 'worker-T1' },
          type: 'worker_restarted',
        },
      ],
    })
    const roots: string[] = []
    const app = new CouncilApp({
      createRunStore(root) {
        roots.push(root)
        return store
      },
      status(input) {
        expect(input).toEqual({ runDir: '/runs/run-a' })
        return Promise.resolve(evalRunSummary())
      },
    })

    const result = await app.eval({ runDir: '/runs/run-a' })

    expect(roots).toEqual(['/runs'])
    expect(store.readEventRunIds).toEqual(['run-a'])
    expect(result).toMatchObject({
      run: 'run-a',
      summary: {
        retry_count: 1,
        task_count: 1,
        worker_result_count: 1,
      },
    })
  })

  it('treats a missing event log as an empty lifecycle stream', async () => {
    const missingEvents = new Error('missing events') as Error & { code: string }
    missingEvents.code = 'ENOENT'
    const store = new RecordingRunStore({ runEventsError: missingEvents })
    const app = new CouncilApp({
      createRunStore: () => store,
      status: () => Promise.resolve(evalRunSummary()),
    })

    await expect(app.eval({ runDir: '/runs/run-a' })).resolves.toMatchObject({
      summary: {
        retry_count: 0,
      },
    })
    expect(store.readEventRunIds).toEqual(['run-a'])
  })

  it('surfaces unexpected event log read failures', async () => {
    const blockedEvents = new Error('events denied') as Error & { code: string }
    blockedEvents.code = 'EACCES'
    const app = new CouncilApp({
      createRunStore: () => new RecordingRunStore({ runEventsError: blockedEvents }),
      status: () => Promise.resolve(evalRunSummary()),
    })

    await expect(app.eval({ runDir: '/runs/run-a' })).rejects.toThrow('events denied')
  })
})

describe('CouncilApp native execution composition', () => {
  it('preserves fleet GitHub dry-run planning through the app workflow seam', async () => {
    const app = new CouncilApp({
      readText: () => Promise.resolve(JSON.stringify([nativeTask()])),
    })

    await expect(
      app.fleet({
        agents: 'codex:gpt-5',
        dryRun: true,
        github: true,
        repoFiles: new Set(['src/native.ts']),
        tasksPath: '/runs/run-native.json',
      }),
    ).resolves.toMatchObject({
      github: 'dry-run',
      run: 'run-native',
    })
  })

  it('executes fanout through concrete DagExecutor adapters and supervises workers through the app seam', async () => {
    const processPort = new NativeExecutionProcess()
    const provisioner = new RecordingWorktreeDependencyProvisioner()
    const store = new RecordingRunStore()
    const supervisors: RecordingSupervisor[] = []
    const promptWrites: { readonly path: string; readonly text: string }[] = []
    const app = new CouncilApp({
      createRunStore: () => store,
      createWorkerSupervisor: (dependencies) => {
        const supervisor = new RecordingSupervisor(dependencies, completedSupervisorResult('T1'))
        supervisors.push(supervisor)
        return supervisor
      },
      nowIso: () => '2026-07-03T13:00:00.000Z',
      process: processPort,
      repoRoot: '/repo',
      integrationWorktreePath: '/repo',
      worktreeDependencyProvisioner: provisioner,
      worktreeRoot: '/repo/.worktrees/workers/run-native',
      status: () => Promise.resolve(nativeRunSummary('run-native')),
      writeText(path, text) {
        promptWrites.push({ path, text })
        return Promise.resolve()
      },
    })

    const plan = await app.fanout({
      baseRef: 'main',
      concurrency: { max_parallel_tasks: 1 },
      dryRun: false,
      eval: { enabled: false },
      execute: true,
      github: false,
      integrationBranch: 'integration/native',
      repoFiles: new Set(['src/native.ts']),
      runDir: '/runs/run-native',
    })

    expect(plan.execution).toMatchObject({
      integration_branch: 'integration/native',
      run_id: 'run-native',
      status: 'succeeded',
      task_results: [
        {
          branch: 'worker/T1',
          commit: 'commit-native',
          files_changed: ['src/native.ts'],
          status: 'succeeded',
          task_id: 'T1',
          verify: {
            command: 'npm test',
            exit_code: 0,
            output: 'verified\n',
            status: 'passed',
          },
          worker_result: {
            branch: 'worker/T1',
            committed: true,
            files_changed: ['src/native.ts'],
            out_of_bounds: [],
            status: 'ok',
            task_id: 'T1',
            verify_output: 'verified\n',
            verify_rc: 0,
            worktree: '/repo/.worktrees/workers/run-native/T1',
          },
          worktree_path: '/repo/.worktrees/workers/run-native/T1',
        },
      ],
    })
    expect(provisioner.requests).toEqual([
      {
        repoRoot: '/repo',
        worktreePath: '/repo/.worktrees/workers/run-native/T1',
      },
    ])
    expect(promptWrites).toEqual([
      {
        path: '/repo/.worktrees/workers/run-native/T1/.council/workers/T1/prompt.md',
        text: expect.stringContaining('Task T1: Native task') as string,
      },
    ])
    expect(supervisors[0]?.startRequests).toEqual([
      {
        args: [
          '-lc',
          'codex exec -m gpt-5 -c model_reasoning_effort=medium --skip-git-repo-check -o .council/workers/T1/output.json "$(cat .council/workers/T1/prompt.md)"',
        ],
        command: 'sh',
        id: 'T1',
        mcpProfile: 'code-intel',
        modelTier: 'sonnet',
        worktree: '/repo/.worktrees/workers/run-native/T1',
      },
    ])
    expect(processPort.commands).toEqual([
      gitCommand('/repo', ['show-ref', '--verify', '--quiet', 'refs/heads/worker/T1']),
      gitCommand('/repo', ['worktree', 'add', '-b', 'worker/T1', '/repo/.worktrees/workers/run-native/T1', 'HEAD']),
      {
        args: ['-lc', 'npm test'],
        command: 'sh',
        cwd: '/repo/.worktrees/workers/run-native/T1',
      },
      gitCommand('/repo/.worktrees/workers/run-native/T1', ['status', '--porcelain=v1']),
      gitCommand('/repo/.worktrees/workers/run-native/T1', ['status', '--porcelain=v1']),
      gitCommand('/repo/.worktrees/workers/run-native/T1', ['add', '-A']),
      gitCommand('/repo/.worktrees/workers/run-native/T1', ['commit', '-m', 'T1 Native task']),
      gitCommand('/repo/.worktrees/workers/run-native/T1', ['rev-parse', 'HEAD']),
      gitCommand('/repo/.worktrees/workers/run-native/T1', ['branch', '--show-current']),
      gitCommand('/repo', ['show-ref', '--verify', '--quiet', 'refs/heads/integration/native']),
      gitCommand('/repo', ['checkout', 'integration/native']),
      gitCommand('/repo', ['merge', '--no-ff', '--no-edit', 'worker/T1']),
      gitCommand('/repo', ['rev-parse', 'HEAD']),
      gitCommand('/repo', ['worktree', 'remove', '--force', '/repo/.worktrees/workers/run-native/T1']),
    ])
    expect(store.events.map(({ event }) => event.type)).toContain('worker_started')
    expect(store.events.at(-1)).toEqual({
      runId: 'run-native',
      event: {
        payload: {
          result_path: 'workers/T1/result.json',
          status: 'ok',
          task_id: 'T1',
          worker_id: 'native-dag:T1',
        },
        type: 'worker_finished',
      },
    })
  })

  it('discovers the repo root for native execution when no repo root is injected', async () => {
    const processPort = new NativeExecutionProcess()
    const store = new RecordingRunStore()
    const supervisors: RecordingSupervisor[] = []
    const app = new CouncilApp({
      createRunStore: () => store,
      createWorkerSupervisor: (dependencies) => {
        const supervisor = new RecordingSupervisor(dependencies, completedSupervisorResult('T1'))
        supervisors.push(supervisor)
        return supervisor
      },
      process: processPort,
      status: () => Promise.resolve(nativeRunSummary('run-native')),
      worktreeDependencyProvisioner: new RecordingWorktreeDependencyProvisioner(),
      writeText: () => Promise.resolve(),
    })

    await expect(
      app.fanout({
        baseRef: 'main',
        concurrency: { max_parallel_tasks: 1 },
        dryRun: false,
        execute: true,
        github: false,
        integrationBranch: 'integration/native',
        repoFiles: new Set(['src/native.ts']),
        runDir: '/runs/run-native',
      }),
    ).resolves.toMatchObject({
      execution: {
        run_id: 'run-native',
        status: 'succeeded',
      },
    })

    expect(processPort.commands[0]).toEqual(gitCommand(process.cwd(), ['rev-parse', '--show-toplevel']))
    expect(supervisors[0]?.startRequests[0]?.worktree).toBe('/repo/.worktrees/workers/run-native/T1')
  })

  it('injects a fake DagExecutor into fleet execute paths for tests', async () => {
    const executeDag = vi.fn(async (input: DagExecutorInput) => {
      const task = onlyTaskResult(input.tasks)
      const assignment = onlyTaskResult(input.agent_pool.assignments)
      await input.hooks.provision({
        assignment,
        base_ref: input.base_ref,
        integration_branch: input.integration_branch,
        run_id: input.run_id,
        task,
      })
      await input.hooks.supervise({
        assignment,
        branch: `worker/${task.id}`,
        dry_run: input.dry_run,
        run_id: input.run_id,
        task,
        worktree_path: `/tmp/${task.id}`,
      })
      await input.hooks.verify({
        assignment,
        command: task.verify,
        run_id: input.run_id,
        task,
        worktree_path: `/tmp/${task.id}`,
      })
      return nativeExecutionResult(input)
    })
    const app = new CouncilApp({
      executeDag,
      readText: () => Promise.resolve(JSON.stringify([nativeTask()])),
    })

    const plan = await app.fleet({
      agents: 'codex:gpt-5',
      baseRef: 'main',
      concurrency: { max_parallel_tasks: 1 },
      dryRun: false,
      eval: { enabled: false },
      execute: true,
      github: false,
      integrationBranch: 'integration/native',
      repoFiles: new Set(['src/native.ts']),
      tasksPath: '/runs/run-native.json',
    })

    expect(plan.execution).toMatchObject({
      run_id: 'run-native',
      status: 'succeeded',
    })
    expect(executeDag).toHaveBeenCalledTimes(1)
    expect(executeDag.mock.calls[0]?.[0]).toMatchObject({
      base_ref: 'main',
      integration_branch: 'integration/native',
      run_id: 'run-native',
      tasks: [nativeTask()],
    })
  })

  it('runs requested eval after executor completion and attaches the eval result to execution output', async () => {
    const calls: string[] = []
    const executeDag = vi.fn((input: DagExecutorInput) => {
      calls.push('execute')
      return Promise.resolve(nativeExecutionResult(input))
    })
    const app = new CouncilApp({
      createRunStore: () => new RecordingRunStore(),
      executeDag,
      status: () => {
        calls.push(calls.includes('execute') ? 'eval-status' : 'plan-status')
        return Promise.resolve(nativeRunSummary('run-native'))
      },
    })

    const plan = await app.fanout({
      baseRef: 'main',
      concurrency: { max_parallel_tasks: 1 },
      dryRun: false,
      eval: { command: 'council eval', enabled: true, require_clean_boundaries: true },
      execute: true,
      github: false,
      integrationBranch: 'integration/native',
      repoFiles: new Set(['src/native.ts']),
      runDir: '/runs/run-native',
    })

    expect(calls).toEqual(['plan-status', 'execute', 'eval-status'])
    expect(plan.execution?.eval).toEqual({
      command: 'council eval',
      exit_code: 0,
      metadata: {
        critical_finding_count: 0,
        finding_count: 0,
        score: 100,
        status: 'pass',
        warning_finding_count: 0,
      },
      output: 'pass score=100 findings=0',
      status: 'passed',
    })
  })

  it('marks failing eval output as a failed execution eval result', async () => {
    const summaries = [nativeRunSummary('run-native'), failedEvalRunSummary('run-native')]
    const app = new CouncilApp({
      createRunStore: () => new RecordingRunStore(),
      executeDag: (input) => Promise.resolve(nativeExecutionResult(input)),
      status: () => Promise.resolve(summaries.shift() ?? failedEvalRunSummary('run-native')),
    })

    const plan = await app.fanout({
      baseRef: 'main',
      concurrency: { max_parallel_tasks: 1 },
      dryRun: false,
      eval: { enabled: true },
      execute: true,
      github: false,
      integrationBranch: 'integration/native',
      repoFiles: new Set(['src/native.ts']),
      runDir: '/runs/run-native',
    })

    expect(plan.execution?.eval).toMatchObject({
      exit_code: 1,
      metadata: {
        status: 'fail',
      },
      status: 'failed',
    })
  })

  it('honors require_clean_boundaries by failing warning-level eval output', async () => {
    const summaries = [nativeRunSummary('run-native'), weakVerifyRunSummary('run-native')]
    const app = new CouncilApp({
      createRunStore: () => new RecordingRunStore(),
      executeDag: (input) => Promise.resolve(nativeExecutionResult(input)),
      status: () => Promise.resolve(summaries.shift() ?? weakVerifyRunSummary('run-native')),
    })

    const plan = await app.fanout({
      baseRef: 'main',
      concurrency: { max_parallel_tasks: 1 },
      dryRun: false,
      eval: { enabled: true, require_clean_boundaries: true },
      execute: true,
      github: false,
      integrationBranch: 'integration/native',
      repoFiles: new Set(['src/native.ts']),
      runDir: '/runs/run-native',
    })

    expect(plan.execution?.eval).toMatchObject({
      exit_code: 1,
      metadata: {
        status: 'warn',
      },
      status: 'failed',
    })
  })
})

describe('CouncilApp.liveStatus', () => {
  it('streams live status frames through the injected reader, clock, ticker, and writer', async () => {
    const reader = recordingLiveReader([
      liveArtifacts(),
      liveArtifacts({
        events: [
          workerStartedEvent({
            attempt: 1,
            pid: 301,
            started_at: '2026-07-03T12:00:00.000Z',
            task_id: 'T1',
            worker_id: 'worker-T1',
          }),
        ],
      }),
    ])
    const ticker = finiteStatusTicker(['tick'])
    const writes: string[] = []
    const app = new CouncilApp({
      clock: fixedClock('2026-07-03T12:05:00.000Z'),
      liveRunDirReader: reader,
      statusTicker: ticker,
      statusWriter: {
        write(output) {
          writes.push(output)
          return Promise.resolve()
        },
      },
    })

    await app.liveStatus({ intervalMs: 250, runDir: '/runs/live' })

    expect(reader.calls).toEqual(['/runs/live', '/runs/live'])
    expect(ticker.intervals).toEqual([250])
    expect(writes.map((output) => output.split('\n')[1])).toEqual([
      'rollup counts=ready:1 ready=T1 critical=T1',
      'rollup counts=running:1 ready=- critical=T1',
    ])
  })

  it('allows a call-scoped writer for JSON one-shot output', async () => {
    const reader = recordingLiveReader([
      liveArtifacts({
        workerResults: new Map([['T1', { status: 'ok', task_id: 'T1' }]]),
      }),
    ])
    const constructorWrites: string[] = []
    const callWrites: string[] = []
    const app = new CouncilApp({
      clock: fixedClock('2026-07-03T12:05:00.000Z'),
      liveRunDirReader: reader,
      statusWriter: {
        write(output) {
          constructorWrites.push(output)
          return Promise.resolve()
        },
      },
    })

    await app.liveStatus({
      json: true,
      runDir: '/runs/live',
      writer: {
        write(output) {
          callWrites.push(output)
          return Promise.resolve()
        },
      },
    })

    expect(reader.calls).toEqual(['/runs/live'])
    expect(constructorWrites).toEqual([])
    expect(callWrites).toHaveLength(1)
    expect(callWrites[0]).toContain('"run": "run-live"')
    expect(callWrites[0]).toContain('"state": "succeeded"')
  })

  it('builds watch ticks from the injected clock when no ticker is provided', async () => {
    const sleeps: number[] = []
    const reader = recordingLiveReader([liveArtifacts(), liveArtifacts()])
    const writes: string[] = []
    const app = new CouncilApp({
      clock: fixedClock('2026-07-03T12:05:00.000Z', (ms) => {
        sleeps.push(ms)
        return Promise.resolve()
      }),
      liveRunDirReader: reader,
    })

    await expect(
      app.liveStatus({
        intervalMs: 25,
        runDir: '/runs/live',
        writer: {
          write(output) {
            writes.push(output)
            if (writes.length === 2) throw new Error('stop after tick')
            return Promise.resolve()
          },
        },
      }),
    ).rejects.toThrow('stop after tick')

    expect(sleeps).toEqual([25])
    expect(reader.calls).toEqual(['/runs/live', '/runs/live'])
    expect(writes).toHaveLength(2)
  })

  it('uses the default no-op writer for one-shot live status output', async () => {
    const reader = recordingLiveReader([liveArtifacts()])
    const app = new CouncilApp({
      clock: fixedClock('2026-07-03T12:05:00.000Z'),
      liveRunDirReader: reader,
    })

    await expect(app.liveStatus({ once: true, runDir: '/runs/live' })).resolves.toBeUndefined()
    expect(reader.calls).toEqual(['/runs/live'])
  })
})

describe('CouncilApp.tail', () => {
  it('streams deterministic tail frames through injected readers and ticker', async () => {
    const path = 'workers/T1/logs/stdout.log'
    const reader = recordingLiveReader([
      liveArtifacts({
        workerResults: new Map([['T1', workerResultWithLogs('T1', { stdout: path })]]),
      }),
    ])
    const logs = new MemoryTailLogReader()
    logs.set(path, 'first\n')
    const ticker = finiteTailTicker([
      () => {
        logs.set(path, 'first\nsecond\n')
      },
    ])
    const app = new CouncilApp({
      liveRunDirReader: reader,
      tailLogReader: logs,
      tailTicker: ticker,
    })

    const frames = await app.tail({
      follow: true,
      intervalMs: 33,
      maxBytes: 100,
      runDir: '/runs/live',
      taskId: 'T1',
    })

    expect(frames).toEqual([
      tailFrame('T1', 'stdout', 'first\n', {
        cursor: 6,
        logPath: path,
      }),
      tailFrame('T1', 'stdout', 'second\n', {
        cursor: 13,
        logPath: path,
        offset: 6,
      }),
    ])
    expect(reader.calls).toEqual(['/runs/live', '/runs/live'])
    expect(logs.stats).toEqual([
      { path, runDir: '/runs/live' },
      { path, runDir: '/runs/live' },
    ])
    expect(logs.reads).toEqual([
      { end: 6, path, runDir: '/runs/live', start: 0 },
      { end: 13, path, runDir: '/runs/live', start: 0 },
    ])
    expect(ticker.intervals).toEqual([33])
  })

  it('reads bounded ranges from the default filesystem log reader', async () => {
    const root = await tempRoot('council-tail-')
    const runDir = join(root, 'run-tail')
    const path = 'workers/T1/logs/stdout.log'
    await mkdir(join(runDir, 'workers', 'T1', 'logs'), { recursive: true })
    await writeFile(join(runDir, path), 'old\nnew\n', 'utf8')
    const app = new CouncilApp({
      liveRunDirReader: recordingLiveReader([
        liveArtifacts({
          workerResults: new Map([['T1', workerResultWithLogs('T1', { stdout: path })]]),
        }),
      ]),
    })

    await expect(app.tail({ lines: 1, maxBytes: 100, runDir, taskId: 'T1' })).resolves.toEqual([
      tailFrame('T1', 'stdout', 'new\n', {
        cursor: 8,
        logPath: path,
        offset: 4,
        truncated: true,
      }),
    ])
  })

  it('reports missing default filesystem logs as deterministic frames', async () => {
    const app = new CouncilApp({
      liveRunDirReader: recordingLiveReader([
        liveArtifacts({
          workerResults: new Map([['T1', workerResultWithLogs('T1', { stdout: 'missing/stdout.log' })]]),
        }),
      ]),
    })

    await expect(app.tail({ maxBytes: 100, runDir: '/runs/live', taskId: 'T1' })).resolves.toEqual([
      {
        chunks: [],
        cursor: { offset: 0, stream: 'stdout' },
        logPath: 'missing/stdout.log',
        logPathSource: 'result',
        missing: true,
        missingReason: 'log',
        rotated: false,
        stream: 'stdout',
        taskId: 'T1',
        truncated: false,
      },
    ])
  })

  it('surfaces unexpected default filesystem stat failures', async () => {
    const root = await tempRoot('council-tail-broken-')
    const runDir = join(root, 'not-a-directory')
    await writeFile(runDir, 'not a directory', 'utf8')
    const app = new CouncilApp({
      liveRunDirReader: recordingLiveReader([
        liveArtifacts({
          workerResults: new Map([['T1', workerResultWithLogs('T1', { stdout: 'workers/T1/logs/stdout.log' })]]),
        }),
      ]),
    })

    await expect(app.tail({ maxBytes: 100, runDir, taskId: 'T1' })).rejects.toMatchObject({
      code: 'ENOTDIR',
    })
  })

  it('builds tail follow ticks from the injected clock when no ticker is provided', async () => {
    const sleeps: number[] = []
    const path = 'workers/T1/logs/stdout.log'
    const reader = recordingLiveReader([
      liveArtifacts({
        workerResults: new Map([['T1', workerResultWithLogs('T1', { stdout: path })]]),
      }),
    ])
    const logs = new MemoryTailLogReader({ failReadAt: 2 })
    logs.set(path, 'one\n')
    const app = new CouncilApp({
      clock: fixedClock('2026-07-03T12:05:00.000Z', (ms) => {
        sleeps.push(ms)
        return Promise.resolve()
      }),
      liveRunDirReader: reader,
      tailLogReader: logs,
    })

    await expect(
      app.tail({
        follow: true,
        intervalMs: 25,
        maxBytes: 100,
        runDir: '/runs/live',
        taskId: 'T1',
      }),
    ).rejects.toThrow('stop after tick')

    expect(sleeps).toEqual([25])
    expect(reader.calls).toEqual(['/runs/live', '/runs/live'])
    expect(logs.reads).toHaveLength(2)
  })
})

describe('CouncilApp.supervise', () => {
  it('starts a worker and persists lifecycle events, snapshots, and result artifacts', async () => {
    const store = new RecordingRunStore()
    const supervisors: RecordingSupervisor[] = []
    const roots: string[] = []
    const app = new CouncilApp({
      createRunStore: (root) => {
        roots.push(root)
        return store
      },
      createWorkerSupervisor: (dependencies) => {
        const supervisor = new RecordingSupervisor(dependencies, completedSupervisorResult('T1'))
        supervisors.push(supervisor)
        return supervisor
      },
      nowIso: () => '2026-07-03T10:00:00.000Z',
    })

    const result = await app.supervise({
      args: ['-e', 'console.log("ok")'],
      command: 'node',
      mcpProfile: 'code-intel',
      modelTier: 'cheap',
      runDir: '/runs/run-a',
      taskId: 'T1',
      watchdog: { maxRestarts: 1, stallAfterS: 1 },
      worktree: '/worktrees/T1',
    })

    expect(roots).toEqual(['/runs'])
    expect(supervisors[0]?.startRequests).toEqual([
      {
        args: ['-e', 'console.log("ok")'],
        command: 'node',
        id: 'T1',
        mcpProfile: 'code-intel',
        modelTier: 'cheap',
        watchdog: { maxRestarts: 1, stallAfterS: 1 },
        worktree: '/worktrees/T1',
      },
    ])
    expect(supervisors[0]?.reattachRequests).toEqual([])
    expect(store.snapshots).toEqual([
      { runId: 'run-a', snapshot: supervisorSnapshot('T1'), taskId: 'T1' },
    ])
    expect(store.events).toEqual([
      {
        runId: 'run-a',
        event: {
          payload: {
            attempt: 1,
            command: ['node', '-e', 'console.log("ok")'],
            cwd: '/worktrees/T1',
            model_tier: 'cheap',
            pid: 101,
            started_at: '2026-07-03T10:00:00.000Z',
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_started',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            byte_count: 3,
            log_path: 'workers/T1/logs/stdout.log',
            offset: 0,
            observed_at: '2026-07-03T10:00:00.000Z',
            stream: 'stdout',
            tail: 'ok\n',
            tail_bytes: 3,
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_output',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            byte_count: 5,
            log_path: 'workers/T1/logs/stderr.log',
            offset: 0,
            observed_at: '2026-07-03T10:00:00.000Z',
            stream: 'stderr',
            tail: 'warn\n',
            tail_bytes: 5,
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_output',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            detected_at: '2026-07-03T10:00:00.000Z',
            pid: 101,
            status: 'progress-stall',
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_detected',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            attempt: 2,
            pid: 102,
            previous_pid: 101,
            reason: 'progress-stall',
            restarted_at: '2026-07-03T10:00:00.000Z',
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_restarted',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            exit_code: 0,
            exited_at: '2026-07-03T10:00:00.000Z',
            pid: 102,
            signal: null,
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_exited',
        },
      },
      {
        runId: 'run-a',
        event: {
          payload: {
            finished_at: '2026-07-03T10:00:00.000Z',
            result_path: 'workers/T1/result.json',
            status: 'ok',
            task_id: 'T1',
            worker_id: 'worker-T1',
          },
          type: 'worker_finished',
        },
      },
    ])
    expect(result).toEqual({
      model_tier: 'cheap',
      status: 'ok',
      stderr_bytes: 5,
      stderr_log_path: 'workers/T1/logs/stderr.log',
      stderr_tail: 'warn\n',
      stdout_bytes: 3,
      stdout_log_path: 'workers/T1/logs/stdout.log',
      stdout_tail: 'ok\n',
      task_id: 'T1',
      worktree: '/worktrees/T1',
    })
    expect(store.results).toEqual([{ result, runId: 'run-a', taskId: 'T1' }])
  })

  it('projects app-produced failure lifecycle events into append-only traces and repair decisions', async () => {
    const store = new RecordingRunStore()
    const app = new CouncilApp({
      createRunStore: () => store,
      createWorkerSupervisor: (dependencies) => new RecordingSupervisor(dependencies, supervisorResult('T6', 'failed')),
      nowIso: () => '2026-07-03T12:00:00.000Z',
    })

    const result = await app.supervise(superviseInput({ modelTier: 'cheap', taskId: 'T6' }))
    const trace = projectWorkerTrace(store.events.map(({ event }) => event))

    expect(result.status).toBe('failed')
    expect(trace).toEqual([
      {
        attempt: 1,
        command: ['node'],
        cwd: '/worktrees/T6',
        kind: 'attempt',
        modelTier: 'cheap',
        occurredAt: '2026-07-03T12:00:00.000Z',
        pid: 101,
        sourceEventType: 'worker_started',
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 1,
        byteCount: 3,
        kind: 'output',
        logPath: 'workers/T6/logs/stdout.log',
        occurredAt: '2026-07-03T12:00:00.000Z',
        offset: 0,
        stream: 'stdout',
        tail: 'ok\n',
        tailBytes: 3,
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 1,
        byteCount: 5,
        kind: 'output',
        logPath: 'workers/T6/logs/stderr.log',
        occurredAt: '2026-07-03T12:00:00.000Z',
        offset: 0,
        stream: 'stderr',
        tail: 'warn\n',
        tailBytes: 5,
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 1,
        kind: 'detection',
        occurredAt: '2026-07-03T12:00:00.000Z',
        pid: 101,
        status: 'progress-stall',
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 2,
        kind: 'attempt',
        occurredAt: '2026-07-03T12:00:00.000Z',
        pid: 102,
        previousPid: 101,
        reason: 'progress-stall',
        sourceEventType: 'worker_restarted',
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 2,
        exitCode: 0,
        kind: 'result',
        occurredAt: '2026-07-03T12:00:00.000Z',
        pid: 102,
        signal: null,
        sourceEventType: 'worker_exited',
        taskId: 'T6',
        workerId: 'worker-T6',
      },
      {
        attempt: 2,
        kind: 'result',
        occurredAt: '2026-07-03T12:00:00.000Z',
        resultPath: 'workers/T6/result.json',
        sourceEventType: 'worker_finished',
        status: 'failed',
        taskId: 'T6',
        workerId: 'worker-T6',
      },
    ])

    const appendedTrace = appendWorkerTraceEvents(trace, [
      workerOutputEvent({
        byte_count: 12,
        log_path: 'workers/T6/logs/stderr.log',
        offset: 5,
        observed_at: '2026-07-03T12:00:01.000Z',
        stream: 'stderr',
        tail: 'still fails\n',
        tail_bytes: 12,
        task_id: 'T6',
        worker_id: 'worker-T6',
      }),
    ])

    expect(appendedTrace.slice(0, trace.length)).toEqual(trace)
    expect(appendedTrace.at(-1)).toEqual({
      attempt: 2,
      byteCount: 12,
      kind: 'output',
      logPath: 'workers/T6/logs/stderr.log',
      occurredAt: '2026-07-03T12:00:01.000Z',
      offset: 5,
      stream: 'stderr',
      tail: 'still fails\n',
      tailBytes: 12,
      taskId: 'T6',
      workerId: 'worker-T6',
    })
    expect(() => {
      validateWorkerTraceAppend(
        trace,
        trace.map((entry, index) => (index === 0 && entry.kind === 'attempt' ? { ...entry, pid: 999 } : entry)),
      )
    }).toThrow('worker trace append mutates prior entry at index 0')
    expect(() => {
      validateWorkerTraceAppend(trace, trace.slice(1))
    }).toThrow('worker trace append removed prior entries')

    const firstDecision = decideRepairLoop({
      maxTailChars: 4,
      state: createRepairLoopState(),
      task: {
        id: 'T6',
        verify: 'npm test',
      },
      trace,
      workerResult: {
        status: 'verify-failed',
        verifyOutput: 'expected repairable verification failure',
        verifyRc: 1,
      },
    })

    expect(firstDecision).toEqual({
      kind: 'repair',
      plan: {
        artifacts: {
          stderrTail: 'arn\n',
          stdoutTail: 'ok\n',
          traceSummary: {
            attempts: [1, 2],
            detections: ['progress-stall'],
            latestResultStatus: 'failed',
            resultStatuses: ['worker_exited:exit-0', 'worker_finished:failed'],
            taskId: 'T6',
            workerIds: ['worker-T6'],
          },
          verifyOutput: 'expected repairable verification failure',
          verifyRc: 1,
          workerResultStatus: 'verify-failed',
        },
        attempt: 1,
        taskId: 'T6',
        verifyCommand: 'npm test',
      },
      state: { repairAttemptConsumed: true },
    })

    expect(
      decideRepairLoop({
        maxTailChars: 4,
        state: firstDecision.state,
        task: {
          id: 'T6',
          verify: 'npm test',
        },
        trace,
        workerResult: {
          status: 'verify-failed',
          verifyOutput: 'still failing after repair',
          verifyRc: 1,
        },
      }),
    ).toEqual({
      artifacts: {
        stderrTail: 'arn\n',
        stdoutTail: 'ok\n',
        traceSummary: {
          attempts: [1, 2],
          detections: ['progress-stall'],
          latestResultStatus: 'failed',
          resultStatuses: ['worker_exited:exit-0', 'worker_finished:failed'],
          taskId: 'T6',
          workerIds: ['worker-T6'],
        },
        verifyOutput: 'still failing after repair',
        verifyRc: 1,
        workerResultStatus: 'verify-failed',
      },
      kind: 'terminal-failure',
      reason: 'repair-attempt-consumed',
      state: { repairAttemptConsumed: true },
    })
  })

  it('uses output event timestamps for RunView freshness when terminal timestamps are absent', () => {
    const view = projectRunView({
      clock: { now: () => new Date('2026-07-03T12:10:00.000Z') },
      events: [
        workerStartedEvent({
          started_at: '2026-07-03T12:00:00.000Z',
          task_id: 'T1',
          worker_id: 'worker-T1',
        }),
        workerOutputEvent({
          byte_count: 3,
          log_path: 'workers/T1/logs/stdout.log',
          observed_at: '2026-07-03T12:04:00.000Z',
          offset: 0,
          stream: 'stdout',
          task_id: 'T1',
          worker_id: 'worker-T1',
        }),
      ],
      summary: {
        run: 'run-a',
        state: { stage: 'fanout' },
        tasks: [nativeTask()],
        waves: [['T1']],
        workerResults: [],
      },
    })

    expect(view.tasks[0]?.updatedAt).toBe('2026-07-03T12:04:00.000Z')
    expect(view.rollup.updatedAt).toBe('2026-07-03T12:04:00.000Z')
  })

  it('reattaches from a saved snapshot and writes terminal result statuses', async () => {
    for (const status of ['failed', 'stalled', 'disk-cap', 'stopped'] as const) {
      const store = new RecordingRunStore({ snapshot: supervisorSnapshot('T2') })
      const supervisors: RecordingSupervisor[] = []
      const app = new CouncilApp({
        createRunStore: () => store,
        createWorkerSupervisor: (dependencies) => {
          const supervisor = new RecordingSupervisor(dependencies, supervisorResult('T2', status))
          supervisors.push(supervisor)
          return supervisor
        },
        nowIso: () => '2026-07-03T11:00:00.000Z',
      })

      const result = await app.supervise(superviseInput({ taskId: 'T2' }))

      expect(supervisors[0]?.startRequests).toEqual([])
      expect(supervisors[0]?.reattachRequests).toEqual([
        {
          request: {
            command: 'node',
            id: 'T2',
            worktree: '/worktrees/T2',
          },
          snapshot: supervisorSnapshot('T2'),
        },
      ])
      expect(result.status).toBe(status)
      expect(store.results).toEqual([{ result, runId: 'run-a', taskId: 'T2' }])
      expect(store.events.at(-1)).toEqual({
        runId: 'run-a',
        event: {
          payload: {
            finished_at: '2026-07-03T11:00:00.000Z',
            result_path: 'workers/T2/result.json',
            status,
            task_id: 'T2',
            worker_id: 'worker-T2',
          },
          type: 'worker_finished',
        },
      })
    }
  })

  it('surfaces unexpected snapshot read failures before starting a worker', async () => {
    const app = new CouncilApp({
      createRunStore: () => new RecordingRunStore({ snapshotError: new Error('read denied') }),
      createWorkerSupervisor: () => {
        throw new Error('supervisor should not be created')
      },
    })

    await expect(app.supervise(superviseInput())).rejects.toThrow('read denied')
  })

  it('composes the real fs run store and process supervisor adapters by default', async () => {
    const root = await tempRoot('council-supervise-')
    const runDir = join(root, 'run-real')
    const worktree = await tempRoot('council-supervise-worktree-')

    const result = await new CouncilApp().supervise({
      args: ['-e', 'process.stdout.write("real ok\\n")'],
      command: process.execPath,
      pollIntervalMs: 1,
      runDir,
      taskId: 'T-real',
      worktree,
    })

    expect(result).toMatchObject({
      status: 'ok',
      stdout_tail: 'real ok\n',
      task_id: 'T-real',
      worktree,
    })
    await expect(readFile(join(runDir, 'workers', 'T-real', 'result.json'), 'utf8')).resolves.toContain(
      '"status": "ok"',
    )
    await expect(readFile(join(runDir, 'events.jsonl'), 'utf8')).resolves.toContain('worker_finished')
    await expect(readFile(join(worktree, 'workers', 'T-real', 'logs', 'stdout.log'), 'utf8')).resolves.toBe(
      'real ok\n',
    )
  })

  it('rejects a run directory whose basename is empty', async () => {
    await expect(new CouncilApp().supervise(superviseInput({ runDir: '/' }))).rejects.toThrow(
      '--run must point to a run directory',
    )
  })
})

describe('NodeProcessAdapter', () => {
  it('executes commands asynchronously with cwd and environment overrides', async () => {
    const root = await tempRoot('node-process-adapter-')
    const resolvedRoot = await realpath(root)
    const adapter = new NodeProcessAdapter()

    await expect(
      adapter.exec({
        args: ['-e', 'process.stdout.write(`${process.cwd()} ${process.env.NODE_PROCESS_ADAPTER_TEST ?? ""}`)'],
        command: process.execPath,
        cwd: root,
        env: { NODE_PROCESS_ADAPTER_TEST: 'ok' },
      }),
    ).resolves.toEqual({
      exitCode: 0,
      stderr: '',
      stdout: `${resolvedRoot} ok`,
    })
  })

  it('captures stderr output from commands', async () => {
    const adapter = new NodeProcessAdapter()

    await expect(
      adapter.exec({
        args: ['-e', 'process.stderr.write("problem\\n")'],
        command: process.execPath,
      }),
    ).resolves.toEqual({
      exitCode: 0,
      stderr: 'problem\n',
      stdout: '',
    })
  })

  it('terminates commands that exceed their timeout', async () => {
    const adapter = new NodeProcessAdapter()

    const result = await adapter.exec({
      args: ['-e', 'setTimeout(() => undefined, 1000)'],
      command: process.execPath,
      timeoutMs: 1,
    })

    expect(result).toMatchObject({
      exitCode: 124,
      stderr: '',
      stdout: '',
    })
  })
})

class NativeExecutionProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []

  exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    if (command.command === 'git') {
      return Promise.resolve(this.gitResult(command.args))
    }
    return Promise.resolve({
      exitCode: 0,
      stderr: '',
      stdout: 'verified\n',
    })
  }

  private gitResult(args: readonly string[]): ProcessResult {
    if (args[0] === 'show-ref' && args.at(-1) === 'refs/heads/worker/T1') {
      return processResult(1)
    }
    if (args[0] === 'status') {
      return processResult(0, ' M src/native.ts\n')
    }
    if (args[0] === 'rev-parse' && args[1] === '--show-toplevel') {
      return processResult(0, '/repo\n')
    }
    if (args[0] === 'rev-parse') {
      return processResult(0, 'commit-native\n')
    }
    if (args[0] === 'branch') {
      return processResult(0, 'worker/T1\n')
    }
    return processResult(0)
  }
}

class RecordingWorktreeDependencyProvisioner implements WorktreeDependencyProvisionerPort {
  readonly requests: WorktreeDependencyProvisioningRequest[] = []

  provision(request: WorktreeDependencyProvisioningRequest): Promise<WorktreeDependencyProvisioningResult> {
    this.requests.push(request)
    return Promise.resolve({ status: 'copied', strategy: 'copy' })
  }
}

class RecordingRunStore implements SuperviseRunStore {
  readonly events: { readonly event: WorkerLifecycleEvent; readonly runId: string }[] = []
  readonly readEventRunIds: string[] = []
  readonly results: { readonly result: unknown; readonly runId: string; readonly taskId: string }[] = []
  readonly snapshots: {
    readonly runId: string
    readonly snapshot: SuperviseWorkerSupervisorSnapshot
    readonly taskId: string
  }[] = []
  private readonly runEvents: readonly RunStoreEvent[]
  private readonly runEventsError: Error | undefined
  private readonly snapshot: SuperviseWorkerSupervisorSnapshot | undefined
  private readonly snapshotError: Error | undefined

  constructor(options: {
    readonly runEvents?: readonly RunStoreEvent[]
    readonly runEventsError?: Error
    readonly snapshot?: SuperviseWorkerSupervisorSnapshot
    readonly snapshotError?: Error
  } = {}) {
    this.runEvents = options.runEvents ?? []
    this.runEventsError = options.runEventsError
    this.snapshot = options.snapshot
    this.snapshotError = options.snapshotError
  }

  appendWorkerEvent(runId: string, event: WorkerLifecycleEvent): Promise<void> {
    this.events.push({ event, runId })
    return Promise.resolve()
  }

  readEvents(runId: string): Promise<readonly RunStoreEvent[]> {
    this.readEventRunIds.push(runId)
    if (this.runEventsError !== undefined) return Promise.reject(this.runEventsError)
    return Promise.resolve(this.runEvents)
  }

  readWorkerSupervisorSnapshot(): Promise<SuperviseWorkerSupervisorSnapshot> {
    if (this.snapshot !== undefined) return Promise.resolve(this.snapshot)
    if (this.snapshotError !== undefined) return Promise.reject(this.snapshotError)
    const error = new Error('missing snapshot') as Error & { code: string }
    error.code = 'ENOENT'
    return Promise.reject(error)
  }

  writeWorkerResult(runId: string, taskId: string, result: unknown): Promise<void> {
    this.results.push({ result, runId, taskId })
    return Promise.resolve()
  }

  writeWorkerSupervisorSnapshot(
    runId: string,
    taskId: string,
    snapshot: SuperviseWorkerSupervisorSnapshot,
  ): Promise<void> {
    this.snapshots.push({ runId, snapshot, taskId })
    return Promise.resolve()
  }
}

class RecordingSupervisor implements SuperviseWorkerSupervisor {
  readonly reattachRequests: {
    readonly request: SuperviseWorkerSupervisorStartRequest
    readonly snapshot: SuperviseWorkerSupervisorSnapshot
  }[] = []
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

  reattach(
    request: SuperviseWorkerSupervisorStartRequest,
    snapshot: SuperviseWorkerSupervisorSnapshot,
  ): SuperviseWorkerSupervisorSession {
    this.reattachRequests.push({ request, snapshot })
    return { inject: rejectingInject, result: Promise.resolve(this.supervisorResult), stop: resolvingStop }
  }

  start(request: SuperviseWorkerSupervisorStartRequest): SuperviseWorkerSupervisorSession {
    this.startRequests.push(request)
    this.emitFixtureEvents(request)
    return { inject: rejectingInject, result: Promise.resolve(this.supervisorResult), stop: resolvingStop }
  }

  private emitFixtureEvents(request: SuperviseWorkerSupervisorStartRequest): void {
    const detection = {
      idleMs: 1000,
      kind: 'progress-stall',
      lastProgressAtMs: 0,
    } as const
    const events: readonly SuperviseWorkerSupervisorEvent[] = [
      {
        attemptId: 1,
        ...(request.modelTier === undefined ? {} : { modelTier: request.modelTier }),
        pid: 101,
        restart: 1,
        restartCount: 0,
        taskId: request.id,
        type: 'started',
      },
      {
        attemptId: 1,
        byteCount: 3,
        logPath: `workers/${request.id}/logs/stdout.log`,
        offset: 0,
        pid: 101,
        restartCount: 0,
        tail: 'ok\n',
        tailBytes: 3,
        taskId: request.id,
        type: 'stdout',
      },
      {
        attemptId: 1,
        byteCount: 5,
        logPath: `workers/${request.id}/logs/stderr.log`,
        offset: 0,
        pid: 101,
        restartCount: 0,
        tail: 'warn\n',
        tailBytes: 5,
        taskId: request.id,
        type: 'stderr',
      },
      {
        attemptId: 1,
        detection,
        pid: 101,
        restartCount: 0,
        taskId: request.id,
        type: 'detected',
      },
      {
        attemptId: 2,
        detection,
        pid: 102,
        preamble: 'retry',
        previousPid: 101,
        restart: 1,
        restartCount: 1,
        taskId: request.id,
        type: 'restarted',
      },
      {
        attemptId: 2,
        exitCode: 0,
        pid: 102,
        restartCount: 1,
        signal: null,
        taskId: request.id,
        type: 'exited',
      },
      {
        attemptId: 2,
        mode: 'checkpoint-and-resume',
        pid: 102,
        restartCount: 1,
        taskId: request.id,
        type: 'injected',
      },
      {
        attemptId: 2,
        modelTier: 'max',
        pid: 102,
        restartCount: 1,
        taskId: request.id,
        type: 'tier-escalated',
      },
      {
        attemptId: 2,
        pid: 102,
        reason: 'operator',
        restartCount: 1,
        taskId: request.id,
        type: 'stopped',
      },
      {
        attemptId: 2,
        pid: 102,
        restartCount: 1,
        signal: 'SIGTERM',
        taskId: request.id,
        type: 'terminated',
      },
    ]

    events.forEach((event) => {
      this.dependencies.onEvent?.(event)
    })
    void this.dependencies.onSnapshot?.(supervisorSnapshot(request.id))
  }
}

function rejectingInject(): Promise<never> {
  return Promise.reject(new Error('not implemented in test'))
}

function resolvingStop(): Promise<void> {
  return Promise.resolve()
}

function superviseInput(overrides: Partial<SuperviseInput> = {}): SuperviseInput {
  return {
    command: 'node',
    runDir: '/runs/run-a',
    taskId: 'T1',
    worktree: `/worktrees/${overrides.taskId ?? 'T1'}`,
    ...overrides,
  }
}

function completedSupervisorResult(taskId: string): SuperviseWorkerSupervisorResult {
  return supervisorResult(taskId, 'completed')
}

function supervisorResult(
  taskId: string,
  status: SuperviseWorkerSupervisorResult['status'],
): SuperviseWorkerSupervisorResult {
  return {
    exitCode: status === 'failed' ? 1 : 0,
    id: taskId,
    modelTier: 'cheap',
    restarts: 0,
    signal: null,
    status,
    stderr: 'warn\n',
    stderrBytes: 5,
    stderrLogPath: `workers/${taskId}/logs/stderr.log`,
    stdout: 'ok\n',
    stdoutBytes: 3,
    stdoutLogPath: `workers/${taskId}/logs/stdout.log`,
  }
}

function supervisorSnapshot(taskId: string): SuperviseWorkerSupervisorSnapshot {
  return {
    attempt_id: 1,
    logs: {
      stderr: `workers/${taskId}/logs/stderr.log`,
      stdout: `workers/${taskId}/logs/stdout.log`,
    },
    model_tier: 'cheap',
    offsets: {
      stderr: 0,
      stdout: 0,
    },
    pid: 101,
    restart_count: 0,
    status: 'running',
    task_id: taskId,
    watchdog: {
      handling_detection: false,
      loop: { actions: [] },
      progress: {
        attemptStartedAtMs: 0,
        lastActionAtMs: 0,
        lastOutputAtMs: 0,
        lastProgressAtMs: 0,
        outputBytes: 0,
        startedAtMs: 0,
      },
      retry: {
        attempts: 0,
        failureFingerprints: [],
      },
    },
  }
}

function fixedClock(iso: string, sleep: ClockPort['sleep'] = () => Promise.resolve()): ClockPort {
  return {
    monotonicMs: () => 0,
    now: () => new Date(iso),
    sleep,
  }
}

function recordingLiveReader(reads: readonly LiveRunArtifacts[]): {
  readonly calls: readonly string[]
  readonly readRunDir: (runDir: string) => Promise<LiveRunArtifacts>
} {
  const calls: string[] = []
  let index = 0
  return {
    calls,
    readRunDir(runDir) {
      calls.push(runDir)
      const current = reads[index] ?? reads.at(-1)
      index += 1
      if (current === undefined) throw new Error('missing live status artifact')
      return Promise.resolve(current)
    },
  }
}

function finiteStatusTicker(ticks: readonly unknown[]): StatusWatchTickerPort & {
  readonly intervals: readonly number[]
} {
  const intervals: number[] = []
  return {
    intervals,
    ticks(input) {
      intervals.push(input.intervalMs)
      let index = 0
      const iterable: AsyncIterable<unknown> & AsyncIterator<unknown> = {
        [Symbol.asyncIterator](): AsyncIterator<unknown> {
          return iterable
        },
        next(): Promise<IteratorResult<unknown>> {
          const value = ticks[index]
          index += 1
          return Promise.resolve(value === undefined ? { done: true, value } : { done: false, value })
        },
      }
      return iterable
    },
  }
}

function finiteTailTicker(onTicks: readonly (() => void)[]): CouncilTailTickerPort & {
  readonly intervals: readonly number[]
} {
  const intervals: number[] = []
  return {
    intervals,
    ticks(input: CouncilTailTickerInput): AsyncIterable<void> {
      intervals.push(input.intervalMs)
      return finiteTailTickIterable(onTicks)
    },
  }
}

async function* finiteTailTickIterable(onTicks: readonly (() => void)[]): AsyncIterable<void> {
  for (const onTick of onTicks) {
    onTick()
    await Promise.resolve()
    yield
  }
}

function bytes(text: string): Uint8Array {
  return new TextEncoder().encode(text)
}

class MemoryTailLogReader implements TailWorkflowLogReaderPort {
  readonly reads: TailWorkflowLogReadInput[] = []
  readonly stats: TailWorkflowLogStatInput[] = []
  private readonly failReadAt: number | undefined
  private readonly logs = new Map<string, Uint8Array>()

  constructor(options: { readonly failReadAt?: number } = {}) {
    this.failReadAt = options.failReadAt
  }

  set(path: string, text: string): void {
    this.logs.set(path, bytes(text))
  }

  stat(input: TailWorkflowLogStatInput): Promise<{ readonly sizeBytes: number } | undefined> {
    this.stats.push(input)
    const buffer = this.logs.get(input.path)
    return Promise.resolve(buffer === undefined ? undefined : { sizeBytes: buffer.byteLength })
  }

  read(input: TailWorkflowLogReadInput): Promise<Uint8Array> {
    this.reads.push(input)
    if (this.reads.length === this.failReadAt) return Promise.reject(new Error('stop after tick'))
    const buffer = this.logs.get(input.path) ?? new Uint8Array()
    return Promise.resolve(buffer.subarray(input.start, input.end))
  }
}

function liveArtifacts(input: {
  readonly events?: LiveRunArtifacts['events']
  readonly workerResults?: ReadonlyMap<string, WorkerResult>
} = {}): LiveRunArtifacts {
  const workerResults = input.workerResults ?? new Map<string, WorkerResult>()
  return {
    events: input.events ?? [],
    normalized: {
      report: undefined,
      runId: 'run-live',
      state: { stage: 'fanout' },
      tasks: [nativeTask()],
      workerResults,
    },
    workerResults,
    workerSupervisorSnapshots: new Map(),
  }
}

function workerResultWithLogs(
  taskId: string,
  paths: { readonly stderr?: string; readonly stdout?: string },
): WorkerResult {
  const result: WorkerResult & {
    readonly stderr_log_path?: string
    readonly stdout_log_path?: string
  } = {
    status: 'ok',
    task_id: taskId,
    ...(paths.stderr === undefined ? {} : { stderr_log_path: paths.stderr }),
    ...(paths.stdout === undefined ? {} : { stdout_log_path: paths.stdout }),
  }
  return result
}

function tailFrame(
  taskId: string,
  stream: TailWorkflowFrame['stream'],
  text: string,
  input: {
    readonly cursor: number
    readonly logPath: string
    readonly offset?: number
    readonly truncated?: boolean
  },
): TailWorkflowFrame {
  return {
    chunks: [{ byteCount: bytes(text).byteLength, offset: input.offset ?? 0, stream, text }],
    cursor: { offset: input.cursor, stream },
    logPath: input.logPath,
    logPathSource: 'result',
    missing: false,
    rotated: false,
    stream,
    taskId,
    truncated: input.truncated ?? false,
  }
}

function evalRunSummary(): RunSummary {
  return {
    run: 'run-a',
    state: { stage: 'fanout' },
    tasks: [
      {
        boundaries: 'Only touch src/example.ts.',
        depends_on: [],
        difficulty: 'moderate',
        id: 'T1',
        model: 'sonnet',
        objective: 'Score app-level eval wiring.',
        output_format: 'Code edits',
        paths: ['src/example.ts'],
        title: 'Eval app wiring',
        verify: 'npx vitest run src/example.test.ts',
      },
    ],
    waves: [['T1']],
    workerResults: [
      {
        files_changed: ['src/example.ts'],
        status: 'ok',
        task_id: 'T1',
        verdict: {
          engine: { cli: 'codex', model: 'gpt-5' },
          issues: [],
          reasons: 'complete',
          satisfied: true,
          task_id: 'T1',
        },
        verify_rc: 0,
      },
    ],
  }
}

async function tempRoot(prefix: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), prefix))
  tempRoots.push(root)
  return root
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

function onlyTaskResult<T>(items: readonly T[]): T {
  expect(items).toHaveLength(1)
  const item = items[0]
  expect(item).toBeDefined()
  if (item === undefined) {
    throw new Error('missing task result')
  }
  return item
}

function nativeExecutionResult(input: DagExecutorInput): DagExecutorResult {
  return {
    base_ref: input.base_ref,
    dry_run: input.dry_run,
    failed_tasks: [],
    integration_branch: input.integration_branch,
    run_id: input.run_id,
    skipped_tasks: [],
    status: 'succeeded',
    task_results: input.tasks.map((task) => ({
      status: 'succeeded',
      task_id: task.id,
    })),
  }
}

function nativeRunSummary(run: string): RunSummary {
  return {
    run,
    state: { stage: 'fanout' },
    tasks: [nativeTask()],
    waves: [['T1']],
    workerResults: [
      {
        files_changed: ['src/native.ts'],
        status: 'ok',
        task_id: 'T1',
        verify_rc: 0,
      },
    ],
  }
}

function weakVerifyRunSummary(run: string): RunSummary {
  const weakTask = {
    ...nativeTask(),
    verify: 'echo ok',
    verify_proves: [],
  }
  return {
    ...nativeRunSummary(run),
    tasks: [weakTask],
    workerResults: [
      {
        files_changed: ['src/native.ts'],
        status: 'ok',
        task_id: 'T1',
        verify_rc: 0,
      },
    ],
  }
}

function failedEvalRunSummary(run: string): RunSummary {
  const failingTask = {
    ...nativeTask(),
    paths: ['src/native.ts'],
    verify: 'echo ok',
    verify_proves: [],
  }
  return {
    ...nativeRunSummary(run),
    tasks: [failingTask],
    workerResults: [
      {
        files_changed: ['docs/outside.md'],
        out_of_bounds: ['docs/outside.md'],
        status: 'no-op',
        task_id: 'T1',
        verify_rc: 1,
      },
    ],
  }
}

function nativeTask(): Task {
  return {
    attachment: {
      activeSkills: ['typescript'],
      mcpProfile: 'code-intel',
    },
    boundaries: 'Only touch src/native.ts.',
    depends_on: [],
    difficulty: 'moderate',
    engine: { cli: 'codex', model: 'gpt-5' },
    id: 'T1',
    model: 'sonnet',
    objective: 'Execute native DAG wiring.',
    output_format: 'Code edits',
    paths: ['src/native.ts'],
    title: 'Native task',
    verify: 'npm test',
    verify_proves: ['npm test exercises the native task behavior'],
  }
}
