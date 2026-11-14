import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  fenced,
  finding,
  harness,
  isolatedExport,
  piStream,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { PrepareResult, RefuteResult } from './index.js'
import { JsonFields } from './domain/json.js'
import { dataScopeBar, readCriticClient } from './domain/routing.js'
import { CRITIQUE_STAGES } from './domain/stage.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const slot = (client: string) => ({ client, lenses: ['fidelity'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 })

const PUBLIC_PI = { data_scope: 'public', family: 'zhipu', kind: 'pi', model: 'glm-5-3', provider: 'litellm-vibe-study' }
const VIBE_MISTRAL = { agent: 'critic-mistral-large', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-large-4-0' }
const VIBE_DEEPSEEK = { agent: 'critic-deepseek-flash', binary: 'vibe', family: 'deepseek', kind: 'vibe', model: 'deepseek-v4-1-flash-1017' }

/** A spec stage whose base critic is a public-scope Pi client, with a Vibe client behind it and a second Vibe critic. */
const ROUTING = {
  clients: { 'pi-public-glm': PUBLIC_PI, 'vibe-deepseek-flash': VIBE_DEEPSEEK, 'vibe-mistral-large': VIBE_MISTRAL },
  fallbacks: { 'pi-public-glm': ['vibe-deepseek-flash'] },
  stages: { spec: { base: [slot('pi-public-glm'), slot('vibe-mistral-large')], deep: [], distinct_families: true } },
  version: 1,
}

async function prepared(options: { readonly public?: boolean; readonly routing?: unknown } = {}): Promise<{ h: Harness; run: PrepareResult }> {
  const h = await harness({ piCli: { scripts: [{ stdout: piStream('x') }] }, routing: options.routing ?? ROUTING })
  open.push(h)
  await h.commands.allow.run({
    families: ['mistral', 'zhipu', 'deepseek'],
    repo: h.repo,
    ...(options.public === undefined ? {} : { public: options.public }),
  })
  const run = (await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })) as PrepareResult
  return { h, run }
}

async function manifestOf(run: PrepareResult): Promise<{ critics: { fallbacks: { client: string }[] }[] }> {
  return JSON.parse(await readFile(join(run.run_dir, 'manifest.json'), 'utf8')) as { critics: { fallbacks: { client: string }[] }[] }
}

describe('a Pi client names its route data scope', () => {
  const read = (value: unknown) => readCriticClient(JsonFields.of(value, 'clients.x'))

  it('reads any or public, and refuses a client that names none or another', () => {
    expect(read(PUBLIC_PI)).toMatchObject({ data_scope: 'public' })
    expect(read({ ...PUBLIC_PI, data_scope: 'any' })).toMatchObject({ data_scope: 'any' })
    expect(() => read({ ...PUBLIC_PI, data_scope: 'private' })).toThrow(/data_scope/)
    expect(() => read({ ...PUBLIC_PI, data_scope: undefined })).toThrow(/data_scope/)
  })

  it('bars only a public-scope Pi client, and only from a repository not recorded as public', () => {
    const client = read(PUBLIC_PI)
    expect([dataScopeBar(client, false), dataScopeBar(client, true)]).toEqual([true, false])
    expect(dataScopeBar(read({ ...PUBLIC_PI, data_scope: 'any' }), false)).toBe(false)
    expect(dataScopeBar(read(VIBE_MISTRAL), false)).toBe(false)
  })
})

describe('the roster of a repository that is not recorded as public', () => {
  it('never holds a public-scope client: its slot goes to the fallback, with the reason', async () => {
    const { run } = await prepared()

    expect(run.critics.map((critic) => critic.client)).toEqual(['vibe-deepseek-flash', 'vibe-mistral-large'])
    expect(run.dropped).toEqual([
      expect.objectContaining({ client: 'pi-public-glm', reason: 'data-scope', replaced_by: 'c1-vibe-deepseek-flash' }),
    ])
    expect((await manifestOf(run)).critics.flatMap((critic) => critic.fallbacks.map((fallback) => fallback.client))).not.toContain('pi-public-glm')
  })

  it('is private by default, and a repository stays private when allow says public false', async () => {
    const { run } = await prepared({ public: false })

    expect(run.dropped).toEqual([expect.objectContaining({ reason: 'data-scope' })])
  })

  it('does not run a public-scope client even as a fallback', async () => {
    const routing = {
      ...ROUTING,
      fallbacks: { 'vibe-mistral-large': ['pi-public-glm'] },
      stages: { spec: { base: [slot('vibe-mistral-large')], deep: [], distinct_families: true } },
    }
    const h = await harness({ piCli: {}, routing })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral', 'zhipu'], repo: h.repo })
    const run = (await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })) as PrepareResult

    expect((await manifestOf(run)).critics.flatMap((critic) => critic.fallbacks)).toEqual([])
  })
})

describe('the roster of a repository recorded as public', () => {
  it('runs the public-scope client in its own slot, and the manifest says the repository is public', async () => {
    const { h, run } = await prepared({ public: true })

    expect(run.critics.map((critic) => critic.client)).toEqual(['pi-public-glm', 'vibe-mistral-large'])
    expect(run.dropped).toEqual([])
    expect(JSON.parse(await readFile(join(run.run_dir, 'manifest.json'), 'utf8'))).toMatchObject({ public_repository: true })
    const allowlist = JSON.parse(await readFile(join(h.home, '.config', 'agent-kit', 'critique-allowlist.json'), 'utf8')) as {
      repositories: Record<string, unknown>
    }
    expect(Object.values(allowlist.repositories)).toEqual([expect.objectContaining({ public: true })])
  })

  it('keeps the flag when allow is run again without it, and the manifest of a private one carries none', async () => {
    const { h } = await prepared({ public: true })

    const again = await h.commands.allow.run({ families: ['mistral', 'zhipu'], repo: h.repo })

    expect(again).toMatchObject({ public: true })
    const { run } = await prepared()
    expect(JSON.parse(await readFile(join(run.run_dir, 'manifest.json'), 'utf8'))).not.toHaveProperty('public_repository')
  })

  it('records no flag at all for a repository that is private', async () => {
    const { h } = await prepared()

    const allowlist = await readFile(join(h.home, '.config', 'agent-kit', 'critique-allowlist.json'), 'utf8')

    expect(allowlist).not.toContain('public')
  })
})

describe('the refuters of a repository that is not recorded as public', () => {
  const vibe = (agent: string, value: unknown): VibeScript => ({
    agent,
    exitCode: 0,
    exportJson: { ...isolatedExport(), cost_usd: 0.01 },
    stdout: vibeStdout(fenced(value)),
  })
  const refuteRouting = {
    clients: { 'pi-public-glm': PUBLIC_PI, 'vibe-mistral-large': VIBE_MISTRAL },
    stages: { spec: { base: [slot('vibe-mistral-large')], deep: [slot('pi-public-glm')], distinct_families: true } },
    version: 1,
  }

  async function refute(isPublic: boolean): Promise<RefuteResult> {
    const h = await harness({ piCli: { scripts: [{ stdout: piStream(`Refuted.\n${fenced({ findings: [], no_findings_reason: 'Compared every story.' })}`) }] }, routing: refuteRouting })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral', 'zhipu'], public: isPublic, repo: h.repo })
    const run = (await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })) as PrepareResult
    h.process.queue(vibe('critic-mistral-large', { findings: [finding({ severity: 'critical' })] }))
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    return h.commands.refute.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })
  }

  it('skip the public-scope client, and say why', async () => {
    const result = await refute(false)

    expect(result.results).toEqual([
      expect.objectContaining({ finding_id: 'F1', outcome: 'skipped', reason_code: 'no-eligible-client' }),
    ])
    expect(JSON.stringify(result.results)).toContain('public-only')
  })

  it('use it once the repository is recorded as public', async () => {
    const result = await refute(true)

    expect(result.results[0]).toMatchObject({ finding_id: 'F1', outcome: 'no-objection' })
  })
})

describe('the shipped routing', () => {
  interface Shipped {
    readonly clients: Record<string, { readonly kind: string; readonly provider?: string; readonly family: string; readonly data_scope?: string }>
    readonly fallbacks: Record<string, readonly string[]>
    readonly stages: Record<string, { readonly distinct_families: boolean; readonly base: { client: string }[]; readonly deep?: { client: string }[] }>
  }
  const shipped = async (): Promise<Shipped> => JSON.parse(await readFile(join(ASSETS_ROOT, 'routing.json'), 'utf8')) as Shipped

  it('has Pi critics, or agy, in every base and deep slot of every stage that has them, never a Vibe login', async () => {
    const routing = await shipped()
    const slots = Object.entries(routing.stages).flatMap(([stage, routes]) =>
      [...routes.base, ...(routes.deep ?? [])].map(({ client }) => ({ client, kind: routing.clients[client]?.kind, stage })),
    )

    expect(slots.length).toBeGreaterThan(10)
    expect(slots.filter(({ kind }) => kind !== 'pi' && kind !== 'agy')).toEqual([])
    expect(Object.keys(routing.stages)).toEqual(expect.arrayContaining([...CRITIQUE_STAGES]))
  })

  it('spreads the Pi critics over at least three model families, and every stage keeps its families distinct', async () => {
    const routing = await shipped()
    const families = new Set(Object.values(routing.clients).filter(({ kind }) => kind === 'pi').map(({ family }) => family))
    const perStage = Object.values(routing.stages).map((routes) => [...routes.base, ...(routes.deep ?? [])].map(({ client }) => routing.clients[client]?.family))

    expect(families.size).toBeGreaterThanOrEqual(3)
    expect(Object.values(routing.stages).every(({ distinct_families: distinct }) => distinct)).toBe(true)
    for (const roster of perStage) expect(new Set(roster).size).toBe(roster.length)
  })

  it('keeps the Vibe login behind every Pi critic, and agy', async () => {
    const routing = await shipped()
    const pi = Object.entries(routing.clients).filter(([, client]) => client.kind === 'pi' && client.data_scope === 'any' && client.provider === 'litellm-vibe')

    expect(pi.length).toBeGreaterThanOrEqual(3)
    for (const [name] of pi) {
      expect(routing.fallbacks[name]?.some((fallback) => routing.clients[fallback]?.kind === 'vibe')).toBe(true)
    }
    expect(Object.values(routing.stages).some((routes) => routes.base.some(({ client }) => routing.clients[client]?.kind === 'agy'))).toBe(true)
  })

  it("gives every Pi client its route's data scope from routes.json, and no public-scope client a primary slot", async () => {
    const routing = await shipped()
    const routes = JSON.parse(await readFile(join(ASSETS_ROOT, '..', '..', '..', 'registry', 'generated', 'litellm', 'routes.json'), 'utf8')) as {
      routes: { agent: string; data_scope: string }[]
    }
    const scopeOf = new Map(routes.routes.map((route) => [`litellm-${route.agent}`, route.data_scope === 'public' ? 'public' : 'any']))
    const primaries = new Set(Object.values(routing.stages).flatMap((stage) => [...stage.base, ...(stage.deep ?? [])].map(({ client }) => client)))

    for (const [name, client] of Object.entries(routing.clients)) {
      if (client.kind !== 'pi') continue
      expect([name, client.data_scope]).toEqual([name, scopeOf.get(client.provider ?? '')])
      if (client.data_scope === 'public') expect(primaries.has(name)).toBe(false)
    }
  })
})
