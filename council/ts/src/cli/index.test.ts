import { describe, expect, it } from 'vitest'

import { commandRegistry, runCli, type CliCommand, type CliResult, type CommandSpec } from './index.js'
import { CouncilApp as RealCouncilApp } from '../app/index.js'
import { PreFanoutGateError } from '../workflows/fanout.js'
import type {
  ConfigCommandInput,
  ConfigPaths,
  CouncilApp,
  CouncilAppLiveStatusInput,
  CouncilAppFanoutInput,
  CouncilAppFleetInput,
  EvalWorkflowInput,
  PlanInput,
  RecommendInput,
  SuperviseInput,
  TriageWorkflowInput,
} from '../app/index.js'
import type { ClockPort, LiveRunArtifacts, WorkerResult } from '../ports/index.js'
import type { Task } from '../shared-kernel/index.js'
import type { StatusWatchTickerPort } from '../workflows/index.js'

interface ReviewPackInput {
  readonly gate: '1' | 'design' | '2'
  readonly runDir: string
}

interface StatusInput {
  readonly runDir: string
}

interface RecordedLiveStatusInput {
  readonly intervalMs?: number
  readonly json?: boolean
  readonly once?: boolean
  readonly runDir: string
}

type AppCall =
  | { readonly input: ConfigCommandInput; readonly method: 'config' }
  | { readonly input: EvalWorkflowInput; readonly method: 'eval' }
  | { readonly input: CouncilAppFanoutInput; readonly method: 'fanout' }
  | { readonly input: CouncilAppFleetInput; readonly method: 'fleet' }
  | { readonly input: RecordedLiveStatusInput; readonly method: 'liveStatus' }
  | { readonly input: PlanInput | undefined; readonly method: 'plan' }
  | { readonly input: RecommendInput; readonly method: 'recommend' }
  | { readonly input: ReviewPackInput; readonly method: 'readReviewPack' }
  | { readonly input: StatusInput; readonly method: 'status' }
  | { readonly input: SuperviseInput; readonly method: 'supervise' }
  | { readonly input: TriageWorkflowInput; readonly method: 'triage' }

interface RecordingAppOptions {
  readonly configError?: Error
  readonly fanoutError?: Error
  readonly planResult?: unknown
  readonly statusError?: unknown
}

const injectedPaths: ConfigPaths = {
  project: '/project/.council.toml',
  user: '/home/test/.config/council/council.toml',
}

class RecordingApp {
  readonly calls: AppCall[] = []
  private readonly options: RecordingAppOptions

  constructor(options: RecordingAppOptions = {}) {
    this.options = options
  }

  plan(input?: PlanInput): Promise<unknown> {
    this.calls.push({ input, method: 'plan' })
    return Promise.resolve(
      this.options.planResult ?? {
        command: 'plan',
        input,
        triage: { input, routed: true },
      },
    )
  }

  fanout(input: CouncilAppFanoutInput): Promise<unknown> {
    this.calls.push({ input, method: 'fanout' })
    if (this.options.fanoutError !== undefined) return Promise.reject(this.options.fanoutError)
    return Promise.resolve(recordingDispatchResult('fanout', input))
  }

  eval(input: EvalWorkflowInput): Promise<unknown> {
    this.calls.push({ input, method: 'eval' })
    return Promise.resolve({ input, method: 'eval' })
  }

  fleet(input: CouncilAppFleetInput): Promise<unknown> {
    this.calls.push({ input, method: 'fleet' })
    return Promise.resolve(recordingDispatchResult('fleet', input))
  }

  recommend(input: RecommendInput): Promise<unknown> {
    this.calls.push({ input, method: 'recommend' })
    return Promise.resolve({ input, method: 'recommend' })
  }

  config(input: ConfigCommandInput): Promise<unknown> {
    this.calls.push({ input, method: 'config' })
    if (this.options.configError !== undefined) return Promise.reject(this.options.configError)
    return Promise.resolve({ input, method: 'config' })
  }

  status(input: StatusInput): Promise<unknown> {
    this.calls.push({ input, method: 'status' })
    if (this.options.statusError !== undefined) return rejectingThenable(this.options.statusError)
    return Promise.resolve({ input, method: 'status' })
  }

  async liveStatus(input: CouncilAppLiveStatusInput): Promise<void> {
    this.calls.push({ input: recordedLiveStatusInput(input), method: 'liveStatus' })
    await input.writer?.write(`live:${liveStatusMode(input)}\n`)
  }

  readReviewPack(input: ReviewPackInput): Promise<unknown> {
    this.calls.push({ input, method: 'readReviewPack' })
    return Promise.resolve({ input, method: 'readReviewPack' })
  }

  supervise(input: SuperviseInput): Promise<unknown> {
    this.calls.push({ input, method: 'supervise' })
    return Promise.resolve({ input, method: 'supervise' })
  }

  triage(input: TriageWorkflowInput): Promise<unknown> {
    this.calls.push({ input, method: 'triage' })
    return Promise.resolve({ input, method: 'triage', route: 'program' })
  }
}

function asCouncilApp(app: RecordingApp): CouncilApp {
  return app as unknown as CouncilApp
}

function injectedRuntime(app = new RecordingApp()): { readonly app: CouncilApp; readonly configPaths: ConfigPaths } {
  return { app: asCouncilApp(app), configPaths: injectedPaths }
}

function rejectingThenable(reason: unknown): Promise<unknown> {
  const thenable = {
    then(_onfulfilled: (value: unknown) => void, onrejected: (reason: unknown) => void): void {
      onrejected(reason)
    },
  }
  return thenable as unknown as Promise<unknown>
}

function recordedLiveStatusInput(input: CouncilAppLiveStatusInput): RecordedLiveStatusInput {
  return {
    ...(input.intervalMs === undefined ? {} : { intervalMs: input.intervalMs }),
    ...(input.json === undefined ? {} : { json: input.json }),
    ...(input.once === undefined ? {} : { once: input.once }),
    runDir: input.runDir,
  }
}

function liveStatusMode(input: CouncilAppLiveStatusInput): string {
  if (input.json === true) return 'json'
  if (input.once === true) return 'once'
  return 'watch'
}

function recordingDispatchResult(
  method: 'fanout' | 'fleet',
  input: CouncilAppFanoutInput | CouncilAppFleetInput,
): unknown {
  if (input.execute !== true) return { input, method }
  return {
    execution: {
      human_summary: `${method} executed in dry-run mode`,
      run_id: method,
      status: 'dry-run',
    },
    input,
    method,
  }
}

function realStatusApp(reads: readonly LiveRunArtifacts[]): CouncilApp {
  return new RealCouncilApp({
    clock: fixedClock('2026-07-03T12:00:00.000Z'),
    liveRunDirReader: recordingLiveReader(reads),
    statusTicker: finiteStatusTicker([]),
  })
}

function fixedClock(iso: string): ClockPort {
  return {
    monotonicMs: () => 0,
    now: () => new Date(iso),
    sleep: () => Promise.resolve(),
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

function cliLiveArtifacts(input: {
  readonly workerResults?: ReadonlyMap<string, WorkerResult>
} = {}): LiveRunArtifacts {
  const workerResults = input.workerResults ?? new Map<string, WorkerResult>()
  return {
    events: [],
    normalized: {
      report: undefined,
      runId: 'run-cli',
      state: { stage: 'fanout' },
      tasks: [cliTask()],
      workerResults,
    },
    workerResults,
    workerSupervisorSnapshots: new Map(),
  }
}

function cliTask(): Task {
  return {
    boundaries: 'Stay in CLI status scope.',
    depends_on: [],
    difficulty: 'moderate',
    id: 'T1',
    model: 'haiku',
    objective: 'Render CLI status.',
    output_format: 'Code edits',
    paths: ['src/cli.ts'],
    title: 'CLI task',
    verify: 'npm test',
  }
}

function parsed(result: CliResult): unknown {
  return JSON.parse(result.stdout) as unknown
}

describe('runCli help and command dispatch', () => {
  it('renders help for no command and help flags', async () => {
    for (const argv of [[], ['--help'], ['-h']] as const) {
      const result = await runCli(argv, injectedRuntime())

      expect(result).toEqual({
        exitCode: 0,
        stderr: '',
        stdout: `${commandRegistry().map((command) => `${command.name}\t${command.help}`).join('\n')}\n`,
      })
    }
  })

  it('can render help with the default app constructor', async () => {
    await expect(runCli(['--help'])).resolves.toMatchObject({
      exitCode: 0,
      stderr: '',
    })
  })

  it('runs the self-test command aliases', async () => {
    for (const argv of [['--self-test'], ['self-test']] as const) {
      const result = await runCli(argv, injectedRuntime())

      expect(result.exitCode).toBe(0)
      expect((parsed(result) as { readonly splitDestUrl: string }).splitDestUrl).toBe('git@github.com:o/n.git')
    }
  })

  it('fails unknown commands before app dispatch', async () => {
    const app = new RecordingApp()

    await expect(runCli(['unknown'], injectedRuntime(app))).resolves.toEqual({
      exitCode: 2,
      stderr: 'unknown command: unknown\n',
      stdout: '',
    })
    expect(app.calls).toEqual([])
  })

  it('registers the recommend, eval, and triage gate commands', () => {
    expect(commandRegistry()).toContainEqual({
      help: 'score a run with the eval workflow',
      name: 'eval',
    })
    expect(commandRegistry()).toContainEqual({
      help: 'recommend council lenses for a problem profile',
      name: 'recommend',
    })
    expect(commandRegistry()).toContainEqual({
      help: 'run the triage gate and emit routing payload',
      name: 'triage',
    })
  })

  it('keeps the post-switch fallback covered for registry entries without an implementation', async () => {
    const registry = commandRegistry() as CommandSpec[]
    const extra = { help: 'future test command', name: 'future-command' as CliCommand }
    registry.push(extra)

    try {
      await expect(runCli(['future-command'], injectedRuntime())).resolves.toEqual({
        exitCode: 2,
        stderr: 'unknown command: future-command\n',
        stdout: '',
      })
    } finally {
      registry.pop()
    }
  })

  it('passes parsed plan flags to the app', async () => {
    const app = new RecordingApp()
    const triage = {
      clarity: 'clear',
      kind: 'bugfix',
      landscape: 'brownfield',
      parallelism: 'none',
      risk: 'low',
      size: 'small',
    } as const

    const result = await runCli(
      [
        'plan',
        '--brief',
        'fix it',
        '--run',
        '/runs/1',
        '--design',
        '--triage',
        JSON.stringify(triage),
        '--intensity',
        'quick',
        '--rounds',
        '3',
        '--planner-a',
        'claude:sonnet',
        '--planner-b',
        'codex:gpt-5',
        '--consolidator',
        'judge',
        '--codex-effort',
        'high',
      ],
      injectedRuntime(app),
    )

    expect(result.exitCode).toBe(0)
    expect(app.calls).toEqual([
      {
        input: {
          brief: 'fix it',
          config: {
            codex_effort: 'high',
            consolidator: 'judge',
            intensity: 'quick',
            planner_a: 'claude:sonnet',
            planner_b: 'codex:gpt-5',
            rounds: 3,
          },
          design: true,
          runDir: '/runs/1',
          triage,
        },
        method: 'plan',
      },
    ])
    expect((parsed(result) as { readonly method?: string }).method).toBeUndefined()
  })

  it('passes plan defaults when optional flags are omitted', async () => {
    const app = new RecordingApp()

    await expect(runCli(['plan'], injectedRuntime(app))).resolves.toMatchObject({
      exitCode: 0,
      stderr: '',
    })
    expect(app.calls).toEqual([
      {
        input: {
          config: {},
          design: false,
        },
        method: 'plan',
      },
    ])
  })

  it('passes parsed fanout, fleet, status, review-pack, eval, triage, and recommend inputs to the app', async () => {
    const triage = {
      clarity: 'needs-questions',
      kind: 'feature',
      landscape: 'greenfield',
      parallelism: 'some',
      risk: 'medium',
      size: 'medium',
    } as const

    const fanout = new RecordingApp()
    await expect(
      runCli(['fanout', '--run', '/runs/2', '--dry-run', '--github'], injectedRuntime(fanout)),
    ).resolves.toMatchObject({ exitCode: 0, stderr: '' })
    expect(fanout.calls).toEqual([
      { input: { dryRun: true, github: true, runDir: '/runs/2' }, method: 'fanout' },
    ])

    const fleet = new RecordingApp()
    await expect(
      runCli(['fleet', '--tasks', '/runs/2/tasks.json', '--agents', 'claude:haiku', '--github'], injectedRuntime(fleet)),
    ).resolves.toMatchObject({ exitCode: 0, stderr: '' })
    expect(fleet.calls).toEqual([
      {
        input: { agents: 'claude:haiku', dryRun: false, github: true, tasksPath: '/runs/2/tasks.json' },
        method: 'fleet',
      },
    ])

    const status = new RecordingApp()
    await expect(runCli(['status', '--run', '/runs/3'], injectedRuntime(status))).resolves.toMatchObject({
      exitCode: 0,
      stderr: '',
    })
    expect(status.calls).toEqual([{ input: { runDir: '/runs/3' }, method: 'status' }])

    const evalApp = new RecordingApp()
    const evalResult = await runCli(['eval', '--run', '/runs/eval-a'], injectedRuntime(evalApp))
    expect(evalResult.exitCode).toBe(0)
    expect(evalApp.calls).toEqual([{ input: { runDir: '/runs/eval-a' }, method: 'eval' }])
    expect(parsed(evalResult)).toEqual({
      input: { runDir: '/runs/eval-a' },
      method: 'eval',
    })

    for (const gate of ['1', 'design', '2'] as const) {
      const reviewPack = new RecordingApp()
      await expect(
        runCli(['review-pack', '--gate', gate, '--run', `/runs/${gate}`], injectedRuntime(reviewPack)),
      ).resolves.toMatchObject({ exitCode: 0, stderr: '' })
      expect(reviewPack.calls).toEqual([
        { input: { gate, runDir: `/runs/${gate}` }, method: 'readReviewPack' },
      ])
    }

    const app = new RecordingApp()
    const result = await runCli(['triage', '--input', JSON.stringify(triage)], injectedRuntime(app))
    expect(result.exitCode).toBe(0)
    expect(app.calls).toEqual([{ input: { triage }, method: 'triage' }])
    expect(parsed(result)).toEqual({
      input: { triage },
      method: 'triage',
      route: 'program',
    })

    const triageRun = new RecordingApp()
    const runResult = await runCli(
      ['triage', '--input', JSON.stringify(triage), '--run', '/runs/triage-a'],
      injectedRuntime(triageRun),
    )
    expect(runResult.exitCode).toBe(0)
    expect(triageRun.calls).toEqual([
      { input: { runDir: '/runs/triage-a', triage }, method: 'triage' },
    ])

    const recommend = new RecordingApp()
    const profile = { kind: 'api', risk: 'high', signals: ['timeout budget'], size: 'medium' } as const
    const recommendation = await runCli(['recommend', '--input', JSON.stringify(profile)], injectedRuntime(recommend))
    expect(recommendation.exitCode).toBe(0)
    expect(recommend.calls).toEqual([{ input: { profile }, method: 'recommend' }])
    expect(parsed(recommendation)).toEqual({
      input: { profile },
      method: 'recommend',
    })
  })

  it('passes parsed live status modes to the app and returns writer output verbatim', async () => {
    const cases = [
      {
        argv: ['status', '--run', '/runs/status-json', '--json'],
        input: { json: true, runDir: '/runs/status-json' },
        stdout: 'live:json\n',
      },
      {
        argv: ['status', '--run', '/runs/status-once', '--once'],
        input: { once: true, runDir: '/runs/status-once' },
        stdout: 'live:once\n',
      },
      {
        argv: ['status', '--run', '/runs/status-watch', '--watch', '--interval-ms', '250'],
        input: { intervalMs: 250, runDir: '/runs/status-watch' },
        stdout: 'live:watch\n',
      },
      {
        argv: ['status', '--run', '/runs/status-watch-default', '--watch'],
        input: { runDir: '/runs/status-watch-default' },
        stdout: 'live:watch\n',
      },
    ] as const

    for (const testCase of cases) {
      const app = new RecordingApp()
      const result = await runCli(testCase.argv, injectedRuntime(app))

      expect(result).toEqual({
        exitCode: 0,
        stderr: '',
        stdout: testCase.stdout,
      })
      expect(app.calls).toEqual([{ input: testCase.input, method: 'liveStatus' }])
    }
  })

  it('renders JSON live status output through the app status watch workflow', async () => {
    const app = realStatusApp([
      cliLiveArtifacts({
        workerResults: new Map([['T1', { status: 'ok', task_id: 'T1' }]]),
      }),
    ])

    const result = await runCli(['status', '--run', '/runs/run-cli', '--json'], { app })

    expect(result.exitCode).toBe(0)
    expect(result.stderr).toBe('')
    expect(parsed(result)).toMatchObject({
      run: 'run-cli',
      tasks: [
        {
          state: 'succeeded',
          taskId: 'T1',
          terminalStatus: 'ok',
        },
      ],
    })
  })

  it('renders one-shot table live status output through the app status watch workflow', async () => {
    const app = realStatusApp([cliLiveArtifacts()])

    const result = await runCli(['status', '--run', '/runs/run-cli', '--once'], { app })

    expect(result).toEqual({
      exitCode: 0,
      stderr: '',
      stdout: `run run-cli stage=fanout elapsed=0s started=- updated=-
rollup counts=ready:1 ready=T1 critical=T1
active -
wave 0
badge     task  duration  details
[READY]   T1    0s        CLI task
`,
    })
  })

  it('renders watch status output with an injected finite ticker', async () => {
    const ticker = finiteStatusTicker(['tick'])
    const reader = recordingLiveReader([
      cliLiveArtifacts(),
      cliLiveArtifacts({
        workerResults: new Map([['T1', { status: 'ok', task_id: 'T1' }]]),
      }),
    ])
    const app = new RealCouncilApp({
      clock: fixedClock('2026-07-03T12:00:00.000Z'),
      liveRunDirReader: reader,
      statusTicker: ticker,
    })

    const result = await runCli(['status', '--run', '/runs/run-cli', '--watch', '--interval-ms', '25'], { app })

    expect(result.exitCode).toBe(0)
    expect(result.stderr).toBe('')
    expect(result.stdout).toContain('rollup counts=ready:1 ready=T1 critical=T1')
    expect(result.stdout).toContain('rollup counts=succeeded:1 ready=- critical=-')
    expect(result.stdout.match(/^run run-cli/gmu)).toHaveLength(2)
    expect(reader.calls).toEqual(['/runs/run-cli', '/runs/run-cli'])
    expect(ticker.intervals).toEqual([25])
  })

  it('passes parsed fanout execute flags to the app and keeps stdout machine-readable', async () => {
    const app = new RecordingApp()

    const result = await runCli(['fanout', '--run', '/runs/run-cli', '--execute'], injectedRuntime(app))

    expect(result.exitCode).toBe(0)
    expect(app.calls).toEqual([
      {
        input: {
          baseRef: 'HEAD',
          concurrency: { max_parallel_tasks: 1 },
          dryRun: false,
          execute: true,
          github: false,
          integrationBranch: 'council/run-cli/integration',
          runDir: '/runs/run-cli',
        },
        method: 'fanout',
      },
    ])
    expect(parsed(result)).toEqual({
      execution: {
        human_summary: 'fanout executed in dry-run mode',
        run_id: 'fanout',
        status: 'dry-run',
      },
      input: app.calls[0]?.input,
      method: 'fanout',
    })
  })

  it('passes parsed fleet execute flags to the app and preserves execution summaries in JSON stdout', async () => {
    const app = new RecordingApp()

    const result = await runCli(
      [
        'fleet',
        '--tasks',
        '/runs/fleet-cli.json',
        '--agents',
        'codex:gpt-5,claude:sonnet',
        '--execute',
        '--dry-run',
        '--eval',
        '--base-ref',
        'origin/main',
        '--concurrency',
        '3',
      ],
      injectedRuntime(app),
    )

    expect(result.exitCode).toBe(0)
    expect(app.calls).toEqual([
      {
        input: {
          agents: 'codex:gpt-5,claude:sonnet',
          baseRef: 'origin/main',
          concurrency: { max_parallel_tasks: 3 },
          dryRun: true,
          eval: { enabled: true },
          execute: true,
          github: false,
          integrationBranch: 'council/fleet-cli/integration',
          tasksPath: '/runs/fleet-cli.json',
        },
        method: 'fleet',
      },
    ])
    expect(parsed(result)).toEqual({
      execution: {
        human_summary: 'fleet executed in dry-run mode',
        run_id: 'fleet',
        status: 'dry-run',
      },
      input: app.calls[0]?.input,
      method: 'fleet',
    })
  })

  it('passes parsed supervise flags and command arguments to the app', async () => {
    const app = new RecordingApp()

    const result = await runCli(
      [
        'supervise',
        '--run',
        '/runs/run-a',
        '--task',
        'T1',
        '--worktree',
        '/worktrees/T1',
        '--command',
        'node',
        '--stdin',
        'initial prompt',
        '--restart-preamble',
        'retry prompt',
        '--checkpoint-preamble',
        'checkpoint prompt',
        '--streaming-stdin',
        '--mcp-profile',
        'code-intel',
        '--model-tier',
        'cheap',
        '--escalation-model-tier',
        'max',
        '--poll-interval-ms',
        '10',
        '--kill-grace-ms',
        '20',
        '--stall-after-s',
        '1',
        '--watchdog-window',
        '3',
        '--watchdog-repeat-limit',
        '2',
        '--watchdog-max-cycle-gram',
        '4',
        '--max-restarts',
        '5',
        '--disk-cap-bytes',
        '6',
        '--wall-clock-cap-ms',
        '7',
        '--output-cap-bytes',
        '8',
        '--attempt-timeout-ms',
        '9',
        '--retry-base-backoff-ms',
        '11',
        '--retry-max-backoff-ms',
        '12',
        '--retry-jitter-ratio',
        '0.5',
        '--no-tier-escalation',
        '--',
        '-e',
        'console.log(1)',
      ],
      injectedRuntime(app),
    )

    expect(result.exitCode).toBe(0)
    expect(app.calls).toEqual([
      {
        input: {
          args: ['-e', 'console.log(1)'],
          checkpointPreamble: 'checkpoint prompt',
          command: 'node',
          escalationModelTier: 'max',
          killGraceMs: 20,
          mcpProfile: 'code-intel',
          modelTier: 'cheap',
          pollIntervalMs: 10,
          restartPreamble: 'retry prompt',
          runDir: '/runs/run-a',
          stdin: 'initial prompt',
          supportsStreamingStdin: true,
          taskId: 'T1',
          watchdog: {
            attemptTimeoutMs: 9,
            diskCapBytes: 6,
            enableTierEscalation: false,
            maxCycleGram: 4,
            maxRestarts: 5,
            outputCapBytes: 8,
            repeatLimit: 2,
            retryBaseBackoffMs: 11,
            retryJitterRatio: 0.5,
            retryMaxBackoffMs: 12,
            stallAfterS: 1,
            wallClockCapMs: 7,
            windowSize: 3,
          },
          worktree: '/worktrees/T1',
        },
        method: 'supervise',
      },
    ])
    expect(parsed(result)).toEqual({
      input: app.calls[0]?.input,
      method: 'supervise',
    })
  })

  it('returns compiled placeholders for commands without TypeScript implementations yet', async () => {
    for (const command of [
      'amend',
      'context',
      'design',
      'grill',
      'inject',
      'split',
      'survey',
      'sync-bmad',
      'sync-skills',
      'tail',
    ] as const) {
      await expect(runCli([command], injectedRuntime())).resolves.toEqual({
        exitCode: 0,
        stderr: '',
        stdout: `${JSON.stringify({ command, compiled: true }, null, 2)}\n`,
      })
    }
  })
})

describe('runCli config actions', () => {
  it('passes show, get, set, unset, and path actions with injected config paths', async () => {
    const cases = [
      {
        argv: ['config', 'show'],
        input: { action: 'show', paths: injectedPaths, project: false },
      },
      {
        argv: ['config', 'get', 'rounds'],
        input: { action: 'get', key: 'rounds', paths: injectedPaths, project: false },
      },
      {
        argv: ['config', 'set', 'rounds', '4', '--project'],
        input: { action: 'set', key: 'rounds', paths: injectedPaths, project: true, value: '4' },
      },
      {
        argv: ['config', 'unset', 'worker'],
        input: { action: 'unset', key: 'worker', paths: injectedPaths, project: false },
      },
      {
        argv: ['config', 'path', '--project'],
        input: { action: 'path', paths: injectedPaths, project: true },
      },
    ] as const

    for (const testCase of cases) {
      const app = new RecordingApp()
      const result = await runCli(testCase.argv, injectedRuntime(app))

      expect(result.exitCode).toBe(0)
      expect(app.calls).toEqual([{ input: testCase.input, method: 'config' }])
    }
  })

  it('uses default config paths when runtime paths are omitted', async () => {
    const originalHome = process.env.HOME
    const app = new RecordingApp()
    delete process.env.HOME

    try {
      const result = await runCli(['config', 'path'], { app: asCouncilApp(app) })

      expect(result.exitCode).toBe(0)
      expect(app.calls).toEqual([
        {
          input: {
            action: 'path',
            paths: {
              project: '.council.toml',
              user: './.config/council/council.toml',
            },
            project: false,
          },
          method: 'config',
        },
      ])
    } finally {
      if (originalHome === undefined) {
        delete process.env.HOME
      } else {
        process.env.HOME = originalHome
      }
    }
  })

  it('reports config parse and app errors', async () => {
    await expect(runCli(['config'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: 'config requires action show|get|set|unset|path\n',
      stdout: '',
    })
    await expect(runCli(['config', 'edit'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: 'config requires action show|get|set|unset|path\n',
      stdout: '',
    })

    const app = new RecordingApp({ configError: new Error('config action requires a key') })
    await expect(runCli(['config', 'get'], injectedRuntime(app))).resolves.toEqual({
      exitCode: 2,
      stderr: 'config action requires a key\n',
      stdout: '',
    })
  })
})

describe('runCli error handling', () => {
  it('surfaces pre-fanout gate diagnostics with structured violation fields', async () => {
    const app = new RecordingApp({
      fanoutError: new PreFanoutGateError([
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
        {
          kind: 'non-proving-verify',
          message: 'task T3 verify command does not prove the task result',
          taskId: 'T3',
          verify: 'echo ok',
        },
      ]),
    })

    await expect(runCli(['fanout', '--run', '/runs/overlap'], injectedRuntime(app))).resolves.toEqual({
      exitCode: 2,
      stderr:
        'pre-fanout static gate failed\n' +
        '- gate=pre-fanout-static kind=same-wave-path-overlap taskId=T1 otherTaskId=T2 wave=0 path=council/ts/src/contexts/graph/adapters/process/session.ts otherPath=council/ts/src/contexts/graph/adapters/process/session.ts message=tasks T1 and T2 both declare council/ts/src/contexts/graph/adapters/process/session.ts in ready wave 0\n' +
        '- gate=pre-fanout-static kind=non-proving-verify taskId=T3 verify=echo ok message=task T3 verify command does not prove the task result\n',
      stdout: '',
    })
  })

  it('reports missing required flags and invalid review gates', async () => {
    await expect(runCli(['plan', '--brief'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: '--brief is required\n',
      stdout: '',
    })
    await expect(runCli(['fanout'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: '--run is required\n',
      stdout: '',
    })
    await expect(runCli(['eval'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: '--run is required\n',
      stdout: '',
    })
    await expect(runCli(['fleet', '--tasks', '/tmp/tasks.json'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: '--agents is required\n',
      stdout: '',
    })
    await expect(
      runCli(['fanout', '--run', '/runs/run-cli', '--execute', '--concurrency'], injectedRuntime()),
    ).resolves.toEqual({
      exitCode: 2,
      stderr: '--concurrency is required\n',
      stdout: '',
    })
    await expect(
      runCli(
        [
          'fleet',
          '--tasks',
          '/tmp/tasks.json',
          '--agents',
          'codex:gpt-5',
          '--execute',
          '--concurrency',
          '1.5',
        ],
        injectedRuntime(),
      ),
    ).resolves.toEqual({
      exitCode: 2,
      stderr: '--concurrency must be a positive integer\n',
      stdout: '',
    })
    await expect(runCli(['status', '--run', '--other'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: '--run is required\n',
      stdout: '',
    })
    await expect(
      runCli(['status', '--run', '/runs/4', '--watch', '--interval-ms'], injectedRuntime()),
    ).resolves.toEqual({
      exitCode: 2,
      stderr: '--interval-ms is required\n',
      stdout: '',
    })
    await expect(
      runCli(['status', '--run', '/runs/4', '--watch', '--interval-ms', '0'], injectedRuntime()),
    ).resolves.toEqual({
      exitCode: 2,
      stderr: '--interval-ms must be a positive integer\n',
      stdout: '',
    })
    await expect(
      runCli(['status', '--run', '/runs/4', '--watch', '--interval-ms', '1.5'], injectedRuntime()),
    ).resolves.toEqual({
      exitCode: 2,
      stderr: '--interval-ms must be a positive integer\n',
      stdout: '',
    })
    await expect(
      runCli(['status', '--run', '/runs/4', '--once', '--interval-ms', '100'], injectedRuntime()),
    ).resolves.toEqual({
      exitCode: 2,
      stderr: '--interval-ms requires --watch\n',
      stdout: '',
    })
    await expect(runCli(['status', '--run', '/runs/4', '--json', '--watch'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: 'status mode must be only one of --json, --once, or --watch\n',
      stdout: '',
    })
    await expect(runCli(['review-pack', '--gate', '3', '--run', '/runs/4'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: '--gate must be 1, design, or 2\n',
      stdout: '',
    })
    await expect(runCli(['triage'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: '--input is required\n',
      stdout: '',
    })
    await expect(runCli(['recommend'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: '--input is required\n',
      stdout: '',
    })
    await expect(runCli(['supervise', '--run', '/runs/run-a', '--task', 'T1'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: '--worktree is required\n',
      stdout: '',
    })
    await expect(
      runCli(
        [
          'supervise',
          '--run',
          '/runs/run-a',
          '--task',
          'T1',
          '--worktree',
          '/worktrees/T1',
          '--command',
          'node',
          '--poll-interval-ms',
          '0',
        ],
        injectedRuntime(),
      ),
    ).resolves.toEqual({
      exitCode: 2,
      stderr: '--poll-interval-ms must be a positive number\n',
      stdout: '',
    })
    await expect(runCli(['recommend', '--input', '{'], injectedRuntime())).resolves.toMatchObject({
      exitCode: 2,
      stdout: '',
    })
  })

  it('stringifies non-Error throws from app dispatch', async () => {
    const app = new RecordingApp({ statusError: 'plain failure' })

    await expect(runCli(['status', '--run', '/runs/5'], injectedRuntime(app))).resolves.toEqual({
      exitCode: 2,
      stderr: 'plain failure\n',
      stdout: '',
    })
  })
})
