import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it, vi } from 'vitest'

import { runCli } from '../cli/index.js'
import { EngineGuard, ModelGatewayError, ledgerPath } from '../contexts/model-gateway/index.js'
import type { WorkerResult } from '../ports/index.js'
import {
  FakeEnv,
  FakeFiles,
  FakeGit,
  FakeGrants,
  FakeLedger,
  FakeWorkflows,
  FixedClock,
} from '../../test/model-gateway/harness.js'
import { FakeEngineGuard } from '../../test/council/fake-engine-guard.js'
import { GrantWriteCommand } from '../contexts/model-gateway/index.js'
import type { ModelGatewayDependencies } from '../contexts/model-gateway/index.js'
import { CouncilApp, defaultEngineGuard } from './council-app.js'
import type {
  SuperviseInput,
  SuperviseRunStore,
  SuperviseWorkerSupervisor,
  SuperviseWorkerSupervisorResult,
  SuperviseWorkerSupervisorSession,
  SuperviseWorkerSupervisorSnapshot,
  SuperviseWorkerSupervisorStartRequest,
} from './council-app.js'
import type { WorkerLifecycleEvent } from '../contexts/runstore/index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

/** The defaults agent-kit ships, which the gateway reads from its own skill folder. */
const SHIPPED_DEFAULTS = join(import.meta.dirname, '../../../../skills/workstation/model-gateway/defaults.json')

const WORKTREE = '/worktrees/T1'
const REMOTES = [{ name: 'origin', url: 'git@github.com:JorisJonkers-dev/agent-kit.git' }]

class MemoryStore implements SuperviseRunStore {
  readonly events: WorkerLifecycleEvent[] = []
  readonly results: WorkerResult[] = []
  snapshot: SuperviseWorkerSupervisorSnapshot | undefined
  /** What storing a result fails with, as a full disk does; none by default. */
  resultError: Error | undefined
  /** What appending the `worker_finished` event fails with; none by default. */
  eventError: Error | undefined

  appendWorkerEvent(_runId: string, event: WorkerLifecycleEvent): Promise<void> {
    if (this.eventError !== undefined && event.type === 'worker_finished') return Promise.reject(this.eventError)
    this.events.push(event)
    return Promise.resolve()
  }

  readEvents(): Promise<readonly never[]> {
    return Promise.resolve([])
  }

  readWorkerSupervisorSnapshot(): Promise<SuperviseWorkerSupervisorSnapshot> {
    if (this.snapshot !== undefined) return Promise.resolve(this.snapshot)
    return Promise.reject(Object.assign(new Error('missing snapshot'), { code: 'ENOENT' }))
  }

  writeWorkerResult(_runId: string, _taskId: string, result: WorkerResult): Promise<void> {
    if (this.resultError !== undefined) return Promise.reject(this.resultError)
    this.results.push(result)
    return Promise.resolve()
  }

  writeWorkerSupervisorSnapshot(): Promise<void> {
    return Promise.resolve()
  }
}

/** A supervisor that spawns nothing: it counts the starts it was asked for and answers with the result the test gives it. */
class CountingSupervisor implements SuperviseWorkerSupervisor {
  readonly starts: SuperviseWorkerSupervisorStartRequest[] = []
  readonly reattaches: SuperviseWorkerSupervisorStartRequest[] = []
  startError: Error | undefined
  /** A promise the worker's session waits on before it ends, so a test can look at the app while a worker runs. */
  hold: Promise<void> | undefined

  constructor(private readonly result: SuperviseWorkerSupervisorResult) {}

  start(request: SuperviseWorkerSupervisorStartRequest): SuperviseWorkerSupervisorSession {
    this.starts.push(request)
    if (this.startError !== undefined) throw this.startError
    const result = this.hold === undefined ? Promise.resolve(this.result) : this.hold.then(() => this.result)
    return { inject: () => Promise.reject(new Error('unused')), result, stop: () => Promise.resolve() }
  }

  reattach(request: SuperviseWorkerSupervisorStartRequest): SuperviseWorkerSupervisorSession {
    this.reattaches.push(request)
    return { inject: () => Promise.reject(new Error('unused')), result: Promise.resolve(this.result), stop: () => Promise.resolve() }
  }
}

function supervisorResult(
  status: SuperviseWorkerSupervisorResult['status'],
  exitCode: number | null = status === 'completed' ? 0 : 1,
): SuperviseWorkerSupervisorResult {
  return {
    exitCode,
    id: 'T1',
    restarts: 0,
    signal: null,
    status,
    stderr: '',
    stderrBytes: 0,
    stderrLogPath: 'workers/T1/logs/stderr.log',
    stdout: 'ok\n',
    stdoutBytes: 3,
    stdoutLogPath: 'workers/T1/logs/stdout.log',
  }
}

function input(overrides: Partial<SuperviseInput> = {}): SuperviseInput {
  return {
    args: ['exec', '-m', 'gpt-6.1-sol', '-'],
    command: 'codex',
    engine: 'codex:gpt-6.1-sol',
    runDir: '/runs/run-a',
    stdin: 'implement the ticket',
    taskId: 'T1',
    worktree: WORKTREE,
    ...overrides,
  }
}

interface Harness {
  readonly app: CouncilApp
  /** The files the app wrote and can read back, by path: the worker's lease file among them. */
  readonly files: Map<string, string>
  readonly store: MemoryStore
  readonly supervisor: CountingSupervisor
}

const LEASE_FILE = '/runs/run-a/workers/T1/engine-lease.json'

function harness(guard: FakeEngineGuard | EngineGuard | undefined, result = supervisorResult('completed')): Harness {
  const store = new MemoryStore()
  const supervisor = new CountingSupervisor(result)
  const files = new Map<string, string>()
  const app = new CouncilApp({
    createRunStore: () => store,
    createWorkerSupervisor: () => supervisor,
    nowIso: () => '2026-11-04T10:00:00.000Z',
    readText: (path) => {
      const text = files.get(path)
      return text === undefined ? Promise.reject(Object.assign(new Error(`no such file: ${path}`), { code: 'ENOENT' })) : Promise.resolve(text)
    },
    writeText: (path, text) => {
      files.set(path, text)
      return Promise.resolve()
    },
    ...(guard === undefined ? {} : { engineGuard: guard }),
  })
  return { app, files, store, supervisor }
}

/** The lease file of a worker whose first supervisor died with its run still open. */
function openLeaseFile(overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({
    lease: {
      council_run: 'run-a',
      engine: 'codex',
      model: 'gpt-6.1-sol',
      task_id: 'T1',
      reservation: { client: 'codex', cost_class: 'metered-employer', estimate_usd: 0.5, family: 'openai', model: 'gpt-6.1-sol', repository: null, run_id: 'dead-run-1', workflow: 'run-a' },
      started_at: '2026-11-04T09:00:00.000Z',
      ...overrides,
    },
    state: 'open',
  })
}

describe('CouncilApp.supervise through the gateway', () => {
  it('asks the gateway before it starts a codex worker, under the council run id and the worker’s repository', async () => {
    const guard = new FakeEngineGuard()
    const { app, supervisor } = harness(guard)

    await app.supervise(input())

    expect(guard.authorized).toEqual([
      // 20 characters of stdin and 4 + 2 + 7 + 1 of arguments.
      { carries_history: true, client: 'codex', model: 'gpt-6.1-sol', prompt_chars: 20 + 'exec'.length + '-m'.length + 'gpt-6.1-sol'.length + '-'.length, repo: WORKTREE, workflow: 'run-a' },
    ])
    expect(supervisor.starts).toHaveLength(1)
  })

  it('counts the prompt the caller names rather than guessing from stdin', async () => {
    const guard = new FakeEngineGuard()

    await harness(guard).app.supervise(input({ promptChars: 4321 }))

    expect(guard.authorized[0]?.prompt_chars).toBe(4321)
  })

  it('holds the worker to its wall-clock cap when the watchdog names one', async () => {
    const guard = new FakeEngineGuard()

    await harness(guard).app.supervise(input({ watchdog: { wallClockCapMs: 90_500 } }))

    expect(guard.authorized[0]).toMatchObject({ time_limit_s: 91 })
  })

  it('refuses a worker the gateway refuses, with the gateway’s reason, and spawns nothing', async () => {
    const guard = new FakeEngineGuard()
    guard.refusal = new ModelGatewayError('cap-exceeded', 'codex:gpt-6.1-sol does not fit grant g of $5.00')
    const { app, store, supervisor } = harness(guard)

    await expect(app.supervise(input())).rejects.toThrow('codex:gpt-6.1-sol does not fit grant g of $5.00')

    expect(supervisor.starts).toEqual([])
    expect(store.events).toEqual([])
    expect(store.results).toEqual([])
  })

  it('records an ok worker once its result is stored', async () => {
    const guard = new FakeEngineGuard()
    const { app, store } = harness(guard)

    await app.supervise(input())

    expect(store.results).toHaveLength(1)
    expect(guard.recorded).toHaveLength(1)
    expect(guard.recorded[0]).toMatchObject({ outcome: 'ok', run_id: 'fake-run-1' })
    expect(guard.recorded[0]).not.toHaveProperty('tokens')
    expect(guard.recorded[0]).not.toHaveProperty('cost_usd')
  })

  it.each([
    ['stalled', 1, 'timeout', 'stalled'],
    ['budget-cap', 0, 'timeout', 'budget-cap'],
    ['failed', 2, 'failed', 'exit-2'],
    ['stopped', null, 'failed', 'stopped'],
    ['disk-cap', 0, 'failed', 'disk-cap'],
  ] as const)('records a worker that ended %s as %s with its reason', async (status, exitCode, outcome, reason) => {
    const guard = new FakeEngineGuard()

    await harness(guard, supervisorResult(status, exitCode)).app.supervise(input())

    expect(guard.recorded[0]).toMatchObject({ outcome, reason })
  })

  it('records a worker whose supervisor throws as failed and raises the supervisor’s own error', async () => {
    const guard = new FakeEngineGuard()
    const { app, supervisor } = harness(guard)
    supervisor.startError = new Error('spawn codex ENOENT')

    await expect(app.supervise(input())).rejects.toThrow('spawn codex ENOENT')

    expect(guard.recorded[0]).toMatchObject({ outcome: 'failed', reason: 'supervise-error' })
  })

  it('keeps the supervisor’s error when the gateway cannot record it either', async () => {
    const guard = new FakeEngineGuard()
    guard.recordError = new Error('ledger locked')
    const { app, supervisor } = harness(guard)
    supervisor.startError = new Error('spawn codex ENOENT')

    await expect(app.supervise(input())).rejects.toThrow('spawn codex ENOENT')
  })

  it('raises a ledger that cannot be written, after the worker’s result is stored', async () => {
    const guard = new FakeEngineGuard()
    guard.recordError = new Error('ledger locked')
    const { app, store } = harness(guard)

    await expect(app.supervise(input())).rejects.toThrow('ledger locked')

    expect(store.results).toHaveLength(1)
  })

  it('leaves an engine the gateway does not guard alone, whatever its model is called', async () => {
    const guard = new FakeEngineGuard()

    await harness(guard).app.supervise(input({ command: 'node', engine: 'generic' }))
    await harness(guard).app.supervise(input({ command: 'node', engine: 'my-engine:m' }))

    expect(guard.authorized).toEqual([])
  })

  it('leaves a command that is no engine alone', async () => {
    const guard = new FakeEngineGuard()

    const { engine: _engine, ...unnamed } = input({ command: 'node' })
    expect(_engine).toBe('codex:gpt-6.1-sol')

    await harness(guard).app.supervise(unnamed)

    expect(guard.authorized).toEqual([])
  })

  it('refuses to start codex or claude by command alone, which would skip the gateway', async () => {
    const guard = new FakeEngineGuard()
    const { app, supervisor } = harness(guard)
    const { engine: _engine, ...unnamed } = input()
    expect(_engine).toBe('codex:gpt-6.1-sol')

    await expect(app.supervise(unnamed)).rejects.toThrow('codex runs through the model gateway: name it with --engine codex:<model>')
    await expect(app.supervise({ ...unnamed, command: '/usr/local/bin/claude' })).rejects.toThrow('claude runs through the model gateway')

    expect(supervisor.starts).toEqual([])
  })

  it('runs a command that is no guarded engine without a guard, as every non-engine worker does', async () => {
    const { app, supervisor } = harness(undefined)

    const { engine: _engine, ...unnamed } = input({ command: 'node' })
    expect(_engine).toBe('codex:gpt-6.1-sol')

    await app.supervise(unnamed)

    expect(supervisor.starts).toHaveLength(1)
  })

  it('refuses a codex or claude command when the app has no guard, rather than run it past the gateway', async () => {
    const { app, supervisor } = harness(undefined)

    await expect(app.supervise(input())).rejects.toMatchObject({ code: 'no-engine-guard' })
    await expect(app.supervise(input({ command: 'claude', engine: 'claude:haiku' }))).rejects.toThrow('no model gateway guard')

    expect(supervisor.starts).toEqual([])
  })

  it.each([
    ['a different guarded engine', { command: 'codex', engine: 'claude:haiku' }, 'engine claude does not match the command codex'],
    ['a engine that is not guarded', { command: 'codex', engine: 'generic:x' }, 'engine generic does not match the command codex'],
    ['a guarded engine named for a command that is none', { command: 'node', engine: 'codex:gpt-6.1-sol' }, 'engine codex does not match the command node'],
    ['no model', { command: 'codex', engine: 'codex' }, '--engine must name an engine and its model'],
    ['a model other than the one its arguments pick', { args: ['exec', '-m', 'gpt-9'], command: 'codex', engine: 'codex:gpt-6.1-sol' }, 'picks gpt-9 but --engine names gpt-6.1-sol'],
    ['a model other than the one its --model picks', { args: ['--model=opus'], command: 'claude', engine: 'claude:haiku' }, 'picks opus but --engine names haiku'],
  ] as const)('refuses %s, and spawns nothing', async (_name, overrides, message) => {
    const guard = new FakeEngineGuard()
    const { app, supervisor } = harness(guard)

    await expect(app.supervise(input(overrides as Partial<SuperviseInput>))).rejects.toThrow(message)

    expect(supervisor.starts).toEqual([])
    expect(guard.authorized).toEqual([])
  })

  it.each([
    ['env', ['codex', 'exec', '-']],
    ['env', ['FOO=1', '/usr/local/bin/claude', '-p']],
    ['sh', ['-c', 'codex exec -']],
    ['bash', ['-lc', 'cd /repo && claude -p']],
    ['zsh', ['-c', 'exec codex']],
    ['node', ['/opt/bin/codex']],
    ['npx', ['codex']],
    ['python', ['codex']],
    ['xargs', ['codex']],
    ['nohup', ['codex']],
    ['exec', ['codex']],
    ['command', ['claude']],
    ['time', ['codex']],
    ['sudo', ['-u', 'x', 'codex']],
    ['env', ['sh', '-c', 'codex exec']],
    ['git', ['codex']],
  ])('refuses %s %j, which would start a guarded engine past the gateway, and spawns nothing', async (command, args) => {
    const guard = new FakeEngineGuard()
    const { app, supervisor } = harness(guard)

    for (const engine of [undefined, 'generic:x', 'codex:gpt-6.1-sol']) {
      const { engine: dropped, ...rest } = input({ args, command })
      expect(dropped).toBe('codex:gpt-6.1-sol')
      await expect(app.supervise(engine === undefined ? rest : { ...rest, engine })).rejects.toMatchObject({ code: 'engine-wrapper' })
    }

    expect(supervisor.starts).toEqual([])
    expect(guard.authorized).toEqual([])
  })

  it('refuses a wrapper in an app with no guard too', async () => {
    const { app, supervisor } = harness(undefined)

    await expect(app.supervise(input({ args: ['codex'], command: 'env', engine: undefined as never }))).rejects.toMatchObject({ code: 'engine-wrapper' })

    expect(supervisor.starts).toEqual([])
  })

  it('names the wrapper and the engine it would start', async () => {
    const { app } = harness(new FakeEngineGuard())

    await expect(app.supervise(input({ args: ['-c', 'codex exec'], command: '/bin/sh' }))).rejects.toThrow('sh would start codex past the model gateway')
  })

  it.each([
    [['exec', '-c', 'model=gpt-9']],
    [['-c', 'model="gpt-9"']],
    [['--config', 'model=gpt-9']],
    [['--config=model=gpt-9']],
    [['--model=gpt-9']],
    [['-m', 'gpt-6.1-sol', '-c', 'model=gpt-9']],
  ])('refuses a codex command whose arguments %j run a model other than the authorised one', async (args) => {
    const guard = new FakeEngineGuard()
    const { app, supervisor } = harness(guard)

    await expect(app.supervise(input({ args }))).rejects.toMatchObject({ code: 'engine-model-override' })

    expect(supervisor.starts).toEqual([])
    expect(guard.authorized).toEqual([])
  })

  it.each([
    [['-p', 'fast']],
    [['--profile', 'x']],
    [['--profile=x']],
    [['-c', 'model_provider=oss']],
    [['--config', 'model_provider="x"']],
    [['-c', 'model=gpt-6.1-sol', '-p', 'x']],
  ])('refuses a codex command whose arguments %j could change the model or its provider', async (args) => {
    const { app, supervisor } = harness(new FakeEngineGuard())

    await expect(app.supervise(input({ args }))).rejects.toMatchObject({ code: 'engine-model-override' })

    expect(supervisor.starts).toEqual([])
  })

  it('lets a command pick the authorised model, however it says so, and set other config', async () => {
    const guard = new FakeEngineGuard()
    const { app, supervisor } = harness(guard)

    await app.supervise(input({ args: ['exec', '-c', 'model="gpt-6.1-sol"', '-c', 'model_reasoning_effort=high', '-m', 'gpt-6.1-sol', '-'] }))
    await app.supervise(input({ args: ['-p', '--model', 'haiku'], command: 'claude', engine: 'claude:haiku' }))

    expect(supervisor.starts).toHaveLength(2)
  })

  it('refuses claude with a profile', async () => {
    const { app } = harness(new FakeEngineGuard())

    await expect(app.supervise(input({ args: ['--profile', 'x'], command: 'claude', engine: 'claude:haiku' }))).rejects.toMatchObject({ code: 'engine-model-override' })
  })

  it.each([
    ['another council run', { council_run: 'run-b' }],
    ['another task', { task_id: 'T9' }],
    ['another engine', { engine: 'claude' }],
    ['another model', { model: 'gpt-9' }],
  ])('refuses to resume a lease copied from %s, and touches nothing', async (_name, overrides) => {
    const guard = new FakeEngineGuard()
    const { app, files, store, supervisor } = harness(guard)
    store.snapshot = { attempt_id: 1, pid: 1, status: 'running', task_id: 'T1' } as unknown as SuperviseWorkerSupervisorSnapshot
    files.set(LEASE_FILE, openLeaseFile(overrides))

    await expect(app.supervise(input())).rejects.toMatchObject({ code: 'engine-lease-mismatch' })

    expect(supervisor.reattaches).toEqual([])
    expect(guard.reauthorized).toEqual([])
    expect(guard.recorded).toEqual([])
    expect(files.get(LEASE_FILE)).toBe(openLeaseFile(overrides))
  })

  it('still raises the gateway’s reason when the ended mark of a refused lease cannot be written', async () => {
    const guard = new FakeEngineGuard()
    guard.reauthorizeRefusal = new ModelGatewayError('no-grant', 'no live grant covers workflow run-a')
    const store = new MemoryStore()
    store.snapshot = { attempt_id: 1, pid: 1, status: 'running', task_id: 'T1' } as unknown as SuperviseWorkerSupervisorSnapshot
    const app = new CouncilApp({
      createRunStore: () => store,
      createWorkerSupervisor: () => new CountingSupervisor(supervisorResult('completed')),
      engineGuard: guard,
      readText: () => Promise.resolve(openLeaseFile()),
      writeText: () => Promise.reject(new Error('read-only file system')),
    })

    await expect(app.supervise(input())).rejects.toThrow('no live grant covers workflow run-a')
  })

  it('asks the gateway again before it resumes a lease, for the same repository and history', async () => {
    const guard = new FakeEngineGuard()
    const { app, files, store } = harness(guard)
    store.snapshot = { attempt_id: 1, pid: 1, status: 'running', task_id: 'T1' } as unknown as SuperviseWorkerSupervisorSnapshot
    files.set(LEASE_FILE, openLeaseFile())

    await app.supervise(input({ noHistory: true }))

    expect(guard.reauthorized).toEqual([{ carries_history: false, repo: WORKTREE, reservation: expect.objectContaining({ run_id: 'dead-run-1' }) as unknown }])
  })

  it('refuses to resume a worker whose grant lapsed, ends its lease as failed with the gateway’s reason, and spawns nothing', async () => {
    const guard = new FakeEngineGuard()
    guard.reauthorizeRefusal = new ModelGatewayError('no-grant', 'no live grant covers workflow run-a')
    const { app, files, store, supervisor } = harness(guard)
    store.snapshot = { attempt_id: 1, pid: 1, status: 'running', task_id: 'T1' } as unknown as SuperviseWorkerSupervisorSnapshot
    files.set(LEASE_FILE, openLeaseFile())

    await expect(app.supervise(input())).rejects.toThrow('no live grant covers workflow run-a')

    expect(supervisor.reattaches).toEqual([])
    expect(guard.recorded).toHaveLength(1)
    expect(guard.recorded[0]).toMatchObject({ outcome: 'failed', reason: 'no-grant', run_id: 'dead-run-1' })
    expect(JSON.parse(files.get(LEASE_FILE) ?? '')).toEqual({ state: 'ended' })
  })

  it('says a worker carries no history only when its caller does', async () => {
    const guard = new FakeEngineGuard()

    await harness(guard).app.supervise(input({ noHistory: true }))

    expect(guard.authorized[0]).toMatchObject({ carries_history: false })
  })

  it('counts the prompt file the command is told to read, so the estimate is not smaller than the prompt', async () => {
    const guard = new FakeEngineGuard()
    const { app, files } = harness(guard)
    files.set(`${WORKTREE}/.council/prompt.md`, 'x'.repeat(500))
    files.set('/abs/prompt.md', 'y'.repeat(70))

    await app.supervise(input({ args: ['exec', '--prompt-file', '.council/prompt.md'], stdin: '' }))
    await app.supervise(input({ args: ['--prompt-file', '/abs/prompt.md'], stdin: '' }))

    expect(guard.authorized[0]?.prompt_chars).toBe(500 + 'exec'.length + '--prompt-file'.length + '.council/prompt.md'.length)
    expect(guard.authorized[1]?.prompt_chars).toBe(70 + '--prompt-file'.length + '/abs/prompt.md'.length)
  })

  it('refuses a worker whose prompt file cannot be read, since its size is unknown', async () => {
    const guard = new FakeEngineGuard()
    const { app, supervisor } = harness(guard)

    await expect(app.supervise(input({ args: ['--prompt-file', 'missing.md'] }))).rejects.toThrow('no such file')

    expect(supervisor.starts).toEqual([])
  })

  it('never counts less than the length the caller names', async () => {
    const guard = new FakeEngineGuard()
    const { app, files } = harness(guard)
    files.set(`${WORKTREE}/p.md`, 'x'.repeat(10))

    await app.supervise(input({ args: ['--prompt-file', 'p.md'], promptChars: 9000 }))

    expect(guard.authorized[0]?.prompt_chars).toBe(9000)
  })

  it('records a worker as failed when its result cannot be stored, and still tells the gateway', async () => {
    const guard = new FakeEngineGuard()
    const { app, store } = harness(guard)
    store.resultError = new Error('disk full')

    await expect(app.supervise(input())).rejects.toThrow('disk full')

    expect(guard.recorded).toHaveLength(1)
    expect(guard.recorded[0]).toMatchObject({ outcome: 'failed', reason: 'store-error' })
  })

  it('records a worker as failed when its events cannot be stored either', async () => {
    const guard = new FakeEngineGuard()
    const { app, store } = harness(guard)
    store.eventError = new Error('log closed')

    await expect(app.supervise(input())).rejects.toThrow('log closed')

    expect(guard.recorded[0]).toMatchObject({ outcome: 'failed', reason: 'store-error' })
  })

  it('writes the lease down when the worker is authorised, so a later process can end it', async () => {
    const guard = new FakeEngineGuard()
    let release: () => void = () => undefined
    const { app, files, supervisor } = harness(guard)
    supervisor.hold = new Promise((resolve) => {
      release = resolve
    })

    const running = app.supervise(input())
    await vi.waitFor(() => {
      expect(files.has(LEASE_FILE)).toBe(true)
    })

    expect(JSON.parse(files.get(LEASE_FILE) ?? '')).toMatchObject({ lease: { reservation: { run_id: 'fake-run-1' } }, state: 'open' })
    expect(guard.recorded).toEqual([])
    release()
    await running
    expect(JSON.parse(files.get(LEASE_FILE) ?? '')).toEqual({ state: 'ended' })
    expect(guard.recorded).toHaveLength(1)
  })

  it('ends the lease of the worker it re-attaches to, as the gateway holds it, and asks nothing new', async () => {
    const guard = new FakeEngineGuard()
    const { app, files, store, supervisor } = harness(guard)
    store.snapshot = { attempt_id: 1, pid: 1, status: 'running', task_id: 'T1' } as unknown as SuperviseWorkerSupervisorSnapshot
    files.set(LEASE_FILE, openLeaseFile())

    await app.supervise(input())

    expect(supervisor.reattaches).toHaveLength(1)
    expect(guard.authorized).toEqual([])
    expect(guard.recorded).toHaveLength(1)
    expect(guard.recorded[0]).toMatchObject({ outcome: 'ok', run_id: 'dead-run-1' })
    expect(guard.recorded[0]?.duration_s).toBeGreaterThan(0)
    expect(JSON.parse(files.get(LEASE_FILE) ?? '')).toEqual({ state: 'ended' })
  })

  it('opens a lease marked as re-attached for a worker whose first lease was never written', async () => {
    const guard = new FakeEngineGuard()
    const { app, files, store } = harness(guard)
    store.snapshot = { attempt_id: 1, pid: 1, status: 'running', task_id: 'T1' } as unknown as SuperviseWorkerSupervisorSnapshot

    await app.supervise(input())

    expect(guard.authorized).toHaveLength(1)
    expect(guard.recorded[0]).toMatchObject({ outcome: 'ok', reattached: true })
    expect(JSON.parse(files.get(LEASE_FILE) ?? '')).toEqual({ state: 'ended' })
  })

  it('does not record a worker twice when it is re-attached to after its run was already recorded', async () => {
    const guard = new FakeEngineGuard()
    const { app, files, store, supervisor } = harness(guard)
    store.snapshot = { attempt_id: 1, pid: 1, status: 'running', task_id: 'T1' } as unknown as SuperviseWorkerSupervisorSnapshot
    files.set(LEASE_FILE, JSON.stringify({ state: 'ended' }))

    await app.supervise(input())

    expect(supervisor.reattaches).toHaveLength(1)
    expect(guard.authorized).toEqual([])
    expect(guard.recorded).toEqual([])
  })

  it('refuses to re-attach to a worker whose lease file it cannot read, rather than run it unrecorded', async () => {
    const guard = new FakeEngineGuard()
    const { app, files, store, supervisor } = harness(guard)
    store.snapshot = { attempt_id: 1, pid: 1, status: 'running', task_id: 'T1' } as unknown as SuperviseWorkerSupervisorSnapshot
    files.set(LEASE_FILE, 'not json')

    await expect(app.supervise(input())).rejects.toThrow('not JSON')

    expect(supervisor.reattaches).toEqual([])
  })

  it('refuses to re-attach to a worker whose lease file cannot be read for any reason but being absent', async () => {
    const guard = new FakeEngineGuard()
    const store = new MemoryStore()
    store.snapshot = { attempt_id: 1, pid: 1, status: 'running', task_id: 'T1' } as unknown as SuperviseWorkerSupervisorSnapshot
    const supervisor = new CountingSupervisor(supervisorResult('completed'))
    const app = new CouncilApp({
      createRunStore: () => store,
      createWorkerSupervisor: () => supervisor,
      engineGuard: guard,
      readText: () => Promise.reject(Object.assign(new Error('permission denied'), { code: 'EACCES' })),
      writeText: () => Promise.resolve(),
    })

    await expect(app.supervise(input())).rejects.toThrow('permission denied')

    expect(supervisor.reattaches).toEqual([])
    expect(guard.authorized).toEqual([])
  })

  it('refuses a worker whose lease cannot be written down, and records the run that was authorised as failed', async () => {
    const guard = new FakeEngineGuard()
    const store = new MemoryStore()
    const supervisor = new CountingSupervisor(supervisorResult('completed'))
    const app = new CouncilApp({
      createRunStore: () => store,
      createWorkerSupervisor: () => supervisor,
      engineGuard: guard,
      writeText: () => Promise.reject(new Error('read-only file system')),
    })

    await expect(app.supervise(input())).rejects.toThrow('read-only file system')

    expect(supervisor.starts).toEqual([])
    expect(guard.recorded[0]).toMatchObject({ outcome: 'failed', reason: 'lease-not-persisted' })
  })

  it('still says why the lease could not be written when the gateway cannot take the failed run either', async () => {
    const guard = new FakeEngineGuard()
    guard.recordError = new Error('ledger locked')
    const app = new CouncilApp({
      createRunStore: () => new MemoryStore(),
      createWorkerSupervisor: () => new CountingSupervisor(supervisorResult('completed')),
      engineGuard: guard,
      writeText: () => Promise.reject(new Error('read-only file system')),
    })

    await expect(app.supervise(input())).rejects.toThrow('read-only file system')
  })
})

describe('CouncilApp.supervise with the gateway’s own guard', () => {
  interface Gateway {
    readonly guard: EngineGuard
    readonly ledger: FakeLedger
    grant(workflow: string, cap: number): Promise<void>
  }

  async function gateway(families: readonly string[], history = true): Promise<Gateway> {
    const dir = await mkdtemp(join(tmpdir(), 'council-engine-guard-'))
    cleanups.push(() => rm(dir, { force: true, recursive: true }))
    const home = join(dir, 'home')
    const defaultsPath = join(dir, 'defaults.json')
    await writeFile(defaultsPath, await readFile(SHIPPED_DEFAULTS, 'utf8'))
    await mkdir(join(home, '.config', 'agent-kit'), { recursive: true })
    await writeFile(
      join(home, '.config', 'agent-kit', 'critique-allowlist.json'),
      JSON.stringify({
        repositories: { 'github.com/jorisjonkers-dev/agent-kit': { code: true, families, history, updated_at: '2026-11-04T00:00:00.000Z' } },
        version: 1,
      }),
    )
    const ledger = new FakeLedger()
    const deps = {
      clock: new FixedClock(),
      defaultsPath,
      env: new FakeEnv({ HOME: home }),
      files: new FakeFiles(),
      git: new FakeGit({ [WORKTREE]: { remotes: REMOTES, root: WORKTREE } }),
      grants: new FakeGrants(),
      ledger,
      workflows: new FakeWorkflows(),
    }
    return {
      grant: async (workflow, cap) => {
        await new GrantWriteCommand(deps as unknown as ModelGatewayDependencies).run({
          answer: 'yes',
          cap_usd: cap,
          client: 'codex',
          question: 'may the council run use codex?',
          workflow,
        })
      },
      guard: new EngineGuard(deps),
      ledger,
    }
  }

  it('refuses codex without a live grant for the run’s workflow, with the gateway’s own reason, and spawns nothing', async () => {
    const g = await gateway(['openai'])
    const { app, store, supervisor } = harness(g.guard)

    const error = await app.supervise(input()).then(
      () => undefined,
      (caught: unknown) => caught as ModelGatewayError,
    )

    expect(error).toBeInstanceOf(ModelGatewayError)
    expect(error?.code).toBe('no-grant')
    expect(error?.message).toContain('no live grant covers workflow run-a, so codex may not run')
    expect(error?.message).toContain('model-gateway grant write --json')
    expect(supervisor.starts).toEqual([])
    expect(store.events).toEqual([])
    expect(g.ledger.records).toEqual([])
  })

  it('takes the prompt length and the no-history mark from the command line', async () => {
    const guard = new FakeEngineGuard()
    const { app } = harness(guard)

    await runCli(
      ['supervise', '--run', '/runs/run-a', '--task', 'T1', '--worktree', WORKTREE, '--command', 'codex', '--engine', 'codex:gpt-6.1-sol', '--prompt-chars', '7777', '--no-history', '--', 'exec', '-'],
      { app },
    )

    expect(guard.authorized[0]).toMatchObject({ carries_history: false, prompt_chars: 7777 })
  })

  it('refuses the same worker from the command line with the gateway’s reason on stderr and a failing exit code', async () => {
    const g = await gateway(['openai'])
    const { app, supervisor } = harness(g.guard)

    const result = await runCli(
      ['supervise', '--run', '/runs/run-a', '--task', 'T1', '--worktree', WORKTREE, '--command', 'codex', '--engine', 'codex:gpt-6.1-sol', '--stdin', 'go', '--', 'exec', '-'],
      { app },
    )

    expect(result.exitCode).toBe(2)
    expect(result.stderr).toContain('no live grant covers workflow run-a')
    expect(supervisor.starts).toEqual([])
  })

  it('refuses a codex worker whose repository does not let history leave, since its prompt carries ticket text', async () => {
    const g = await gateway(['openai'], false)
    await g.grant('run-a', 5)
    const { app, supervisor } = harness(g.guard)

    await expect(app.supervise(input())).rejects.toThrow('may not carry history')

    expect(supervisor.starts).toEqual([])
    expect(g.ledger.records.filter((entry) => 'kind' in entry && entry.kind === 'reservation')).toEqual([])
  })

  it('starts the worker once the owner’s grant covers the run, and puts it in the gateway ledger as a council write run', async () => {
    const g = await gateway(['openai'])
    await g.grant('run-a', 5)
    const { app, supervisor } = harness(g.guard)

    await app.supervise(input())

    expect(supervisor.starts).toHaveLength(1)
    const lines = g.ledger.records.filter((entry) => !('kind' in entry))
    expect(lines).toHaveLength(1)
    expect(lines[0]).toMatchObject({
      client: 'codex',
      mode: 'write',
      model: 'gpt-6.1-sol',
      outcome: 'ok',
      repository: 'github.com/jorisjonkers-dev/agent-kit',
      source: 'council',
      workflow: 'run-a',
    })
    expect((lines[0] as { cost_usd: number }).cost_usd).toBeGreaterThan(0)
  })

  it('names the same workflow for a run id the gateway’s name rule would refuse', async () => {
    const g = await gateway(['openai'])
    await g.grant('my-run--1-', 5)
    const { app, supervisor } = harness(g.guard)

    await app.supervise(input({ runDir: '/runs/my run (1)' }))

    expect(supervisor.starts).toHaveLength(1)
  })
})

describe('CouncilApp native DAG workers through the gateway', () => {
  /** One DAG worker supervised by the app's own seam, as the native executor calls it, on an engine the test names. */
  async function superviseDagWorker(app: CouncilApp, kind: string, model: string): Promise<unknown> {
    const task = {
      boundaries: 'stay in scope',
      depends_on: [],
      difficulty: 'moderate',
      id: 'T1',
      model: 'haiku',
      objective: 'do the thing',
      output_format: 'edits',
      paths: ['a.ts'],
      title: 'T1',
      verify: 'npm test',
    }
    const run = (app as unknown as {
      superviseDagWorker(runDir: string, dag: unknown, repoRoot: string, request: unknown): Promise<unknown>
    }).superviseDagWorker.bind(app)
    return run(
      '/runs/run-a',
      { agent_pool: { available: [{ id: 'agent-1', kind, model }], assignments: [] } },
      '/repo',
      { assignment: { agent_id: 'agent-1', model, task_id: 'T1' }, branch: 'worker/T1', dry_run: false, run_id: 'run-a', task, worktree_path: WORKTREE },
    )
  }

  function dagApp(guard: FakeEngineGuard): { app: CouncilApp; supervisor: CountingSupervisor } {
    const supervisor = new CountingSupervisor(supervisorResult('completed'))
    const app = new CouncilApp({
      createRunStore: () => new MemoryStore(),
      createWorkerSupervisor: () => supervisor,
      engineGuard: guard,
      writeText: () => Promise.resolve(),
    })
    return { app, supervisor }
  }

  it('authorises a codex worker the executor starts, counting the prompt it wrote to the worktree', async () => {
    const guard = new FakeEngineGuard()
    const { app, supervisor } = dagApp(guard)

    await superviseDagWorker(app, 'codex', 'gpt-6.1-sol')

    expect(guard.authorized).toHaveLength(1)
    expect(guard.authorized[0]).toMatchObject({ client: 'codex', model: 'gpt-6.1-sol', repo: WORKTREE, workflow: 'run-a' })
    expect(guard.authorized[0]?.prompt_chars).toBeGreaterThan(100)
    expect(guard.recorded).toHaveLength(1)
    expect(supervisor.starts).toHaveLength(1)
  })

  it('refuses a codex worker the gateway refuses before the executor’s worker is spawned', async () => {
    const guard = new FakeEngineGuard()
    guard.refusal = new ModelGatewayError('no-grant', 'no live grant covers workflow run-a')
    const { app, supervisor } = dagApp(guard)

    await expect(superviseDagWorker(app, 'codex', 'gpt-6.1-sol')).rejects.toThrow('no live grant covers workflow run-a')

    expect(supervisor.starts).toEqual([])
  })

  it('leaves an engine the gateway does not guard to council', async () => {
    const guard = new FakeEngineGuard()
    const { app, supervisor } = dagApp(guard)

    await expect(superviseDagWorker(app, 'cursor', 'x')).rejects.toThrow('Unknown engine: cursor')

    expect(guard.authorized).toEqual([])
    expect(supervisor.starts).toEqual([])
  })
})

describe('defaultEngineGuard', () => {
  it('is the gateway’s guard on the real ledger, wired from the environment it is given and the defaults beside the skill', async () => {
    const home = await mkdtemp(join(tmpdir(), 'council-engine-guard-home-'))
    cleanups.push(() => rm(home, { force: true, recursive: true }))
    const env = new FakeEnv({ HOME: home })
    const guard = defaultEngineGuard({ clock: new FixedClock(), defaultsPath: SHIPPED_DEFAULTS, env })

    const { reservation } = await guard.authorize({ carries_history: false, client: 'claude', model: 'haiku', prompt_chars: 10, workflow: 'run-a' })
    await guard.record({ duration_s: 1, outcome: 'ok', reservation, run_id: reservation.run_id })

    expect((await readFile(ledgerPath(env), 'utf8')).trim().split('\n')).toHaveLength(1)
  })

  it('looks for the defaults in the model-gateway skill beside the council skill when it is told nothing', async () => {
    await expect(
      defaultEngineGuard().authorize({ client: 'codex', model: 'gpt-6.1-sol', prompt_chars: 1, workflow: 'run-a' }),
    ).rejects.toMatchObject({ code: 'invalid-config' })
  })
})
