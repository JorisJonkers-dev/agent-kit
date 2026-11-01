import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  agyFindings,
  agyResult,
  agyStream,
  fenced,
  harness,
  isolatedExport,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { SetupCheckOutput, SetupItem, SetupListResult, SetupQuotaOutput } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

type HarnessOptions = NonNullable<Parameters<typeof harness>[0]>

async function machine(options: HarnessOptions = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  return h
}

async function listed(h: Harness, input: Record<string, unknown> = {}): Promise<SetupListResult> {
  const result = await h.commands.setup.run({ action: 'list', ...input })
  if (result.status !== 'listed') throw new Error(`not listed: ${JSON.stringify(result)}`)
  return result
}

async function list(h: Harness): Promise<readonly SetupItem[]> {
  return (await listed(h)).items
}

async function check(
  h: Harness,
  id: string,
  extra: Record<string, unknown> = {},
): Promise<SetupCheckOutput> {
  const result = await h.commands.setup.run({ action: 'check', check: id, ...extra })
  if (result.status === 'listed' || result.status === 'quota-set' || result.status === 'kill-switch-set')
    throw new Error(`not a check: ${JSON.stringify(result)}`)
  return result
}

async function setQuota(h: Harness, input: Record<string, unknown>): Promise<SetupQuotaOutput> {
  const result = await h.commands.setup.run({ action: 'quota', ...input })
  if (result.status !== 'quota-set') throw new Error(`quota not set: ${JSON.stringify(result)}`)
  return result
}

function item(items: readonly SetupItem[], id: string): SetupItem {
  const found = items.find((entry) => entry.id === id)
  if (found === undefined) throw new Error(`no item ${id}`)
  return found
}

/** A Vibe critic that answers setup's trivial prompt with an empty findings list. */
function vibeAnswers(agent: string): VibeScript {
  return {
    agent,
    exitCode: 0,
    exportJson: { ...isolatedExport({ active_model: agent }), cost_usd: 0.0042 },
    stdout: vibeStdout(
      fenced({ findings: [], no_findings_reason: 'setup check: nothing to review' }),
    ),
  }
}

const AGY_ANSWERS = agyFindings({
  findings: [],
  no_findings_reason: 'setup check: nothing to review',
})

function dataDir(h: Harness): string {
  return join(h.home, '.local', 'share', 'agent-kit', 'critique')
}

function overridePath(h: Harness): string {
  return join(h.home, '.config', 'agent-kit', 'critique-routing.json')
}

async function ledgerLines(h: Harness): Promise<readonly unknown[]> {
  const text = await h.readText(join(dataDir(h), 'ledger.jsonl'))
  return text
    .split('\n')
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line) as unknown)
}

describe("setup's machine checklist", () => {
  it('lists every check in the order setup walks them, Vibe first, then Antigravity, the read-only proofs, then the hooks', async () => {
    const h = await machine()

    expect((await list(h)).map((entry) => entry.id)).toEqual([
      'vibe-signed-in',
      'critic:vibe-mistral-large',
      'critic:vibe-glm',
      'critic:vibe-deepseek-flash',
      'critic:vibe-mistral-small',
      'agy-signed-in',
      'critic:agy-gemini-flash-high',
      'agy-quota',
      'read-only:vibe-mistral-large',
      'read-only:agy-gemini-flash-high',
      'read-only:vibe-glm',
      'read-only:vibe-deepseek-flash',
      'read-only:vibe-mistral-small',
      'hooks-installed',
      'kill-switch',
    ])
  })

  it('says what each item costs: sign-ins and the quota are free, a critic spends on its own CLI', async () => {
    const h = await machine()

    const costs = Object.fromEntries((await list(h)).map((entry) => [entry.id, entry.cost]))

    expect(costs).toMatchObject({
      'agy-quota': 'free',
      'agy-signed-in': 'free',
      'critic:agy-gemini-flash-high': 'agy',
      'critic:vibe-glm': 'vibe',
      'vibe-signed-in': 'free',
    })
  })

  it('gives every item one sentence on why it matters and a fix to offer', async () => {
    const h = await machine()

    const items = await list(h)

    expect(
      items.filter(
        (entry) =>
          entry.why.split('. ').length !== 1 || !entry.why.endsWith('.') || entry.fix.length === 0,
      ),
    ).toEqual([])
  })

  it('marks every check never checked on a fresh machine', async () => {
    const h = await machine()

    const checks = (await list(h)).filter((entry) => entry.kind !== 'read-only')

    expect(checks.map((entry) => [entry.status, entry.due_because])).toEqual(
      Array.from({ length: 10 }, () => ['due', 'never-checked']),
    )
  })

  it('launches no critic while listing', async () => {
    const h = await machine()

    await list(h)

    expect([h.process.commands, h.agy.launches]).toEqual([[], []])
  })

  it('reads each CLI version as an input its checks depend on', async () => {
    const h = await machine({ agyVersion: '1.4.12', vibeVersion: 'vibe 3.0.4' })

    const items = await list(h)

    expect([item(items, 'vibe-signed-in').inputs, item(items, 'agy-signed-in').inputs]).toEqual([
      { cli_version: 'vibe 3.0.4' },
      { cli_version: '1.4.12' },
    ])
  })

  it('reads no version for a CLI that is not installed', async () => {
    const h = await machine({ vibeOnPath: false })

    expect(item(await list(h), 'vibe-signed-in').inputs).toEqual({ cli_version: null })
  })

  it('leaves out the Antigravity items when routing has no agy client', async () => {
    const h = await machine({
      routing: {
        clients: {
          'vibe-glm': {
            agent: 'critic-glm',
            binary: 'vibe',
            family: 'zhipu',
            kind: 'vibe',
            model: 'glm-5',
          },
        },
        stages: {},
        version: 1,
      },
    })

    expect((await list(h)).map((entry) => entry.id)).toEqual([
      'vibe-signed-in',
      'critic:vibe-glm',
      'read-only:vibe-glm',
      'hooks-installed',
      'kill-switch',
    ])
  })

  it('leaves out the Vibe items when routing has no Vibe client', async () => {
    const h = await machine({
      routing: {
        clients: {
          'agy-gemini-flash-high': {
            binary: 'agy',
            family: 'google',
            kind: 'agy',
            model: 'gemini-3.8-flash-high',
          },
        },
        stages: {},
        version: 1,
      },
    })

    expect((await list(h)).map((entry) => entry.id)).toEqual([
      'agy-signed-in',
      'critic:agy-gemini-flash-high',
      'agy-quota',
      'read-only:agy-gemini-flash-high',
      'hooks-installed',
      'kill-switch',
    ])
  })
})

describe('the read-only proof', () => {
  it('is not available until #178 lands, for every critic client', async () => {
    const h = await machine()

    const proofs = (await list(h)).filter((entry) => entry.kind === 'read-only')

    expect(proofs.map((entry) => [entry.status, entry.detail])).toEqual(
      Array.from({ length: 5 }, () => [
        'not-available',
        expect.stringContaining('not available until agent-kit#178 lands') as unknown,
      ]),
    )
  })

  it('runs and records nothing when asked to check', async () => {
    const h = await machine()

    const result = await check(h, 'read-only:vibe-glm', { confirmed: true })

    expect([
      result.status,
      h.process.commands,
      h.agy.launches,
      await h.readText(result.results_path).catch(() => 'no file'),
    ]).toEqual(['not-available', [], [], 'no file'])
  })
})

describe('a sign-in check', () => {
  it('passes on a signed-in machine and records its time, result and CLI version beside the ledger', async () => {
    const h = await machine()

    const result = await check(h, 'vibe-signed-in')

    expect(result.status).toBe('checked')
    expect(JSON.parse(await h.readText(join(dataDir(h), 'machine-setup.json')))).toEqual({
      checks: {
        'vibe-signed-in': {
          checked_at: '2026-10-30T12:00:00.000Z',
          detail: 'vibe is installed and signed in',
          inputs: { cli_version: 'vibe 3.0.4' },
          result: 'pass',
        },
      },
      version: 1,
    })
  })

  it("fails with setup's own sign-in wording when the personal Vibe home is signed out", async () => {
    const h = await machine({ vibeSignIn: 'none' })

    const result = await check(h, 'vibe-signed-in')

    expect([result.item.last?.result, result.item.last?.detail]).toEqual([
      'fail',
      'vibe: not signed in; run vibe and sign in with your Mistral account, then re-run setup',
    ])
  })

  it('fails when vibe is not on PATH', async () => {
    const h = await machine({ vibeOnPath: false })

    expect((await check(h, 'vibe-signed-in')).item.last?.detail).toBe('vibe: vibe is not on PATH')
  })

  it("fails with setup's own sign-in wording when Antigravity is signed out", async () => {
    const h = await machine({ agyModels: 'signed-out' })

    expect((await check(h, 'agy-signed-in')).item.last?.detail).toBe(
      'antigravity: needs sign-in: run agy and sign in with your Google account',
    )
  })

  it('passes Antigravity on a signed-in Mac', async () => {
    const h = await machine()

    expect((await check(h, 'agy-signed-in')).item.last?.result).toBe('pass')
  })

  it('is current on the next list once it passed', async () => {
    const h = await machine()
    await check(h, 'vibe-signed-in')

    expect(item(await list(h), 'vibe-signed-in').status).toBe('current')
  })

  it('does not run again while it is current', async () => {
    const h = await machine()
    await check(h, 'agy-signed-in')
    h.agy.commands.splice(0)

    const result = await check(h, 'agy-signed-in')

    expect([result.status, h.agy.commands.map((command) => command.command)]).toEqual([
      'current',
      ['agy'],
    ])
  })

  it('runs again while current when forced', async () => {
    const h = await machine()
    await check(h, 'vibe-signed-in')

    expect((await check(h, 'vibe-signed-in', { force: true })).status).toBe('checked')
  })

  it('is due again after the CLI version changed', async () => {
    const h = await machine()
    await check(h, 'vibe-signed-in')
    h.process.version = 'vibe 3.1.0'

    const entry = item(await list(h), 'vibe-signed-in')

    expect([entry.status, entry.due_because, entry.changed]).toEqual([
      'due',
      'inputs-changed',
      ['cli_version'],
    ])
  })

  it('is due again after it failed, though nothing changed', async () => {
    const h = await machine({ vibeSignIn: 'none' })
    await check(h, 'vibe-signed-in')

    const entry = item(await list(h), 'vibe-signed-in')

    expect([entry.status, entry.due_because]).toEqual(['due', 'failed'])
  })
})

describe('a critic check', () => {
  it('asks for confirmation before it spends Vibe money, and launches nothing', async () => {
    const h = await machine()

    const result = await check(h, 'critic:vibe-glm')

    expect([result.status, h.process.commands]).toEqual(['needs-confirmation', []])
  })

  it('asks for confirmation before it spends Antigravity quota, and launches nothing', async () => {
    const h = await machine()

    const result = await check(h, 'critic:agy-gemini-flash-high')

    expect([result.status, h.agy.launches]).toEqual(['needs-confirmation', []])
  })

  it('records nothing while unconfirmed', async () => {
    const h = await machine()

    await check(h, 'critic:vibe-glm')

    expect(item(await list(h), 'critic:vibe-glm').due_because).toBe('never-checked')
  })

  it('once confirmed, puts the trivial prompt to the critic agent and passes when it answers', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })

    const result = await check(h, 'critic:vibe-glm', { confirmed: true })

    expect([result.status, result.item.last?.result, result.item.last?.detail]).toEqual([
      'checked',
      'pass',
      'vibe-glm answered the trivial prompt (charged $0.0042)',
    ])
  })

  it('launches the critic read-only in a one-note packet under the data directory, never in the repository', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })

    await check(h, 'critic:vibe-glm', { confirmed: true })

    const [launch] = h.process.commands
    const args = launch?.args ?? []
    expect({
      agent: args[args.indexOf('--agent') + 1],
      cwd: launch?.cwd,
      note: await h.readText(join(dataDir(h), 'setup-checks', 'vibe-glm', 'packet', 'note.md')),
      tools: args[args.indexOf('--enabled-tools') + 1],
    }).toEqual({
      agent: 'critic-glm',
      cwd: join(dataDir(h), 'setup-checks', 'vibe-glm', 'packet'),
      note: "This note is agent-kit's setup check. It has nothing in it to review.\n",
      tools: 'read_file',
    })
  })

  it('caps the trivial prompt at five cents', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })

    await check(h, 'critic:vibe-glm', { confirmed: true })

    const args = h.process.commands[0]?.args ?? []
    expect(args[args.indexOf('--max-price') + 1]).toBe('0.05')
  })

  it('fails with the failure and the charge when the critic does not answer', async () => {
    const h = await machine({
      scripts: [{ agent: 'critic-glm', exitCode: 2, stderr: 'agent not found', stdout: '' }],
    })

    const result = await check(h, 'critic:vibe-glm', { confirmed: true })

    expect([result.item.last?.result, result.item.last?.detail]).toEqual([
      'fail',
      'infrastructure-failure: vibe exited 2: agent not found (charged $0.0500)',
    ])
  })

  it('fails without asking or launching when the critic agent is not installed', async () => {
    const h = await machine({ vibeAgents: ['critic-mistral-large'] })

    const result = await check(h, 'critic:vibe-glm')

    expect([result.status, result.item.last?.detail, h.process.commands]).toEqual([
      'checked',
      'vibe: the critic agent critic-glm is not installed in ~/.vibe/agents; re-run setup',
      [],
    ])
  })

  it('records the CLI version, the agent file hash and the routing hash it depended on', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })

    const result = await check(h, 'critic:vibe-glm', { confirmed: true })

    // The harness installs every agent file empty; this is the sha256 of no bytes.
    expect(result.item.last?.inputs).toEqual({
      agent_hash: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
      cli_version: 'vibe 3.0.4',
      routing_hash: expect.stringMatching(/^[0-9a-f]{64}$/) as unknown,
    })
  })

  it('is due again after its agent file changed', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })
    await check(h, 'critic:vibe-glm', { confirmed: true })
    await h.writeText(
      join(h.home, '.vibe', 'agents', 'critic-glm.toml'),
      'active_model = "critic-glm"\n',
    )

    expect(item(await list(h), 'critic:vibe-glm').changed).toEqual(['agent_hash'])
  })

  it('is due again after the routing override changed its client', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })
    await check(h, 'critic:vibe-glm', { confirmed: true })
    await h.writeText(
      overridePath(h),
      JSON.stringify({ clients: { 'vibe-glm': { model: 'zai-glm-5-3-flash' } } }),
    )

    expect(item(await list(h), 'critic:vibe-glm').changed).toEqual(['routing_hash'])
  })

  it('stays current when routing changes another client', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })
    await check(h, 'critic:vibe-glm', { confirmed: true })
    await h.writeText(
      overridePath(h),
      JSON.stringify({ clients: { 'vibe-mistral-small': { model: 'mistral-small-4' } } }),
    )

    expect(item(await list(h), 'critic:vibe-glm').status).toBe('current')
  })

  it("never reads an earlier check's export as this one's", async () => {
    const h = await machine({
      scripts: [
        vibeAnswers('critic-glm'),
        { agent: 'critic-glm', exitCode: 0, stdout: vibeStdout('pong') },
      ],
    })
    await check(h, 'critic:vibe-glm', { confirmed: true })

    const result = await check(h, 'critic:vibe-glm', { confirmed: true, force: true })

    expect(result.item.last?.detail).toBe(
      'missing-export: vibe wrote no readable export.json (charged $0.0500)',
    )
  })

  it('logs the launch to the ledger outside any repository or flow, so its Vibe spend stays on record', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })

    await check(h, 'critic:vibe-glm', { confirmed: true })

    expect(await ledgerLines(h)).toEqual([
      {
        charged_usd: 0.0042,
        client: 'vibe-glm',
        critic_id: 'setup-vibe-glm',
        family: 'zhipu',
        flow: 'machine-setup',
        kind: 'vibe',
        model: 'zai-glm-5-3',
        recorded_at: '2026-10-30T12:00:00.000Z',
        repository: 'machine-setup',
        status: 'ok',
        type: 'invocation',
      },
    ])
  })

  it('passes the Gemini critic when agy returns the empty findings list as structured output', async () => {
    const h = await machine({ agyScripts: [AGY_ANSWERS] })

    const result = await check(h, 'critic:agy-gemini-flash-high', { confirmed: true })

    expect([result.item.last?.result, result.item.last?.detail]).toEqual([
      'pass',
      'agy-gemini-flash-high answered the trivial prompt',
    ])
  })

  it('fails the Gemini critic when agy returns prose instead of structured output', async () => {
    const h = await machine({ agyScripts: [{ stdout: agyStream(agyResult(undefined)) }] })

    const result = await check(h, 'critic:agy-gemini-flash-high', { confirmed: true })

    expect(result.item.last?.detail).toBe(
      'missing-structured-output: the reply did not match the finding schema, so agy returned no structured_output',
    )
  })

  it("counts the Gemini check against today's Antigravity quota", async () => {
    const h = await machine({ agyScripts: [AGY_ANSWERS] })
    await check(h, 'critic:agy-gemini-flash-high', { confirmed: true })

    expect((await check(h, 'agy-quota')).item.last?.detail).toContain('1 used today')
  })
})

describe('the Antigravity quota', () => {
  it('fails while no plan limit is stored, because Antigravity cannot report its quota', async () => {
    const h = await machine()

    const result = await check(h, 'agy-quota')

    expect([result.item.last?.result, result.item.last?.detail]).toEqual([
      'fail',
      "Antigravity cannot report its quota, so routing's default applies (daily cap 20, reserve threshold 10, 0 used today); give your plan's daily limit",
    ])
  })

  it("stores the plan's limit and the reserve in the per-machine routing override", async () => {
    const h = await machine()

    await setQuota(h, { daily_cap: 50, reserve_threshold: 25 })

    expect(JSON.parse(await h.readText(overridePath(h)))).toEqual({
      antigravity_quota: { daily_cap: 50, reserve_threshold: 25 },
    })
  })

  it("keeps the override's other keys", async () => {
    const h = await machine({
      routingOverride: {
        antigravity_quota: { reserved_stages: ['spec'] },
        clients: { 'vibe-glm': { model: 'zai-glm-5-3' } },
      },
    })

    await setQuota(h, { daily_cap: 50, reserve_threshold: 25 })

    expect(JSON.parse(await h.readText(overridePath(h)))).toEqual({
      antigravity_quota: { daily_cap: 50, reserve_threshold: 25, reserved_stages: ['spec'] },
      clients: { 'vibe-glm': { model: 'zai-glm-5-3' } },
    })
  })

  it('passes the quota check on the stored values', async () => {
    const h = await machine()

    const result = await setQuota(h, { daily_cap: 50, reserve_threshold: 25 })

    expect([result.item.status, result.item.last?.detail]).toEqual([
      'current',
      `daily cap 50, reserve threshold 25, 0 used today; set in ${overridePath(h)}`,
    ])
  })

  it('rejects a reserve above the daily cap', async () => {
    const h = await machine()

    await expect(
      h.commands.setup.run({ action: 'quota', daily_cap: 10, reserve_threshold: 11 }),
    ).rejects.toMatchObject({
      code: 'invalid-input',
      message: 'setup input: the reserve of 11 is above the daily cap of 10',
    })
  })

  it('writes nothing over a routing override that does not parse', async () => {
    const h = await machine({ routingOverride: '{ not json' })

    await expect(
      h.commands.setup.run({ action: 'quota', daily_cap: 50, reserve_threshold: 25 }),
    ).rejects.toMatchObject({
      code: 'invalid-routing',
    })
    expect(await h.readText(overridePath(h))).toBe('{ not json')
  })

  it('is due again after the stored limit changed', async () => {
    const h = await machine()
    await setQuota(h, { daily_cap: 50, reserve_threshold: 25 })
    await h.writeText(
      overridePath(h),
      JSON.stringify({ antigravity_quota: { daily_cap: 60, reserve_threshold: 25 } }),
    )

    expect(item(await list(h), 'agy-quota').changed).toEqual(['routing_hash'])
  })
})

describe("the repository's opt-in", () => {
  it('reports a repository with no allowlist entry as not opted in, with the families routing uses', async () => {
    const h = await machine()

    expect((await listed(h, { repo: h.repo })).repository).toEqual({
      code: false,
      families: [],
      key: 'github.com/jorisjonkers-dev/agent-kit',
      routed_families: ['mistral', 'google', 'zhipu', 'deepseek'],
      status: 'not-opted-in',
    })
  })

  it('reports the families and, separately, the code permission of an opted-in repository', async () => {
    const h = await machine()
    await h.commands.allow.run({ families: ['mistral', 'google'], repo: h.repo })

    expect((await listed(h, { repo: h.repo })).repository).toMatchObject({
      code: false,
      families: ['mistral', 'google'],
      status: 'opted-in',
    })
  })

  it('reports the refusal for a repository with no remote', async () => {
    const h = await machine({ remotes: [] })

    expect((await listed(h, { repo: h.repo })).repository).toMatchObject({
      reason: 'no-remote',
      status: 'refused',
    })
  })

  it('says nothing about a repository when none is named', async () => {
    const h = await machine()

    expect((await listed(h)).repository).toBeUndefined()
  })
})

describe("setup's input and results file", () => {
  it('lists when no action is named', async () => {
    const h = await machine()

    expect((await h.commands.setup.run({})).status).toBe('listed')
  })

  it('rejects a check that is not on the checklist', async () => {
    const h = await machine()

    await expect(
      h.commands.setup.run({ action: 'check', check: 'after-skill-hook' }),
    ).rejects.toMatchObject({
      code: 'invalid-input',
      message: "setup input: check after-skill-hook is not on this machine's checklist",
    })
  })

  it('rejects a results file that does not parse', async () => {
    const h = await machine()
    await h.writeText(join(dataDir(h), 'machine-setup.json'), '{"version": 1, "checks": {"x": {}}}')

    await expect(list(h)).rejects.toMatchObject({ code: 'invalid-setup' })
  })
})
