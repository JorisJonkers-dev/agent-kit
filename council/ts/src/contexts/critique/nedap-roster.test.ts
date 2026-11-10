import { afterEach, describe, expect, it } from 'vitest'

import { AGENT_KIT_REMOTE, harness, type Harness } from '../../../test/critique/harness.js'
import { parseRouting } from './domain/routing.js'
import { clientReadiness, clientSignIn, clientVersion, reviewWith, type CriticClients, type CriticLaunch } from './application/ports.js'
import { CriticTools } from './application/critic-tools.js'
import type { PrepareResult } from './index.js'

const NEDAP_REMOTE = { name: 'origin', url: 'git@github.com:nedap/widgets.git' }
const READY = { ready: true } as const

const GRANT = {
  answer: 'yes',
  cap_usd: 5,
  client: 'codex',
  expires_at: '2026-10-31T12:00:00Z',
  id: 'g1',
  question: 'may I spend up to $5 of nedap money on the widgets spec?',
  workflow: 'widgets-spec',
  written_at: '2026-10-30T11:00:00Z',
}

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

async function nedap(options: Parameters<typeof harness>[0] = {}): Promise<Harness> {
  const h = await harness({ remotes: [NEDAP_REMOTE], workClients: { claude: READY, codex: READY }, ...options })
  open.push(h)
  return h
}

async function prepare(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const result = await h.commands.prepare.run({
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: '.scratch/widgets/spec.md',
    stage: 'spec',
    workflow: 'widgets-spec',
    ...extra,
  })
  if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
  return result
}

describe('the work-only critic roster of a nedap repository', () => {
  it('runs only the Codex critic for a Claude orchestrator, under a live grant', async () => {
    const h = await nedap({ grants: [GRANT] })

    const result = await prepare(h)

    expect(result.critics.map((critic) => [critic.client, critic.family])).toEqual([['codex-work', 'openai']])
    expect(result.dropped).toEqual([{ client: 'claude-work', family: 'anthropic', reason: 'orchestrator-family' }])
    expect(result.history).toBe(true)
    const manifest = JSON.parse(await h.readText(result.manifest_path)) as Record<string, unknown>
    expect(manifest).toMatchObject({ allowed_families: ['anthropic', 'openai'], repository: 'github.com/nedap/widgets' })
  })

  it('runs only the work Claude critic for a Codex orchestrator, needing no grant', async () => {
    const h = await nedap()

    const result = await prepare(h, { orchestrator_family: 'openai', workflow: undefined })

    expect(result.critics.map((critic) => [critic.client, critic.family])).toEqual([['claude-work', 'anthropic']])
    expect(result.dropped).toEqual([{ client: 'codex-work', family: 'openai', reason: 'orchestrator-family' }])
  })

  it('drops Codex with a reason that names the missing grant when none covers the workflow', async () => {
    const h = await nedap({ grants: [{ ...GRANT, workflow: 'another-workflow' }] })

    const result = await prepare(h)

    expect(result.critics).toEqual([])
    expect(result.dropped).toEqual([
      {
        client: 'codex-work',
        detail: 'no live grant covers workflow widgets-spec for codex: ask the owner, then write one with model-gateway grant write',
        family: 'openai',
        reason: 'no-grant',
      },
      { client: 'claude-work', family: 'anthropic', reason: 'orchestrator-family' },
    ])
  })

  it('drops Codex when a grant exists but has lapsed', async () => {
    const h = await nedap({ grants: [{ ...GRANT, expires_at: '2026-10-30T11:30:00Z' }] })

    expect((await prepare(h)).dropped).toContainEqual(expect.objectContaining({ client: 'codex-work', reason: 'no-grant' }))
  })

  it('drops Codex and says how to name the workflow when the run names none', async () => {
    const h = await nedap({ grants: [GRANT] })

    const result = await prepare(h, { workflow: undefined })

    expect(result.dropped).toContainEqual({
      client: 'codex-work',
      detail: 'name the model-gateway workflow with workflow so a grant for it can be found',
      family: 'openai',
      reason: 'no-grant',
    })
  })

  it('looks for the grant under the run flow when no workflow is named', async () => {
    const h = await nedap({ grants: [GRANT] })

    const result = await prepare(h, { flow: 'widgets-spec', workflow: undefined })

    expect(result.critics.map((critic) => critic.client)).toEqual(['codex-work'])
  })

  it('runs both work critics for an orchestrator from neither of their families', async () => {
    const h = await nedap({ grants: [GRANT] })

    const result = await prepare(h, { orchestrator_family: 'mistral' })

    expect(result.critics.map((critic) => critic.client)).toEqual(['codex-work', 'claude-work'])
    expect(result.dropped).toEqual([])
  })

  it('drops a work critic whose client cannot run here, with the reason', async () => {
    const h = await nedap({ grants: [GRANT], workClients: { claude: READY, codex: { detail: 'codex is not signed in', ready: false } } })

    const result = await prepare(h)

    expect(result.critics).toEqual([])
    expect(result.dropped).toContainEqual({ client: 'codex-work', detail: 'codex is not signed in', family: 'openai', reason: 'unavailable' })
  })

  it('drops a work critic on a machine that wires no client for it', async () => {
    const h = await nedap({ grants: [GRANT], workClients: {} })

    const result = await prepare(h)

    expect(result.dropped).toContainEqual({
      client: 'codex-work',
      detail: 'no codex critic client is wired on this machine',
      family: 'openai',
      reason: 'unavailable',
    })
  })

  it('drops a work critic that is past a quota the client reports', async () => {
    const h = await nedap({
      grants: [GRANT],
      workClients: { claude: READY, codex: { detail: 'out of requests', ready: false, reason: 'quota' } as never },
    })

    expect((await prepare(h)).dropped).toContainEqual(expect.objectContaining({ client: 'codex-work', reason: 'quota' }))
  })

  it('never picks another family, whatever a machine allowlist says', async () => {
    const h = await nedap({ grants: [GRANT] })
    h.git.remoteList = [AGENT_KIT_REMOTE]
    await h.commands.allow.run({ repo: h.repo, families: ['mistral', 'google', 'zhipu', 'deepseek'] })
    h.git.remoteList = [AGENT_KIT_REMOTE, NEDAP_REMOTE]

    const result = await prepare(h)

    expect(result.critics.map((critic) => critic.family)).toEqual(['openai'])
    expect([...result.critics, ...result.dropped].map((critic) => critic.client).sort()).toEqual(['claude-work', 'codex-work'])
  })

  it('still refuses to record an allowlist entry for a nedap repository', async () => {
    const h = await nedap({ grants: [GRANT] })

    expect(await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })).toMatchObject({ reason: 'nedap', status: 'refused' })
  })
})

describe('the work clients outside a nedap repository', () => {
  it('are never picked for a repository the allowlist opts in for their families', async () => {
    const h = await harness({ grants: [GRANT], workClients: { claude: READY, codex: READY } })
    open.push(h)
    await h.commands.allow.run({ repo: h.repo, families: ['mistral', 'anthropic', 'openai'] })

    const result = await prepare(h, { orchestrator_family: 'google' })

    expect(result.critics.map((critic) => critic.client)).toEqual(['vibe-mistral-large'])
    expect(result.dropped.map((critic) => critic.client)).not.toContain('codex-work')
    expect(result.dropped.map((critic) => critic.client)).not.toContain('claude-work')
  })
})

describe('routing with work clients', () => {
  const client = { agent: 'critic-x', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'm' }
  const codex = { binary: 'codex', family: 'openai', kind: 'codex', model: 'gpt-5.5' }
  const slot = (name: string) => ({ client: name, lenses: [], max_price_usd: 1, role: 'r', time_limit_s: 1 })
  const routing = (stage: Record<string, unknown>, fallbacks: Record<string, string[]> = {}) =>
    JSON.stringify({ clients: { codex, vibe: client }, fallbacks, stages: { spec: { base: [], distinct_families: true, ...stage } }, version: 1 })

  it('reads a stage whose work roster names a work client', () => {
    expect(parseRouting(routing({ work: [slot('codex')] })).stages.spec?.work).toEqual([slot('codex')])
  })

  it('rejects a base roster that names a work client', () => {
    expect(() => parseRouting(routing({ base: [slot('codex')] }))).toThrow(/stages\.spec\.base names codex, a work client/)
  })

  it('rejects a deep roster that names a work client', () => {
    expect(() => parseRouting(routing({ deep: [slot('codex')] }))).toThrow(/stages\.spec\.deep names codex, a work client/)
  })

  it('rejects a work roster that names another client', () => {
    expect(() => parseRouting(routing({ work: [slot('vibe')] }))).toThrow(/stages\.spec\.work names vibe, which is not a codex or claude client/)
  })

  it('rejects a fallback to a work client, and from one', () => {
    expect(() => parseRouting(routing({}, { vibe: ['codex'] }))).toThrow(/fallbacks\.vibe names codex, a work client/)
    expect(() => parseRouting(routing({}, { codex: ['vibe'] }))).toThrow(/fallbacks\.codex names codex, a work client/)
  })

  it('rejects a codex client of another family', () => {
    expect(() => parseRouting(JSON.stringify({ clients: { codex: { ...codex, family: 'mistral' } }, stages: {}, version: 1 }))).toThrow(/family/)
  })
})

describe('critic clients by kind', () => {
  const config = { binary: 'claude', family: 'anthropic', kind: 'claude', model: 'm' } as const
  const codexConfig = { binary: 'codex', family: 'openai', kind: 'codex', model: 'm' } as const
  const launch = { critic: { client_config: codexConfig } } as unknown as CriticLaunch
  const claudeLaunch = { critic: { client_config: config } } as unknown as CriticLaunch
  const bare = {} as unknown as CriticClients

  it('says a kind nothing is wired for cannot run, has no version and cannot review', async () => {
    for (const each of [config, codexConfig]) {
      expect(await clientReadiness(bare, each)).toEqual({ detail: `no ${each.kind} critic client is wired on this machine`, ready: false })
      expect(await clientSignIn(bare, each)).toMatchObject({ ready: false })
      expect(await clientVersion(bare, each)).toBeNull()
    }
    await expect(reviewWith(bare, launch)).rejects.toThrow(/no codex critic client is wired/)
    await expect(reviewWith(bare, claudeLaunch)).rejects.toThrow(/no claude critic client is wired/)
  })

  it('asks the client wired for the kind', async () => {
    const calls: string[] = []
    const port = (name: string) => ({
      readiness: () => Promise.resolve({ ready: true as const }),
      review: () => {
        calls.push(name)
        return Promise.resolve({} as never)
      },
      signIn: () => Promise.resolve({ ready: true as const }),
      version: () => Promise.resolve(`${name} 1`),
    })
    const wired = { claude: port('claude'), codex: port('codex') } as unknown as CriticClients

    expect(await clientReadiness(wired, config)).toEqual({ ready: true })
    expect(await clientSignIn(wired, codexConfig)).toEqual({ ready: true })
    expect(await clientVersion(wired, codexConfig)).toBe('codex 1')
    expect(await clientVersion(wired, config)).toBe('claude 1')
    await reviewWith(wired, launch)
    await reviewWith(wired, claudeLaunch)
    expect(calls).toEqual(['codex', 'claude'])
  })
})

describe('work critics and search', () => {
  it('read the packet alone, since the runner searches only for Vibe', async () => {
    const tools = new CriticTools({ clients: {} as CriticClients, env: {} as never, files: {} as never }, { max_bytes: 1, max_matches: 1 }, '/nowhere')
    const critic = { client: 'codex-work', client_config: { binary: 'codex', family: 'openai', kind: 'codex', model: 'm' } }

    expect(await tools.choose(critic as never, '/packet')).toEqual({ toolset: { kind: 'vibe-read' } })
  })
})
