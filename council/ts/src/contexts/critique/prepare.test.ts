import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { DECISION_CANARY, harness, SPEC_CANARY, type Harness } from '../../../test/critique/harness.js'
import type { PrepareResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

async function optedIn(...args: Parameters<typeof harness>): Promise<Harness> {
  const h = await harness(...args)
  open.push(h)
  await h.commands.allow({ repo: h.repo, families: ['mistral'] })
  return h
}

async function ready(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const result = await h.commands.prepare({
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: '.scratch/widgets/spec.md',
    stage: 'spec',
    ...extra,
  })
  if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
  return result
}

describe('prepare', () => {
  it('freezes the spec and its decision doc into a packet in the git common directory', async () => {
    const h = await optedIn()

    const result = await ready(h)

    expect(result.run_id).toBe('20261030T120000Z-spec')
    expect(result.run_dir).toBe(join(h.repo, '.git', 'critique', '20261030T120000Z-spec'))
    expect(result.packet_dir).toBe(join(result.run_dir, 'packet'))
    expect(result.packet_files).toEqual(['artifact/spec.md', 'references/CONTEXT.md'])
    expect(await readFile(join(result.packet_dir, 'artifact', 'spec.md'), 'utf8')).toContain(SPEC_CANARY)
    expect(await readFile(join(result.packet_dir, 'references', 'CONTEXT.md'), 'utf8')).toContain(DECISION_CANARY)
  })

  it('resolves a one-critic roster from the shipped routing', async () => {
    const h = await optedIn()

    const result = await ready(h)

    expect(result.depth).toBe('base')
    expect(result.critics).toEqual([
      {
        client: 'vibe-mistral-large',
        critic_id: 'c1-vibe-mistral-large',
        family: 'mistral',
        lenses: ['fidelity', 'testability'],
        model: 'mistral-large-4-0-3',
        role: 'spec-redteam',
      },
    ])
    expect(result.dropped).toEqual([])
    const manifest = JSON.parse(await readFile(result.manifest_path, 'utf8')) as Record<string, unknown>
    expect(manifest).toMatchObject({ repository: 'github.com/jorisjonkers-dev/agent-kit', stage: 'spec' })
  })

  it('keeps the packet unchanged when the spec is edited afterwards', async () => {
    const h = await optedIn()
    const result = await ready(h)
    const frozen = await readFile(join(result.packet_dir, 'artifact', 'spec.md'), 'utf8')

    await h.writeText(join(h.repo, '.scratch', 'widgets', 'spec.md'), '# Rewritten\n')

    expect(await readFile(join(result.packet_dir, 'artifact', 'spec.md'), 'utf8')).toBe(frozen)
  })

  it('freezes the references the caller names, under their repository paths', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'GLOSSARY.md'), '# Words\n')

    const result = await ready(h, { references: ['GLOSSARY.md', join(h.repo, 'CONTEXT.md')] })

    expect(result.packet_files).toEqual(['artifact/spec.md', 'references/GLOSSARY.md', 'references/CONTEXT.md'])
  })

  it('prefers GLOSSARY.md over CONTEXT.md when no reference is named', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'GLOSSARY.md'), '# Words\n')

    expect((await ready(h)).packet_files).toEqual(['artifact/spec.md', 'references/GLOSSARY.md'])
  })

  it('freezes the spec alone when the repository has no decision doc', async () => {
    const h = await optedIn()
    await h.removeFile(join(h.repo, 'CONTEXT.md'))

    expect((await ready(h)).packet_files).toEqual(['artifact/spec.md'])
  })

  it('gives a second run in the same second its own directory', async () => {
    const h = await optedIn()

    const first = await ready(h)
    const second = await ready(h)

    expect(second.run_id).toBe('20261030T120000Z-spec-2')
    expect(second.run_dir).not.toBe(first.run_dir)
  })

  it('refuses a spec outside the repository', async () => {
    const h = await optedIn()

    await expect(ready(h, { spec: '../elsewhere/spec.md' })).rejects.toThrow(/outside the repository/)
  })

  it('drops a critic from the orchestrator family and refuses when none is left', async () => {
    const h = await optedIn()

    const result = await h.commands.prepare({
      orchestrator_family: 'mistral',
      repo: h.repo,
      spec: '.scratch/widgets/spec.md',
      stage: 'spec',
    })

    expect(result).toMatchObject({ reason: 'no-critic-allowed', status: 'refused' })
    expect(result.status === 'refused' ? result.message : '').toContain('vibe-mistral-large: orchestrator-family')
  })

  it('reports a dropped critic next to the ones that run', async () => {
    const routing = {
      clients: {
        'vibe-mistral-large': { agent: 'critic-mistral-large', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-large-4-0-3' },
        'vibe-glm': { agent: 'critic-glm', binary: 'vibe', family: 'zhipu', kind: 'vibe', model: 'glm-5-3' },
      },
      stages: {
        spec: {
          base: [
            { client: 'vibe-glm', lenses: ['integration'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 },
            { client: 'vibe-mistral-large', lenses: ['fidelity'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 },
          ],
          distinct_families: true,
        },
      },
      version: 1,
    }
    const h = await optedIn({ routing })

    const result = await ready(h)

    expect(result.critics.map((critic) => critic.client)).toEqual(['vibe-mistral-large'])
    expect(result.dropped).toEqual([{ client: 'vibe-glm', family: 'zhipu', reason: 'family-not-allowed' }])
  })

  it('rejects a stage routing does not know, and a client routing does not define', async () => {
    const h = await optedIn({
      routing: {
        clients: {},
        stages: { spec: { base: [{ client: 'ghost', lenses: [], max_price_usd: 0, role: 'x', time_limit_s: 1 }], distinct_families: true } },
        version: 1,
      },
    })

    await expect(ready(h, { stage: 'feature' })).rejects.toThrow(/routing has no stage feature/)
    await expect(ready(h)).rejects.toThrow(/unknown client ghost/)
  })

  it('rejects input that is not an object or misses a field', async () => {
    const h = await optedIn()

    await expect(h.commands.prepare([])).rejects.toThrow(/prepare input must be a JSON object/)
    await expect(h.commands.prepare({ repo: h.repo, stage: 'spec' })).rejects.toThrow(/orchestrator_family must be a non-empty string/)
    await expect(ready(h, { references: 'CONTEXT.md' })).rejects.toThrow(/references must be a list of strings/)
  })
})
