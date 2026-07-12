import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  assignAgents,
  CouncilApp,
  extractJson,
  localizeVerify,
  parseAgentsPool,
  pythonSelfTestGolden,
  renderTemplate,
  splitDestUrl,
} from '../../src/app/index.js'
import { commandRegistry, runCli } from '../../src/cli/index.js'
import type {
  ClockPort,
  GhPort,
  GhPrRequest,
  LiveRunArtifacts,
  LiveRunDirReaderPort,
  WorkerResult,
} from '../../src/ports/index.js'
import type { RunState, Task } from '../../src/shared-kernel/index.js'
import type {
  TailWorkflowLogReadInput,
  TailWorkflowLogReaderPort,
  TailWorkflowLogStatInput,
} from '../../src/workflows/index.js'

const tempRoots: string[] = []

afterEach(async () => {
  await Promise.all(tempRoots.splice(0).map((root) => rm(root, { force: true, recursive: true })))
})

describe('CLI composition', () => {
  it('registers the required command surface', () => {
    expect(commandRegistry().map((command) => command.name).sort()).toEqual([
      'amend',
      'config',
      'context',
      'design',
      'eval',
      'fanout',
      'fleet',
      'grill',
      'inject',
      'monitor',
      'plan',
      'recommend',
      'review-pack',
      'self-test',
      'split',
      'status',
      'supervise',
      'survey',
      'sync-bmad',
      'sync-skills',
      'tail',
      'triage',
    ])
  })

  it('routes plan through the app service with direct-tier shrink-only policy', async () => {
    const result = await runCli([
      'plan',
      '--triage',
      JSON.stringify({
        clarity: 'clear',
        kind: 'bugfix',
        landscape: 'brownfield',
        parallelism: 'none',
        risk: 'low',
        size: 'trivial',
      }),
      '--design',
      '--rounds',
      '1',
    ])

    expect(result.exitCode).toBe(0)
    expect(JSON.parse(result.stdout)).toMatchObject({
      designRequested: true,
      directTierPolicy: 'shrink-dag-only',
      executesWorkers: false,
      taskLimit: 1,
    })
  })

  it('routes triage and eval through the composed app workflows', async () => {
    const triageInput = {
      clarity: 'clear',
      kind: 'feature',
      landscape: 'brownfield',
      parallelism: 'high',
      risk: 'medium',
      size: 'medium',
    } as const
    const inputOnly = await runCli(['triage', '--input', JSON.stringify(triageInput)])

    expect(inputOnly.exitCode).toBe(0)
    expect(JSON.parse(inputOnly.stdout)).toMatchObject({
      council_worthy: true,
      input: triageInput,
      route: 'program',
      topology: 'parallel',
    })

    const runDir = await tempRoot('council-triage-run-')
    const withRun = await runCli(['triage', '--input', JSON.stringify(triageInput), '--run', runDir])
    const triageFile = JSON.parse(await readFile(join(runDir, 'triage.json'), 'utf8')) as unknown

    expect(withRun.exitCode).toBe(0)
    expect(JSON.parse(withRun.stdout)).toEqual(triageFile)

    const evalRunDir = join(await writeLegacyPythonRuns(), 'legacy-ordinal-ids')
    const evalResult = await runCli(['eval', '--run', evalRunDir])

    expect(evalResult.exitCode).toBe(0)
    expect(JSON.parse(evalResult.stdout)).toMatchObject({
      run: 'legacy-ordinal-ids',
      status: 'pass',
      summary: {
        retry_count: 0,
        task_count: 2,
        worker_result_count: 2,
      },
    })
  })

  it('dogfoods eval through CLI and app on realistic run artifacts', async () => {
    const runDir = await writeDogfoodEvalRun()
    const appResult = await new CouncilApp().eval({ runDir })
    const cliResult = await runCli(['eval', '--run', runDir])
    const cliPayload = JSON.parse(cliResult.stdout) as unknown

    expect(cliResult).toMatchObject({ exitCode: 0, stderr: '' })
    expect(cliPayload).toEqual(appResult)
    expect(appResult).toMatchObject({
      categories: {
        boundary_compliance: { finding_count: 1, score: 83, status: 'warn' },
        lucky_pass_suspicion: { finding_count: 2, score: 67, status: 'fail' },
        no_op_rate: { finding_count: 1, score: 83, status: 'warn' },
        out_of_bounds_edits: { finding_count: 1, score: 83, status: 'warn' },
        result_completeness: { finding_count: 1, score: 83, status: 'warn' },
        retries: { finding_count: 1, score: 83, status: 'warn' },
        verify_relevance: { finding_count: 1, score: 90, status: 'warn' },
      },
      run: basename(runDir),
      score: 83,
      status: 'warn',
      summary: {
        completed_count: 5,
        failed_verify_count: 1,
        lucky_pass_suspicion_count: 2,
        missing_worker_result_count: 1,
        no_op_count: 1,
        out_of_bounds_count: 1,
        report_task_count: 6,
        retry_count: 2,
        satisfied_verdict_count: 5,
        task_count: 6,
        wave_count: 3,
        weak_verify_count: 1,
        worker_result_count: 5,
      },
    })
  })

  it('routes live status JSON and table rendering through CouncilApp without timers', async () => {
    const artifacts = liveRunArtifacts('parity-status')

    const jsonReader = new ParityLiveRunReader([artifacts])
    const jsonResult = await runCli(['status', '--run', '/runs/parity-status', '--json'], {
      app: new CouncilApp({
        clock: fixedClock('2026-07-03T12:00:00.000Z'),
        liveRunDirReader: jsonReader,
      }),
    })

    expect(jsonResult).toMatchObject({ exitCode: 0, stderr: '' })
    expect(JSON.parse(jsonResult.stdout)).toMatchObject({
      run: 'parity-status',
      tasks: [
        {
          state: 'succeeded',
          taskId: 'T1',
          terminalStatus: 'ok',
        },
      ],
    })
    expect(jsonReader.calls).toEqual(['/runs/parity-status'])

    const tableReader = new ParityLiveRunReader([artifacts])
    const tableResult = await runCli(['status', '--run', '/runs/parity-status', '--once'], {
      app: new CouncilApp({
        clock: fixedClock('2026-07-03T12:00:00.000Z'),
        liveRunDirReader: tableReader,
      }),
    })

    expect(tableResult).toEqual({
      exitCode: 0,
      stderr: '',
      stdout: `run parity-status stage=fanout elapsed=0s started=- updated=-
rollup counts=succeeded:1 ready=- critical=-
active -
wave 0
badge   task  duration  details
[OK]    T1    0s        Parity status task; terminal=ok
`,
    })
    expect(tableReader.calls).toEqual(['/runs/parity-status'])
  })

  it('routes tail through CouncilApp with a finite ticker instead of real timers', async () => {
    const path = 'workers/T1/logs/stdout.log'
    const reader = new ParityLiveRunReader([liveRunArtifacts('parity-tail', workerResultWithStdoutLog(path))])
    const logs = new ParityTailLogReader()
    logs.set(path, 'first\n')
    const ticker = finiteTailTicker([
      () => {
        logs.set(path, 'first\nsecond\n')
      },
      () => {
        logs.set(path, 'first\nsecond\nthird\n')
      },
    ])

    const result = await runCli(['tail', 'T1', '--run', '/runs/parity-tail', '--follow', '--interval-ms', '5'], {
      app: new CouncilApp({
        liveRunDirReader: reader,
        tailLogReader: logs,
        tailTicker: ticker,
      }),
    })

    expect(result).toEqual({
      exitCode: 0,
      stderr: '',
      stdout: 'first\nsecond\nthird\n',
    })
    expect(reader.calls).toEqual(['/runs/parity-tail', '/runs/parity-tail', '/runs/parity-tail'])
    expect(ticker.intervals).toEqual([5])
  })
})

describe('plan and GitHub gating', () => {
  it('never invokes the gh adapter from plan regardless of config', async () => {
    const gh = new RecordingGh()
    const app = new CouncilApp({ gh })

    await expect(
      app.plan({
        config: { github: { enabled: true } },
        triage: {
          clarity: 'clear',
          kind: 'bugfix',
          landscape: 'brownfield',
          parallelism: 'none',
          risk: 'low',
          size: 'trivial',
        },
      }),
    ).resolves.toMatchObject({
      directTierPolicy: 'shrink-dag-only',
      executesWorkers: false,
      taskLimit: 1,
    })
    expect(gh.requests).toEqual([])
  })

  it('supports plan defaults and non-direct run resumes without execution', async () => {
    await expect(new CouncilApp().plan()).resolves.toMatchObject({
      designRequested: false,
      estimatedModelCalls: 11,
      executesWorkers: false,
    })
    const resumed = await new CouncilApp().plan({
      runDir: '/runs/full',
      triage: {
        clarity: 'needs-questions',
        kind: 'feature',
        landscape: 'brownfield',
        parallelism: 'some',
        risk: 'medium',
        size: 'medium',
      },
    })
    expect(resumed).toMatchObject({
      runDir: '/runs/full',
      triage: { plan: { executesWorkers: false } },
    })
    expect(resumed).not.toHaveProperty('taskLimit')
  })

  it('gates fanout and fleet gh calls behind --github without --dry-run', async () => {
    const gh = new RecordingGh()
    const app = new CouncilApp({ gh })
    const fixturesRoot = await writeLegacyPythonRuns()
    const runDir = join(fixturesRoot, 'legacy-ordinal-ids')
    const tasksPath = join(runDir, 'tasks.json')

    await expect(app.fanout({ dryRun: true, github: true, runDir })).resolves.toMatchObject({
      github: 'dry-run',
    })
    await expect(app.fanout({ dryRun: false, github: false, runDir })).resolves.toMatchObject({
      github: 'disabled',
    })
    await expect(app.fanout({ dryRun: false, github: true, runDir })).resolves.toMatchObject({
      github: 'created',
      prUrl: 'https://example.test/pr/1',
    })
    await expect(app.fleet({ agents: 'claude:haiku', dryRun: false, github: false, tasksPath })).resolves.toMatchObject({
      github: 'disabled',
    })
    await expect(app.fleet({ agents: 'claude:haiku', dryRun: false, github: true, tasksPath })).resolves.toMatchObject({
      github: 'created',
      prUrl: 'https://example.test/pr/2',
    })
    expect(gh.requests).toHaveLength(2)
    expect(gh.requests[0]?.title).toBe('Council legacy-ordinal-ids')
  })

  it('routes CLI fleet through round-robin assignment and non-dry-run PR creation', async () => {
    const gh = new RecordingGh()
    const app = new CouncilApp({ gh })
    const fixturesRoot = await writeLegacyPythonRuns()
    const tasksPath = join(fixturesRoot, 'legacy-ordinal-ids', 'tasks.json')

    const result = await runCli(
      ['fleet', '--tasks', tasksPath, '--agents', 'claude:haiku,codex:gpt-5.5', '--github'],
      { app },
    )

    expect(result).toMatchObject({ exitCode: 0, stderr: '' })
    expect(JSON.parse(result.stdout)).toMatchObject({
      agents: {
        T1: 'claude:haiku',
        T2: 'codex:gpt-5.5',
      },
      github: 'created',
      prUrl: 'https://example.test/pr/1',
      run: 'tasks',
    })
    expect(gh.requests).toEqual([
      {
        body: 'Council run tasks',
        cwd: '.',
        draft: true,
        title: 'Council tasks',
      },
    ])
  })

  it('requires an adapter for non-dry-run GitHub fanout', async () => {
    await expect(
      new CouncilApp().fanout({
        dryRun: false,
        github: true,
        runDir: join(await writeLegacyPythonRuns(), 'legacy-ordinal-ids'),
      }),
    ).rejects.toThrow('--github requires a gh adapter')
  })
})

describe('Python self-test parity', () => {
  it('matches the golden pure-function cases carried over from council.py --self-test', async () => {
    const golden = JSON.parse(await readFile(join(import.meta.dirname, 'python-self-test-golden.json'), 'utf8')) as unknown

    expect(pythonSelfTestGolden()).toEqual(golden)
    expect(extractJson('Warning: something\n{"result": "ok", "n": 2}')).toEqual({
      n: 2,
      result: 'ok',
    })
    expect(extractJson('text\n```json\n{"a": [1,2]}\n```\nmore')).toEqual({ a: [1, 2] })
    expect(() => extractJson('no json here')).toThrow('no JSON object found')
    expect(() => extractJson('{"a": nope}')).toThrow('no JSON object found')
    expect(renderTemplate('hi {{name}} {{name}}', { name: 'x' })).toBe('hi x x')
    expect(splitDestUrl('o', 'n')).toBe('git@github.com:o/n.git')
    expect(localizeVerify('cd /workspace/services/foo && npm test', '/workspace', '/tmp/wt/T1')).toBe(
      'cd /tmp/wt/T1/services/foo && npm test',
    )
  })

  it('matches Python fleet agent parsing and assignment behavior', () => {
    expect(parseAgentsPool('codex:gpt-5.5*2,claude:haiku*1')).toEqual([
      { cli: 'codex', label: 'codex:gpt-5.5', model: 'gpt-5.5' },
      { cli: 'codex', label: 'codex:gpt-5.5', model: 'gpt-5.5' },
      { cli: 'claude', label: 'claude:haiku', model: 'haiku' },
    ])
    expect(Object.fromEntries(assignAgents(['t1', 't2', 't3'], parseAgentsPool('claude:haiku,codex:gpt-5.5')))).toEqual({
      t1: { cli: 'claude', label: 'claude:haiku', model: 'haiku' },
      t2: { cli: 'codex', label: 'codex:gpt-5.5', model: 'gpt-5.5' },
      t3: { cli: 'claude', label: 'claude:haiku', model: 'haiku' },
    })
    expect(() => parseAgentsPool('codex:x*0')).toThrow('positive integer')
    expect(() => parseAgentsPool('')).toThrow('agents pool must not be empty')
    expect(() => parseAgentsPool('*1')).toThrow('malformed agent spec')
    expect(() => parseAgentsPool('codex:x*1*2')).toThrow('malformed agent spec')
    expect(() => parseAgentsPool('ollama:x*1')).toThrow('engine must be')
    expect(() => parseAgentsPool('notvalid')).toThrow('engine must be')
    expect(() => assignAgents(['t1'], [])).toThrow('agents pool must not be empty')
  })
})

describe('Python run fixtures', () => {
  it('assembles review-pack summaries for gated checkpoints', async () => {
    await expect(
      new CouncilApp().readReviewPack({
        gate: '2',
        runDir: join(await writeLegacyPythonRuns(), 'legacy-ordinal-ids'),
      }),
    ).resolves.toMatchObject({
      gate: '2',
      run: 'legacy-ordinal-ids',
      task_count: 2,
      waves: [['T1'], ['T2']],
      worker_results: 2,
    })
  })

  it('summarizes run dirs without legacy report files', async () => {
    const dir = await tempRoot('council-run-')
    await writeFile(
      join(dir, 'state.json'),
      JSON.stringify({ stage: 'planned', task_count: 1 }, null, 2),
      'utf8',
    )
    await writeFile(
      join(dir, 'tasks.json'),
      JSON.stringify(
        [
          {
            boundaries: 'Stay in scope',
            depends_on: [],
            difficulty: 'trivial',
            id: 'T1',
            model: 'haiku',
            objective: 'One task',
            output_format: 'Code edits',
            paths: ['x.txt'],
            title: 'One task',
            verify: 'test -f x.txt',
          },
        ],
        null,
        2,
      ),
      'utf8',
    )

    const summary = await new CouncilApp().status({ runDir: dir })
    expect(summary).toMatchObject({
      run: dir.split('/').at(-1),
      waves: [['T1']],
      workerResults: [],
    })
    expect(summary).not.toHaveProperty('report')
  })

  it('loads Python run dirs and preserves waves, task schemas, and markdown round-trips', async () => {
    const app = new CouncilApp()
    const fixturesRoot = await writeLegacyPythonRuns()

    for (const scenario of ['legacy-ordinal-ids', 'watchdog-table-config', 'grown-schema-task']) {
      const runDir = join(fixturesRoot, scenario)
      const summary = await app.status({ runDir })
      const report = JSON.parse(await readFile(join(runDir, 'report.json'), 'utf8')) as { waves: readonly (readonly string[])[] }
      const tasks = JSON.parse(await readFile(join(runDir, 'tasks.json'), 'utf8')) as unknown

      expect(summary.run).toBe(scenario)
      expect(summary.waves).toEqual(report.waves)
      expect(summary.tasks).toEqual(tasks)
      await expect(app.roundTripTasksMarkdown(join(runDir, 'tasks.json'))).resolves.toEqual(tasks)
    }
  })

  it('resumes grown Python run dirs as backward-compatible schema supersets', async () => {
    const app = new CouncilApp()
    const fixturesRoot = await writeLegacyPythonRuns()
    const summary = await app.status({ runDir: join(fixturesRoot, 'grown-schema-task') })

    expect(summary.state).toMatchObject({
      intensity: 'quick',
      rounds: 1,
      spec_id: '001-grown-schema-task',
      stage: 'fanned-out',
      task_count: 1,
    })
    expect(summary.tasks[0]).toMatchObject({
      archetype: 'schema-maintenance',
      content_hash: 'sha256:fixture-grown-schema-task',
      context_profile: 'contracts',
      context_refs: ['kb://council/tasks/grown-schema'],
      engine: { cli: 'codex', model: 'gpt-5.5' },
      id: 'ck-b200',
      model_tier: 'expensive',
    })
    expect(summary.workerResults[0]).toMatchObject({
      files_changed: ['schema/grown.json'],
      status: 'ok',
      task_id: 'ck-b200',
    })
  })
})

describe('line-preserving config service', () => {
  it('handles path, show, get, new-file writes, and invalid keys', async () => {
    const writes = new Map<string, string>()
    const app = new CouncilApp({
      readText: (path) => {
        const value = writes.get(path)
        if (value === undefined) {
          const error = new Error(path) as Error & { code: string }
          error.code = 'ENOENT'
          return Promise.reject(error)
        }
        return Promise.resolve(value)
      },
      writeText: (path, text) => {
        writes.set(path, text)
        return Promise.resolve()
      },
    })
    const paths = { project: '/project/new.toml', user: '/user/new.toml' }

    await expect(app.config({ action: 'path', paths })).resolves.toEqual({ paths })
    await expect(app.config({ action: 'show', paths })).resolves.toMatchObject({
      resolved: { intensity: 'standard' },
      target: '/user/new.toml',
    })
    await expect(app.config({ action: 'get', key: 'rounds', paths })).resolves.toMatchObject({
      value: 2,
    })
    await expect(
      app.config({ action: 'set', key: 'worker', paths, value: 'codex:gpt-5.5' }),
    ).resolves.toMatchObject({
      target: '/user/new.toml',
      value: 'codex:gpt-5.5',
    })
    expect(writes.get('/user/new.toml')).toContain('worker = "codex:gpt-5.5"')
    await expect(app.config({ action: 'set', key: 'rounds', paths })).rejects.toThrow(
      'config set requires <key> <value>',
    )
    await expect(app.config({ action: 'get', paths })).rejects.toThrow('config action requires a key')
    await expect(app.config({ action: 'get', key: 'bogus', paths })).rejects.toThrow('unknown key bogus')
  })

  it('updates root keys without dropping table sections', async () => {
    const writes = new Map<string, string>()
    const reads = new Map<string, string>([
      [
        '/project/council.toml',
        [
          '# keep this',
          'rounds = 2',
          '',
          '[watchdog]',
          'stall_after_s = 90',
          '',
        ].join('\n'),
      ],
    ])
    const app = new CouncilApp({
      readText: (path) => {
        const value = writes.get(path) ?? reads.get(path)
        if (value === undefined) {
          const error = new Error(path) as Error & { code: string }
          error.code = 'ENOENT'
          return Promise.reject(error)
        }
        return Promise.resolve(value)
      },
      writeText: (path, text) => {
        writes.set(path, text)
        return Promise.resolve()
      },
    })

    await app.config({
      action: 'set',
      key: 'rounds',
      paths: { project: '/project/council.toml', user: '/user/council.toml' },
      project: true,
      value: '3',
    })
    expect(writes.get('/project/council.toml')).toContain('rounds = 3')
    expect(writes.get('/project/council.toml')).toContain('[watchdog]')

    await app.config({
      action: 'unset',
      key: 'rounds',
      paths: { project: '/project/council.toml', user: '/user/council.toml' },
      project: true,
    })
    expect(writes.get('/project/council.toml')).not.toContain('rounds =')
    expect(writes.get('/project/council.toml')).toContain('[watchdog]')
  })

  it('surfaces non-ENOENT config read failures and writes with the default writer', async () => {
    const app = new CouncilApp({
      readText: () => {
        const error = new Error('denied') as Error & { code: string }
        error.code = 'EACCES'
        return Promise.reject(error)
      },
    })
    await expect(
      app.config({
        action: 'show',
        paths: { project: '/project/blocked.toml', user: '/user/blocked.toml' },
      }),
    ).rejects.toThrow('denied')

    let reads = 0
    const flaky = new CouncilApp({
      readText: () => {
        reads += 1
        if (reads <= 2) return Promise.resolve('')
        const error = new Error('write read failed') as Error & { code: string }
        error.code = 'EIO'
        return Promise.reject(error)
      },
    })
    await expect(
      flaky.config({
        action: 'set',
        key: 'rounds',
        paths: { project: '/project/flaky.toml', user: '/user/flaky.toml' },
        value: '3',
      }),
    ).rejects.toThrow('write read failed')

    const dir = await tempRoot('council-config-')
    const path = join(dir, 'council.toml')
    await new CouncilApp().config({
      action: 'set',
      key: 'rounds',
      paths: { project: join(dir, 'project.toml'), user: path },
      value: '4',
    })
    await expect(readFile(path, 'utf8')).resolves.toContain('rounds = 4')
  })
})

class RecordingGh implements GhPort {
  readonly requests: GhPrRequest[] = []

  createPullRequest(request: GhPrRequest): Promise<{ readonly number: number; readonly url: string }> {
    this.requests.push(request)
    return Promise.resolve({
      number: this.requests.length,
      url: `https://example.test/pr/${String(this.requests.length)}`,
    })
  }

  viewPullRequest(): Promise<{ readonly number: number; readonly url: string }> {
    return Promise.resolve({ number: 1, url: 'https://example.test/pr/1' })
  }
}

class ParityLiveRunReader implements LiveRunDirReaderPort {
  readonly calls: string[] = []
  private index = 0

  constructor(private readonly artifacts: readonly LiveRunArtifacts[]) {}

  readRunDir(runDir: string): Promise<LiveRunArtifacts> {
    this.calls.push(runDir)
    const artifact = this.artifacts[Math.min(this.index, this.artifacts.length - 1)]
    this.index += 1
    if (artifact === undefined) throw new Error('no live run artifacts configured')
    return Promise.resolve(artifact)
  }
}

class ParityTailLogReader implements TailWorkflowLogReaderPort {
  private readonly logs = new Map<string, string>()

  set(path: string, text: string): void {
    this.logs.set(path, text)
  }

  stat(input: TailWorkflowLogStatInput): Promise<{ readonly sizeBytes: number } | undefined> {
    const text = this.logs.get(input.path)
    return Promise.resolve(text === undefined ? undefined : { sizeBytes: Buffer.byteLength(text) })
  }

  read(input: TailWorkflowLogReadInput): Promise<Uint8Array> {
    const text = this.logs.get(input.path)
    if (text === undefined) throw new Error(`missing log ${input.path}`)
    return Promise.resolve(Buffer.from(text).subarray(input.start, input.end))
  }
}

function fixedClock(iso: string): ClockPort {
  return {
    monotonicMs: () => 0,
    now: () => new Date(iso),
    sleep: () => Promise.resolve(),
  }
}

function finiteTailTicker(onTicks: readonly (() => void)[]): {
  readonly intervals: readonly number[]
  ticks(input: { readonly intervalMs: number }): AsyncIterable<void>
} {
  const intervals: number[] = []
  return {
    get intervals() {
      return intervals
    },
    async *ticks(input) {
      intervals.push(input.intervalMs)
      for (const onTick of onTicks) {
        await Promise.resolve()
        onTick()
        yield undefined
      }
    },
  }
}

function liveRunArtifacts(runId: string, result: WorkerResult = workerResult('T1')): LiveRunArtifacts {
  const task = parityTask('T1')
  const state: RunState = {
    stage: 'fanout',
    task_count: 1,
  }
  return {
    events: [],
    normalized: {
      report: {
        run: runId,
        tasks: [{ status: result.status, task_id: result.task_id }],
        waves: [['T1']],
      },
      runId,
      state,
      tasks: [task],
      workerResults: new Map([[result.task_id, result]]),
    },
    workerResults: new Map([[result.task_id, result]]),
    workerSupervisorSnapshots: new Map(),
  }
}

function parityTask(id: 'T1'): Task {
  return {
    boundaries: 'Stay in the parity fixture.',
    depends_on: [],
    difficulty: 'trivial',
    id,
    model: 'haiku',
    objective: 'Exercise status and tail parity.',
    output_format: 'Validated CLI output.',
    paths: ['council/ts/test/parity/cli-app-parity.test.ts'],
    title: 'Parity status task',
    verify: 'npx vitest run test/parity/cli-app-parity.test.ts',
  }
}

function workerResult(taskId: 'T1'): WorkerResult {
  return {
    status: 'ok',
    task_id: taskId,
  }
}

function workerResultWithStdoutLog(path: string): WorkerResult & { readonly stdout_log_path: string } {
  return {
    ...workerResult('T1'),
    stdout_log_path: path,
  }
}

async function tempRoot(prefix: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), prefix))
  tempRoots.push(root)
  return root
}

async function writeLegacyPythonRuns(): Promise<string> {
  const root = await tempRoot('council-python-runs-')
  await writeLegacyOrdinalRun(join(root, 'legacy-ordinal-ids'))
  await writeWatchdogRun(join(root, 'watchdog-table-config'))
  await writeGrownSchemaRun(join(root, 'grown-schema-task'))
  return root
}

interface FixtureTask extends Record<string, unknown> {
  readonly id: string
  readonly paths: readonly string[]
}

async function writeDogfoodEvalRun(): Promise<string> {
  const runDir = join(await tempRoot('council-eval-dogfood-'), 'realistic-run')
  const tasks = dogfoodEvalTasks()
  await writeRunJson(runDir, 'state.json', {
    integration_branch: 'council/realistic-run/integration',
    intensity: 'quick',
    rounds: 1,
    spec_id: '001-realistic-run',
    spec_relpath: 'specs/001-realistic-run',
    spec_slug: 'realistic-run',
    stage: 'fanned-out',
    task_count: tasks.length,
  })
  await writeRunJson(runDir, 'tasks.json', tasks)
  await writeRunJson(runDir, 'report.json', {
    integration_branch: 'council/realistic-run/integration',
    run: basename(runDir),
    tasks: tasks.map((task) => ({ status: task.id === 'T6' ? 'missing' : 'ok', task_id: task.id })),
    waves: [['T1', 'T2'], ['T3', 'T4'], ['T5', 'T6']],
  })
  await writeWorkerResult(runDir, 'T1', {
    files_changed: ['src/clean.ts'],
    out_of_bounds: [],
    status: 'ok',
    task_id: 'T1',
    verdict: dogfoodVerdict('T1'),
    verify_rc: 0,
  })
  await writeWorkerResult(runDir, 'T2', {
    files_changed: [],
    out_of_bounds: [],
    status: 'no-op',
    task_id: 'T2',
    verdict: dogfoodVerdict('T2'),
    verify_rc: 0,
  })
  await writeWorkerResult(runDir, 'T3', {
    files_changed: ['src/bounds.ts', 'docs/outside.md'],
    out_of_bounds: ['docs/outside.md'],
    status: 'ok',
    task_id: 'T3',
    verdict: dogfoodVerdict('T3'),
    verify_rc: 0,
  })
  await writeWorkerResult(runDir, 'T4', {
    files_changed: ['src/weak.ts'],
    out_of_bounds: [],
    status: 'ok',
    task_id: 'T4',
    verdict: dogfoodVerdict('T4'),
    verify_rc: 0,
  })
  await writeWorkerResult(runDir, 'T5', {
    files_changed: ['src/failed.ts'],
    out_of_bounds: [],
    status: 'ok',
    task_id: 'T5',
    verdict: dogfoodVerdict('T5'),
    verify_rc: 2,
  })
  await writeEventsJsonl(runDir, [
    { payload: dogfoodVerdict('T1'), type: 'review_verdict' },
    {
      payload: { attempt: 1, task_id: 'T3', worker_id: 'worker-T3' },
      type: 'worker_started',
    },
    {
      payload: {
        attempt: 2,
        reason: 'progress-stall',
        task_id: 'T3',
        worker_id: 'worker-T3',
      },
      type: 'worker_restarted',
    },
    {
      payload: { attempt: 3, reason: 'loop', task_id: 'T3', worker_id: 'worker-T3' },
      type: 'worker_restarted',
    },
    {
      payload: {
        result_path: 'workers/T3/result.json',
        status: 'ok',
        task_id: 'T3',
        worker_id: 'worker-T3',
      },
      type: 'worker_finished',
    },
  ])
  return runDir
}

function dogfoodEvalTasks(): readonly FixtureTask[] {
  return [
    dogfoodTask('T1', 'Clean task', ['src/clean.ts'], ['the targeted unit verifies the clean path']),
    dogfoodTask('T2', 'No-op task', ['src/noop.ts'], ['the targeted unit verifies the no-op path']),
    dogfoodTask('T3', 'Boundary task', ['src/bounds.ts'], ['the targeted unit verifies bounds handling']),
    dogfoodTask('T4', 'Weak verify task', ['src/weak.ts'], []),
    dogfoodTask('T5', 'Failed verify task', ['src/failed.ts'], ['the targeted unit verifies failures']),
    dogfoodTask('T6', 'Missing result task', ['src/missing.ts'], ['the targeted unit verifies missing results']),
  ]
}

function dogfoodTask(
  id: string,
  title: string,
  paths: readonly string[],
  verifyProves: readonly string[],
): FixtureTask {
  return {
    acceptance_criteria: [`${title} is represented in eval output.`],
    boundaries: `Only touch ${paths.join(', ')}.`,
    depends_on: [],
    difficulty: 'moderate',
    id,
    model: 'sonnet',
    objective: `Exercise eval scoring for ${title}.`,
    output_format: 'Patch',
    paths,
    title,
    verify: 'npx vitest run src/workflows/eval.test.ts',
    verify_proves: verifyProves,
  }
}

function dogfoodVerdict(taskId: string): Record<string, unknown> {
  return {
    issues: [],
    reasons: 'fixture reviewer was satisfied',
    satisfied: true,
    task_id: taskId,
  }
}

async function writeEventsJsonl(
  runDir: string,
  events: readonly Record<string, unknown>[],
): Promise<void> {
  await writeFile(
    join(runDir, 'events.jsonl'),
    events.map((event) => `${JSON.stringify(event)}\n`).join(''),
    'utf8',
  )
}

async function writeLegacyOrdinalRun(runDir: string): Promise<void> {
  const tasks = legacyOrdinalTasks()
  await writeRunJson(runDir, 'state.json', {
    integration_branch: 'council/legacy-ordinal-ids/integration',
    intensity: 'quick',
    rounds: 1,
    spec_id: '001-legacy-ordinal-ids',
    spec_relpath: 'specs/001-legacy-ordinal-ids',
    spec_slug: 'legacy-ordinal-ids',
    stage: 'fanned-out',
    task_count: 2,
  })
  await writeRunJson(runDir, 'tasks.json', tasks)
  await writeRunJson(runDir, 'report.json', {
    integration_branch: 'council/legacy-ordinal-ids/integration',
    run: 'legacy-ordinal-ids',
    tasks: tasks.map((task) => ({ status: 'ok', task_id: task.id })),
    waves: [['T1'], ['T2']],
  })
  await writeWorkerResult(runDir, 'T1', {
    files_changed: ['legacy/t1.txt'],
    status: 'ok',
    task_id: 'T1',
  })
  await writeWorkerResult(runDir, 'T2', {
    files_changed: ['legacy/t2.txt'],
    status: 'ok',
    task_id: 'T2',
  })
}

async function writeWatchdogRun(runDir: string): Promise<void> {
  const tasks = [
    {
      acceptance_criteria: ['The [watchdog] table remains present.'],
      boundaries: 'Only touch config/service.toml.',
      depends_on: [],
      dev_notes: '[watchdog]\ninterval = "30s"',
      difficulty: 'trivial',
      id: 'ck-a100',
      model: 'haiku',
      objective: 'Exercise a task that mentions a [watchdog] TOML table.',
      output_format: 'Patch',
      paths: ['config/service.toml'],
      title: 'Touch watchdog table config',
      verify: "rg '\\[watchdog\\]' config/service.toml",
    },
  ]
  await writeRunJson(runDir, 'state.json', {
    integration_branch: 'council/watchdog-table-config/integration',
    intensity: 'quick',
    rounds: 1,
    spec_id: '001-watchdog-table-config',
    spec_relpath: 'specs/001-watchdog-table-config',
    spec_slug: 'watchdog-table-config',
    stage: 'fanned-out',
    task_count: 1,
  })
  await writeRunJson(runDir, 'tasks.json', tasks)
  await writeRunJson(runDir, 'report.json', {
    run: 'watchdog-table-config',
    tasks: [{ status: 'ok', task_id: 'ck-a100' }],
    waves: [['ck-a100']],
  })
  await writeWorkerResult(runDir, 'ck-a100', {
    status: 'ok',
    task_id: 'ck-a100',
  })
}

async function writeGrownSchemaRun(runDir: string): Promise<void> {
  const tasks = [
    {
      archetype: 'schema-maintenance',
      boundaries: 'Only touch schema/grown.json.',
      content_hash: 'sha256:fixture-grown-schema-task',
      context_profile: 'contracts',
      context_refs: ['kb://council/tasks/grown-schema'],
      depends_on: [],
      difficulty: 'moderate',
      engine: { cli: 'codex', model: 'gpt-5.5' },
      id: 'ck-b200',
      model: 'sonnet',
      model_tier: 'expensive',
      objective: 'Exercise a task with the grown task schema.',
      output_format: 'Patch',
      paths: ['schema/grown.json'],
      supersedes: [],
      title: 'Maintain grown schema',
      verify: 'test -f schema/grown.json',
    },
  ]
  await writeRunJson(runDir, 'state.json', {
    intensity: 'quick',
    rounds: 1,
    spec_id: '001-grown-schema-task',
    spec_relpath: 'specs/001-grown-schema-task',
    spec_slug: 'grown-schema-task',
    stage: 'fanned-out',
    task_count: 1,
  })
  await writeRunJson(runDir, 'tasks.json', tasks)
  await writeRunJson(runDir, 'report.json', {
    run: 'grown-schema-task',
    tasks: [{ status: 'ok', task_id: 'ck-b200' }],
    waves: [['ck-b200']],
  })
  await writeWorkerResult(runDir, 'ck-b200', {
    files_changed: ['schema/grown.json'],
    status: 'ok',
    task_id: 'ck-b200',
    verdict: {
      issues: [],
      reasons: 'fixture',
      satisfied: true,
    },
  })
}

function legacyOrdinalTasks(): readonly Record<string, unknown>[] {
  return [
    {
      boundaries: 'Only touch legacy/t1.txt.',
      depends_on: [],
      difficulty: 'trivial',
      id: 'T1',
      model: 'haiku',
      objective: 'Exercise a legacy T1 task id.',
      output_format: 'Patch',
      paths: ['legacy/t1.txt'],
      title: 'Legacy T1',
      verify: 'test -f legacy/t1.txt',
    },
    {
      boundaries: 'Only touch legacy/t2.txt.',
      depends_on: ['T1'],
      difficulty: 'trivial',
      id: 'T2',
      model: 'haiku',
      objective: 'Exercise a legacy T2 task id that depends on T1.',
      output_format: 'Patch',
      paths: ['legacy/t2.txt'],
      title: 'Legacy T2',
      verify: 'test -f legacy/t2.txt',
    },
  ]
}

async function writeWorkerResult(runDir: string, taskId: string, result: Record<string, unknown>): Promise<void> {
  await writeRunJson(join(runDir, 'workers', taskId), 'result.json', result)
}

async function writeRunJson(runDir: string, file: string, value: unknown): Promise<void> {
  await mkdir(runDir, { recursive: true })
  await writeFile(join(runDir, file), `${JSON.stringify(value, null, 2)}\n`, 'utf8')
}
