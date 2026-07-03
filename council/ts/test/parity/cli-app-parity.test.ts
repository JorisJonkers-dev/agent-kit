import { mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { describe, expect, it } from 'vitest'

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
import type { GhPort, GhPrRequest } from '../../src/ports/index.js'

const fixturesRoot = join(import.meta.dirname, '..', 'fixtures', 'python-runs')

describe('CLI composition', () => {
  it('registers the required command surface', () => {
    expect(commandRegistry().map((command) => command.name).sort()).toEqual([
      'amend',
      'config',
      'context',
      'design',
      'fanout',
      'fleet',
      'grill',
      'inject',
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
        runDir: join(fixturesRoot, 'legacy-ordinal-ids'),
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
        runDir: join(fixturesRoot, 'legacy-ordinal-ids'),
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
    const dir = await mkdtemp(join(tmpdir(), 'council-run-'))
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

    const dir = await mkdtemp(join(tmpdir(), 'council-config-'))
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
