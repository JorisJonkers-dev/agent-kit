import { describe, expect, it } from 'vitest'

import { commandRegistry, runCli, type CliCommand, type CliResult, type CommandSpec } from './index.js'
import { PreFanoutGateError } from '../workflows/fanout.js'
import type {
  ConfigCommandInput,
  ConfigPaths,
  CouncilApp,
  EvalWorkflowInput,
  FanoutInput,
  FleetInput,
  PlanInput,
  RecommendInput,
  SuperviseInput,
  TriageWorkflowInput,
} from '../app/index.js'

interface ReviewPackInput {
  readonly gate: '1' | 'design' | '2'
  readonly runDir: string
}

interface StatusInput {
  readonly runDir: string
}

type AppCall =
  | { readonly input: ConfigCommandInput; readonly method: 'config' }
  | { readonly input: EvalWorkflowInput; readonly method: 'eval' }
  | { readonly input: FanoutInput; readonly method: 'fanout' }
  | { readonly input: FleetInput; readonly method: 'fleet' }
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

  fanout(input: FanoutInput): Promise<unknown> {
    this.calls.push({ input, method: 'fanout' })
    if (this.options.fanoutError !== undefined) return Promise.reject(this.options.fanoutError)
    return Promise.resolve({ input, method: 'fanout' })
  }

  eval(input: EvalWorkflowInput): Promise<unknown> {
    this.calls.push({ input, method: 'eval' })
    return Promise.resolve({ input, method: 'eval' })
  }

  fleet(input: FleetInput): Promise<unknown> {
    this.calls.push({ input, method: 'fleet' })
    return Promise.resolve({ input, method: 'fleet' })
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
    await expect(runCli(['status', '--run', '--other'], injectedRuntime())).resolves.toEqual({
      exitCode: 2,
      stderr: '--run is required\n',
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
