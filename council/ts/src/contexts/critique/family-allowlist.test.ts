import { existsSync } from 'node:fs'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { AGENT_KIT_REMOTE, harness, type Harness } from '../../../test/critique/harness.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

async function setup(...args: Parameters<typeof harness>): Promise<Harness> {
  const h = await harness(...args)
  open.push(h)
  return h
}

function prepareInput(h: Harness): Record<string, unknown> {
  return { repo: h.repo, stage: 'spec', spec: '.scratch/widgets/spec.md', orchestrator_family: 'anthropic' }
}

function critiqueDir(h: Harness): string {
  return join(h.repo, '.git', 'critique')
}

describe('family allowlist', () => {
  it('refuses a repository with no allowlist entry before any packet exists', async () => {
    const h = await setup()

    const result = await h.commands.prepare.run(prepareInput(h))

    expect(result).toMatchObject({ status: 'refused', reason: 'not-opted-in' })
    expect(existsSync(critiqueDir(h))).toBe(false)
  })

  it('refuses to record an allowlist entry for a repository with no remote', async () => {
    const h = await setup({ remotes: [] })

    expect(await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })).toMatchObject({
      status: 'refused',
      reason: 'no-remote',
    })
  })

  it('refuses to prepare a repository with no remote before any packet exists', async () => {
    const h = await setup({ remotes: [] })

    expect(await h.commands.prepare.run(prepareInput(h))).toMatchObject({ status: 'refused', reason: 'no-remote' })
    expect(existsSync(critiqueDir(h))).toBe(false)
  })

  it.each(['/srv/git/widgets.git', 'https://github.com/lonely'])(
    'treats a remote with no owner and repository (%s) as no remote',
    async (url) => {
      const h = await setup({ remotes: [{ name: 'origin', url }] })

      expect(await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })).toMatchObject({ reason: 'no-remote' })
    },
  )

  it.each([
    'git@github.com:nedap/widgets.git',
    'https://github.com/nedap/widgets.git',
    'ssh://git@github.com/Nedap/widgets',
    'git@github-work:nedap/widgets.git',
  ])('refuses a nedap remote (%s) whatever its allowlist says', async (url) => {
    const h = await setup({ remotes: [AGENT_KIT_REMOTE] })
    await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })
    h.git.remoteList = [AGENT_KIT_REMOTE, { name: 'upstream', url }]

    const result = await h.commands.prepare.run(prepareInput(h))

    expect(result).toMatchObject({ status: 'refused', reason: 'nedap' })
    expect(existsSync(critiqueDir(h))).toBe(false)
  })

  it('refuses to record an allowlist entry for a nedap repository', async () => {
    const h = await setup({ remotes: [{ name: 'origin', url: 'https://github.com/nedap/widgets' }] })

    expect(await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })).toMatchObject({
      status: 'refused',
      reason: 'nedap',
    })
    expect(existsSync(join(h.home, '.config', 'agent-kit', 'critique-allowlist.json'))).toBe(false)
  })

  it('keeps the allowlist on this machine, keyed by the remote, outside the repository', async () => {
    const h = await setup()

    const result = await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })

    const path = join(h.home, '.config', 'agent-kit', 'critique-allowlist.json')
    expect(result).toEqual({
      status: 'allowed',
      repository: 'github.com/jorisjonkers-dev/agent-kit',
      families: ['mistral'],
      allowlist_path: path,
    })
    expect(JSON.parse(await h.readText(path))).toMatchObject({
      repositories: { 'github.com/jorisjonkers-dev/agent-kit': { families: ['mistral'] } },
    })
  })

  it('replaces a repository entry and leaves other repositories alone', async () => {
    const h = await setup()
    await h.commands.allow.run({ repo: h.repo, families: ['mistral', 'google'] })
    h.git.remoteList = [{ name: 'origin', url: 'https://github.com/someone/else.git' }]
    await h.commands.allow.run({ repo: h.repo, families: ['zhipu'] })
    h.git.remoteList = [AGENT_KIT_REMOTE]

    await h.commands.allow.run({ repo: h.repo, families: ['google'] })

    const allowlist = JSON.parse(
      await h.readText(join(h.home, '.config', 'agent-kit', 'critique-allowlist.json')),
    ) as { repositories: Record<string, { families: string[] }> }
    expect(allowlist.repositories['github.com/jorisjonkers-dev/agent-kit']?.families).toEqual(['google'])
    expect(allowlist.repositories['github.com/someone/else']?.families).toEqual(['zhipu'])
  })

  it('rejects a family it does not know', async () => {
    const h = await setup()

    await expect(h.commands.allow.run({ repo: h.repo, families: ['openai'] })).rejects.toThrow(/families\[0\] must be one of mistral, zhipu, deepseek, google/)
  })

  it('prepares a run with no critics when the allowlist permits none of the routed families', async () => {
    const h = await setup()
    await h.commands.allow.run({ repo: h.repo, families: ['google'] })

    const result = await h.commands.prepare.run(prepareInput(h))

    expect(result).toMatchObject({
      critics: [],
      dropped: [{ client: 'vibe-mistral-large', family: 'mistral', reason: 'family-not-allowed' }],
      status: 'ready',
    })
  })

  it.each<[string, unknown]>([
    ['families written as one string', 'mistral'],
    ['a family it does not know', ['mistral', 'openai']],
  ])('refuses to trust an allowlist entry with %s', async (_name, families) => {
    const h = await setup()
    await h.writeText(
      join(h.home, '.config', 'agent-kit', 'critique-allowlist.json'),
      JSON.stringify({ repositories: { 'github.com/jorisjonkers-dev/agent-kit': { families, updated_at: 'x' } }, version: 1 }),
    )

    await expect(h.commands.prepare.run(prepareInput(h))).rejects.toThrow(/critique-allowlist\.json: .*families/)
    expect(existsSync(critiqueDir(h))).toBe(false)
  })

  it('keys the allowlist by origin when a repository has several remotes', async () => {
    const h = await setup({
      remotes: [
        { name: 'fork', url: 'https://github.com/someone/agent-kit.git' },
        AGENT_KIT_REMOTE,
      ],
    })

    expect(await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })).toMatchObject({
      repository: 'github.com/jorisjonkers-dev/agent-kit',
    })
  })
})
