import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  FakeEnv,
  FakeFiles,
  FakeGit,
  FakeGrants,
  FakeLedger,
  FakeWorkflows,
  FixedClock,
  type FakeRepository,
} from '../../../../test/model-gateway/harness.js'
import { isReservation, isRunRecord, isSettled, type LedgerRecord } from '../domain/ledger.js'
import { EngineGuard, type EngineAuthorizeRequest } from './engine-guard.js'
import { GrantListCommand, GrantWriteCommand } from './grants.js'
import type { ModelGatewayDependencies } from './ports.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

/** The defaults agent-kit ships, which the bundle reads from its own folder. */
const SHIPPED_DEFAULTS = join(import.meta.dirname, '../../../../../../skills/workstation/model-gateway/defaults.json')

const REPO = '/work/agent-kit'
const KEY = 'github.com/jorisjonkers-dev/agent-kit'
const REMOTES = [{ name: 'origin', url: 'git@github.com:JorisJonkers-dev/agent-kit.git' }]

interface Workspace {
  readonly clock: FixedClock
  readonly deps: ModelGatewayDependencies
  readonly guard: EngineGuard
  readonly ledger: FakeLedger
  readonly workflows: FakeWorkflows
  grant(workflow: string, cap: number): Promise<void>
  writeAllowlist(families: readonly string[], code?: boolean, history?: boolean): Promise<void>
  writeOverride(value: unknown): Promise<void>
}

async function workspace(): Promise<Workspace> {
  const dir = await mkdtemp(join(tmpdir(), 'model-gateway-engine-guard-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const home = join(dir, 'home')
  const defaultsPath = join(dir, 'defaults.json')
  await writeFile(defaultsPath, await readFile(SHIPPED_DEFAULTS, 'utf8'))
  const repositories: Readonly<Record<string, FakeRepository>> = { [REPO]: { remotes: REMOTES, root: REPO } }
  const clock = new FixedClock()
  const ledger = new FakeLedger()
  const workflows = new FakeWorkflows()
  const deps = {
    clock,
    defaultsPath,
    env: new FakeEnv({ HOME: home }),
    files: new FakeFiles(),
    git: new FakeGit(repositories),
    grants: new FakeGrants(),
    ledger,
    workflows,
  }
  const config = join(home, '.config', 'agent-kit')
  return {
    clock,
    deps: deps as unknown as ModelGatewayDependencies,
    grant: async (workflow, cap) => {
      await new GrantWriteCommand(deps as unknown as ModelGatewayDependencies).run({
        answer: 'yes, up to the cap',
        cap_usd: cap,
        client: 'codex',
        question: 'may codex work on this?',
        workflow,
      })
    },
    guard: new EngineGuard(deps),
    ledger,
    workflows,
    writeAllowlist: async (families, code = true, history = true) => {
      await mkdir(config, { recursive: true })
      await writeFile(
        join(config, 'critique-allowlist.json'),
        JSON.stringify({ repositories: { [KEY]: { code, families, history, updated_at: '2026-11-04T00:00:00.000Z' } }, version: 1 }),
      )
    },
    writeOverride: async (value) => {
      await mkdir(config, { recursive: true })
      await writeFile(join(config, 'model-gateway.json'), JSON.stringify(value))
    },
  }
}

function request(overrides: Partial<EngineAuthorizeRequest> = {}): EngineAuthorizeRequest {
  return { client: 'codex', model: 'gpt-5.5', prompt_chars: 3500, repo: REPO, workflow: 'run-1', ...overrides }
}

async function refusal(promise: Promise<unknown>): Promise<{ code: string; message: string }> {
  try {
    await promise
  } catch (error) {
    const { code, message } = error as { code: string; message: string }
    return { code, message }
  }
  throw new Error('expected the guard to refuse')
}

function failTest(message: string): never {
  throw new Error(message)
}

function runLines(ledger: FakeLedger): LedgerRecord[] {
  return ledger.records.filter(isRunRecord)
}

describe('EngineGuard.authorize', () => {
  it('refuses a codex run whose workflow has no live grant, with the gateway’s own reason and grant write hint', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'])

    const refused = await refusal(w.guard.authorize(request()))

    expect(refused.code).toBe('no-grant')
    expect(refused.message).toContain('no live grant covers workflow run-1')
    expect(refused.message).toContain('model-gateway grant write --json')
    expect(w.ledger.records).toEqual([])
    expect(await w.workflows.read()).toEqual([])
  })

  it('reserves the estimate against the grant, starts the workflow and hands back the reservation', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'])
    await w.grant('run-1', 5)

    const { reservation } = await w.guard.authorize(request())

    expect(reservation).toMatchObject({ client: 'codex', cost_class: 'metered-employer', family: 'openai', model: 'gpt-5.5', workflow: 'run-1' })
    expect(reservation.run_id).toMatch(/^20261104T105215Z-[0-9a-f]{6}$/u)
    expect(w.ledger.records.filter(isReservation)).toHaveLength(1)
    expect(w.ledger.records.find(isReservation)).toMatchObject({ client: 'codex', run_id: reservation.run_id, workflow: 'run-1' })
    expect((await w.workflows.read()).map((workflow) => workflow.name)).toEqual(['run-1'])
  })

  it('refuses a run that does not fit its grant with cap-exceeded', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'])
    await w.grant('run-1', 0.01)

    const refused = await refusal(w.guard.authorize(request({ prompt_chars: 350_000 })))

    expect(refused.code).toBe('cap-exceeded')
    expect(w.ledger.records.filter(isReservation)).toEqual([])
  })

  it('refuses a codex run the repository’s policy does not permit', async () => {
    const w = await workspace()
    await w.writeAllowlist(['anthropic'])
    await w.grant('run-1', 5)

    const refused = await refusal(w.guard.authorize(request()))

    expect(refused.code).toBe('policy-refused')
    expect(refused.message).toContain('codex:gpt-5.5 is not permitted')
  })

  it('refuses code to a repository whose entry does not let code leave', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'], false)
    await w.grant('run-1', 5)

    const refused = await refusal(w.guard.authorize(request()))

    expect(refused.code).toBe('policy-refused')
    expect(refused.message).toContain('may not carry code')
  })

  it('refuses a worker whose prompt may carry history to a repository whose entry does not let history leave, unless the caller says it carries none', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'], true, false)
    await w.grant('run-1', 5)

    const refused = await refusal(w.guard.authorize(request()))
    expect(refused.code).toBe('policy-refused')
    expect(refused.message).toContain('may not carry history')
    expect(w.ledger.records.filter(isReservation)).toEqual([])

    const allowed = await w.guard.authorize(request({ carries_history: false }))
    expect(allowed.reservation.client).toBe('codex')
  })

  it('refuses history to a run in no repository, where nothing may be sent', async () => {
    const w = await workspace()
    const { repo, ...inline } = request({ client: 'claude', model: 'claude-sonnet-5-5' })
    expect(repo).toBe(REPO)

    expect((await refusal(w.guard.authorize(inline))).code).toBe('policy-refused')
  })

  it('refuses a workflow name the gateway would not accept', async () => {
    const w = await workspace()

    expect((await refusal(w.guard.authorize(request({ workflow: 'has spaces' })))).code).toBe('invalid-input')
  })

  it('refuses a workflow that already ended', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'])
    await w.workflows.update(() => ({
      workflow: { ended_at: '2026-11-04T09:00:00.000Z', name: 'run-1', started_at: '2026-11-04T08:00:00.000Z' },
      workflows: [{ ended_at: '2026-11-04T09:00:00.000Z', name: 'run-1', started_at: '2026-11-04T08:00:00.000Z' }],
    }))

    expect((await refusal(w.guard.authorize(request()))).code).toBe('workflow-ended')
  })

  it('refuses a client or a model the catalogue does not know', async () => {
    const w = await workspace()

    expect((await refusal(w.guard.authorize(request({ client: 'mystery' })))).code).toBe('unknown-client')
    expect((await refusal(w.guard.authorize(request({ model: 'mystery-9', client: 'claude' })))).code).toBe('unknown-model')
  })

  it('refuses a metered run the catalogue cannot price', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'])
    await w.writeOverride({ models: [{ client: 'codex', family: 'openai', model: 'gpt-9', notes: 'new', tier: 'hard' }] })
    await w.grant('run-1', 5)

    expect((await refusal(w.guard.authorize(request({ model: 'gpt-9' })))).code).toBe('no-estimate')
  })

  it('lets a subsidised claude run through without a grant or a reservation, resolving the alias council uses', async () => {
    const w = await workspace()
    await w.writeAllowlist(['anthropic'])

    const { reservation } = await w.guard.authorize(request({ client: 'claude', model: 'haiku' }))

    expect(reservation).toMatchObject({ client: 'claude', cost_class: 'subsidised', family: 'anthropic' })
    expect(w.ledger.records).toEqual([])
    expect((await w.workflows.read()).map((workflow) => workflow.name)).toEqual(['run-1'])
  })

  it('refuses a subsidised run whose client is at 100% of a plan window, unable to start', async () => {
    const w = await workspace()
    await w.writeAllowlist(['anthropic'])
    await w.writeOverride({ clients: { claude: { plan_limits: [{ limit_runs: 1, window: 'day' }] } } })
    await w.ledger.append({
      at: '2026-11-04T09:00:00.000Z',
      client: 'claude',
      cost_usd: 0,
      duration_s: 1,
      family: 'anthropic',
      mode: 'write',
      model: 'claude-haiku-5-5',
      outcome: 'ok',
      repository: null,
      run_id: 'earlier',
      workflow: 'other',
    })

    const refused = await refusal(w.guard.authorize(request({ client: 'claude', model: 'claude-haiku-5-5' })))

    expect(refused.code).toBe('unable-to-start')
    expect(refused.message).toBe('claude:claude-haiku-5-5 cannot start: at 100% of its day plan limit (1 of 1 runs)')
  })

  it('refuses a model the machine’s configuration disabled', async () => {
    const w = await workspace()
    await w.writeOverride({ disabled: ['codex:gpt-5.5'] })

    const refused = await refusal(w.guard.authorize(request()))

    expect(refused.code).toBe('unknown-model')
    expect(refused.message).toContain('disabled')
  })

  it('gives the hold back when the workflow cannot be started, so nothing stays reserved', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'])
    await w.grant('run-1', 5)
    w.workflows.updateError = new Error('workflows file locked')

    await expect(w.guard.authorize(request())).rejects.toThrow('workflows file locked')

    const settled = w.ledger.records.filter(isSettled)
    expect(w.ledger.records.filter(isReservation)).toHaveLength(1)
    expect(settled).toHaveLength(1)
    expect(settled[0]).toMatchObject({ cost_usd: 0, reason: 'not-started' })
  })

  it('works without a repository, which the policy treats as inline content', async () => {
    const w = await workspace()

    const { repo, ...inline } = request({ carries_history: false, client: 'claude', model: 'claude-sonnet-5-5' })
    expect(repo).toBe(REPO)

    const { reservation } = await w.guard.authorize(inline)

    expect(reservation.repository).toBeNull()
  })
})

describe('EngineGuard.record', () => {
  it('writes the run’s ledger line as a write run from council and settles the reservation at what it cost', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'])
    await w.grant('run-1', 5)
    const { reservation } = await w.guard.authorize(request())

    await w.guard.record({
      duration_s: 42,
      outcome: 'ok',
      reservation,
      run_id: reservation.run_id,
      tokens: { cached: 0, input: 1000, output: 2000 },
    })

    expect(runLines(w.ledger)).toEqual([
      {
        at: '2026-11-04T10:52:15.000Z',
        client: 'codex',
        cost_class: 'metered-employer',
        cost_usd: 0.02125,
        duration_s: 42,
        family: 'openai',
        mode: 'write',
        model: 'gpt-5.5',
        outcome: 'ok',
        repository: KEY,
        run_id: reservation.run_id,
        source: 'council',
        tokens: { cached: 0, input: 1000, output: 2000 },
        workflow: 'run-1',
      },
    ])
    const settled = w.ledger.records.filter(isSettled)
    expect(settled).toHaveLength(1)
    expect(settled[0]).toMatchObject({ cost_usd: 0.02125, run_id: reservation.run_id })
    const grants = await new GrantListCommand(w.deps).run({})
    expect(grants.grants[0]).toMatchObject({ remaining_usd: 4.97875, reserved_usd: 0, spent_usd: 0.02125 })
  })

  it('takes the cost the engine reported over a count of tokens', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'])
    await w.grant('run-1', 5)
    const { reservation } = await w.guard.authorize(request())

    await w.guard.record({
      cost_usd: 0.5,
      duration_s: 3,
      outcome: 'ok',
      reservation,
      run_id: reservation.run_id,
      tokens: { cached: 0, input: 1, output: 1 },
    })

    expect(runLines(w.ledger)[0]?.cost_usd).toBe(0.5)
  })

  it('estimates the cost from the catalogue price when the engine reports neither tokens nor cost', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'])
    await w.grant('run-1', 5)
    const { reservation } = await w.guard.authorize(request())

    await w.guard.record({ duration_s: 3, outcome: 'ok', reservation, run_id: reservation.run_id })

    const line = runLines(w.ledger)[0]
    expect(line?.cost_usd).toBe(reservation.estimate_usd)
    expect(line?.cost_usd).toBeGreaterThan(0)
    expect(line).not.toHaveProperty('tokens')
  })

  it('records a failure with its reason, and a timeout as a failed run that timed out', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'])
    await w.grant('run-1', 5)
    const first = await w.guard.authorize(request())
    const second = await w.guard.authorize(request())

    await w.guard.record({ duration_s: 1, outcome: 'failed', reason: 'exit-2', reservation: first.reservation, run_id: first.reservation.run_id })
    await w.guard.record({ duration_s: 9, outcome: 'timeout', reservation: second.reservation, run_id: second.reservation.run_id })

    expect(runLines(w.ledger).map((line) => [line.outcome, line.reason])).toEqual([
      ['failed', 'exit-2'],
      ['failed', 'timeout'],
    ])
  })

  it('records a subsidised run, which holds no reservation, at no more than it reported', async () => {
    const w = await workspace()
    await w.writeAllowlist(['anthropic'])
    const { reservation } = await w.guard.authorize(request({ client: 'claude', model: 'claude-haiku-5-5' }))

    await w.guard.record({ cost_usd: 0.07, duration_s: 5, outcome: 'ok', reservation, run_id: reservation.run_id })

    expect(runLines(w.ledger)).toHaveLength(1)
    expect(runLines(w.ledger)[0]).toMatchObject({ client: 'claude', cost_class: 'subsidised', cost_usd: 0.07, source: 'council' })
    expect(w.ledger.records.filter(isSettled)).toEqual([])
  })

  it('marks a run that was re-attached to, so a reader can tell its start was not seen', async () => {
    const w = await workspace()
    await w.writeAllowlist(['anthropic'])
    const { reservation } = await w.guard.authorize(request({ client: 'claude', model: 'claude-haiku-5-5' }))

    await w.guard.record({ duration_s: 5, outcome: 'ok', reattached: true, reservation, run_id: reservation.run_id })

    expect(runLines(w.ledger)[0]).toMatchObject({ reattached: true })
  })

  it('counts a run once even when its settlement is missing, since its own ledger line already holds its spend', async () => {
    const w = await workspace()
    await w.writeAllowlist(['openai'])
    await w.grant('run-1', 5)
    const { reservation } = await w.guard.authorize(request())
    await w.guard.record({ cost_usd: 0.4, duration_s: 1, outcome: 'ok', reservation, run_id: reservation.run_id })
    w.ledger.records.splice(0, w.ledger.records.length, ...w.ledger.records.filter((entry) => !isSettled(entry)))

    const grants = await new GrantListCommand(w.deps).run({})

    expect(grants.grants[0]).toMatchObject({ reserved_usd: 0, spent_usd: 0.4 })
  })

  it('refuses a record that names another run than the one its reservation holds', async () => {
    const w = await workspace()
    await w.writeAllowlist(['anthropic'])
    const { reservation } = await w.guard.authorize(request({ client: 'claude', model: 'claude-haiku-5-5' }))

    expect((await refusal(w.guard.record({ duration_s: 1, outcome: 'ok', reservation, run_id: 'someone-else' }))).code).toBe('invalid-input')
    expect(w.ledger.records).toEqual([])
  })
})

describe('EngineGuard.reauthorize', () => {
  async function standing(w: Workspace, request: Partial<EngineAuthorizeRequest> = {}) {
    await w.writeAllowlist(['openai', 'anthropic'])
    if ((request.client ?? 'codex') === 'codex') await w.grant('run-1', 5)
    return (await w.guard.authorize({ ...baseRequest(), ...request })).reservation
  }

  function baseRequest(): EngineAuthorizeRequest {
    return request()
  }

  it('lets a run go on while its grant, its workflow and its repository’s policy still allow it', async () => {
    const w = await workspace()
    const reservation = await standing(w)

    await expect(w.guard.reauthorize({ repo: REPO, reservation })).resolves.toBeUndefined()
  })

  it('refuses a metered-employer run whose grant is gone since, with the gateway’s no-grant reason', async () => {
    const w = await workspace()
    const reservation = await standing(w)
    await w.deps.grants.update((grants) => ({ grant: grants[0] ?? failTest('no grant was written'), grants: [] }))

    const refused = await refusal(w.guard.reauthorize({ repo: REPO, reservation }))

    expect(refused.code).toBe('no-grant')
    expect(refused.message).toContain('model-gateway grant write --json')
  })

  it('refuses a metered-employer run whose grant has run out of room, since a restart would keep spending', async () => {
    const w = await workspace()
    const reservation = await standing(w)
    await w.guard.record({ cost_usd: 5, duration_s: 1, outcome: 'ok', reservation, run_id: reservation.run_id })

    expect((await refusal(w.guard.reauthorize({ repo: REPO, reservation }))).code).toBe('no-grant')
  })

  it('refuses a run whose workflow ended since', async () => {
    const w = await workspace()
    const reservation = await standing(w)
    await w.workflows.update((workflows) => ({
      workflow: { ended_at: '2026-11-04T11:00:00.000Z', name: 'run-1', started_at: workflows[0]?.started_at ?? '' },
      workflows: [{ ended_at: '2026-11-04T11:00:00.000Z', name: 'run-1', started_at: workflows[0]?.started_at ?? '' }],
    }))

    expect((await refusal(w.guard.reauthorize({ repo: REPO, reservation }))).code).toBe('workflow-ended')
  })

  it('refuses a run whose repository no longer allows its family, or its history', async () => {
    const w = await workspace()
    const reservation = await standing(w)
    await w.writeAllowlist(['anthropic'])

    expect((await refusal(w.guard.reauthorize({ repo: REPO, reservation }))).code).toBe('policy-refused')

    await w.writeAllowlist(['openai'], true, false)
    expect((await refusal(w.guard.reauthorize({ repo: REPO, reservation }))).message).toContain('may not carry history')
    await expect(w.guard.reauthorize({ carries_history: false, repo: REPO, reservation })).resolves.toBeUndefined()
  })

  it('refuses a subsidised run whose client reached its plan limit since', async () => {
    const w = await workspace()
    const reservation = await standing(w, { client: 'claude', model: 'claude-haiku-5-5' })
    await w.writeOverride({ clients: { claude: { plan_limits: [{ limit_runs: 1, window: 'day' }] } } })
    await w.guard.record({ duration_s: 1, outcome: 'ok', reservation, run_id: reservation.run_id })

    expect((await refusal(w.guard.reauthorize({ repo: REPO, reservation }))).code).toBe('unable-to-start')
  })

  it('lets a subsidised run with room go on, and refuses a client the configuration no longer declares', async () => {
    const w = await workspace()
    const reservation = await standing(w, { client: 'claude', model: 'claude-haiku-5-5' })

    await expect(w.guard.reauthorize({ repo: REPO, reservation })).resolves.toBeUndefined()
    expect((await refusal(w.guard.reauthorize({ repo: REPO, reservation: { ...reservation, client: 'mystery' } }))).code).toBe('unknown-client')
  })
})
