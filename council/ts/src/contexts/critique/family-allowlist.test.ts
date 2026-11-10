import { existsSync } from 'node:fs'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { AGENT_KIT_REMOTE, harness, type Harness } from '../../../test/critique/harness.js'

const open: Harness[] = []
const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
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
  ])('gives a nedap remote (%s) the work-only roster whatever its allowlist says', async (url) => {
    const h = await setup({ remotes: [AGENT_KIT_REMOTE] })
    await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })
    h.git.remoteList = [AGENT_KIT_REMOTE, { name: 'upstream', url }]

    const result = await h.commands.prepare.run(prepareInput(h))

    expect(result).toMatchObject({ status: 'ready', critics: [] })
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
      code: false,
      history: false,
      allowlist_path: path,
    })
    expect(JSON.parse(await h.readText(path))).toMatchObject({
      repositories: { 'github.com/jorisjonkers-dev/agent-kit': { families: ['mistral'] } },
    })
  })

  it('keeps the allowlist under XDG_CONFIG_HOME when it is set, as the model gateway reads it', async () => {
    const xdg = await mkdtemp(join(tmpdir(), 'critique-xdg-'))
    cleanups.push(() => rm(xdg, { force: true, recursive: true }))
    const h = await setup({ env: { XDG_CONFIG_HOME: xdg } })

    const result = await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })

    const path = join(xdg, 'agent-kit', 'critique-allowlist.json')
    expect(result).toMatchObject({ allowlist_path: path, status: 'allowed' })
    expect(existsSync(path)).toBe(true)
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

    await expect(h.commands.allow.run({ repo: h.repo, families: ['meta'] })).rejects.toThrow(
      /families\[0\] must be one of mistral, zhipu, deepseek, google, anthropic, openai/,
    )
  })

  it.each(['anthropic', 'openai'])('accepts %s as a family, so a delegation can reach that client', async (family) => {
    const h = await setup()

    const result = await h.commands.allow.run({ repo: h.repo, families: [family] })

    expect(result).toMatchObject({ status: 'allowed', families: [family] })
    expect(JSON.parse(await h.readText(join(h.home, '.config', 'agent-kit', 'critique-allowlist.json')))).toMatchObject({
      repositories: { 'github.com/jorisjonkers-dev/agent-kit': { families: [family] } },
    })
  })

  it('prepares a run with no critics when the allowlist permits none of the routed families', async () => {
    const h = await setup()
    await h.commands.allow.run({ repo: h.repo, families: ['deepseek'] })

    const result = await h.commands.prepare.run(prepareInput(h))

    expect(result).toMatchObject({
      critics: [],
      dropped: [
        { client: 'vibe-mistral-large', family: 'mistral', reason: 'family-not-allowed' },
        { client: 'agy-gemini-flash-high', family: 'google', reason: 'family-not-allowed' },
      ],
      status: 'ready',
    })
  })

  it.each<[string, unknown]>([
    ['families written as one string', 'mistral'],
    ['a family it does not know', ['mistral', 'meta']],
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

  describe('the code permission', () => {
    function allowlistFile(h: Harness): string {
      return join(h.home, '.config', 'agent-kit', 'critique-allowlist.json')
    }

    async function entry(h: Harness): Promise<{ families: string[]; code: boolean }> {
      const allowlist = JSON.parse(await h.readText(allowlistFile(h))) as {
        repositories: Record<string, { families: string[]; code: boolean }>
      }
      const found = allowlist.repositories['github.com/jorisjonkers-dev/agent-kit']
      if (found === undefined) throw new Error('no entry')
      return found
    }

    it('is off for a new entry', async () => {
      const h = await setup()

      await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })

      expect((await entry(h)).code).toBe(false)
    })

    it('is turned on by allow', async () => {
      const h = await setup()

      const result = await h.commands.allow.run({ repo: h.repo, families: ['mistral'], code: true })

      expect(result).toMatchObject({ status: 'allowed', code: true })
      expect((await entry(h)).code).toBe(true)
    })

    it('is turned off again by allow', async () => {
      const h = await setup()
      await h.commands.allow.run({ repo: h.repo, families: ['mistral'], code: true })

      await h.commands.allow.run({ repo: h.repo, families: ['mistral'], code: false })

      expect((await entry(h)).code).toBe(false)
    })

    it('stays as it was when allow changes only the families', async () => {
      const h = await setup()
      await h.commands.allow.run({ repo: h.repo, families: ['mistral'], code: true })

      await h.commands.allow.run({ repo: h.repo, families: ['mistral', 'google'] })

      expect(await entry(h)).toMatchObject({ families: ['mistral', 'google'], code: true })
    })

    it('is off for an entry recorded before the permission existed', async () => {
      const h = await setup()
      await h.writeText(
        allowlistFile(h),
        JSON.stringify({ repositories: { 'github.com/jorisjonkers-dev/agent-kit': { families: ['mistral'], updated_at: 'x' } }, version: 1 }),
      )

      const result = await h.commands.prepare.run({ ...prepareInput(h), stage: 'ticket-review', ticket: '.scratch/widgets/issues/01-widgets.md', base: 'HEAD' })

      expect(result).toMatchObject({ status: 'refused', reason: 'code-not-allowed' })
    })

    it('refuses to trust an entry whose code permission is not true or false', async () => {
      const h = await setup()
      await h.writeText(
        allowlistFile(h),
        JSON.stringify({
          repositories: { 'github.com/jorisjonkers-dev/agent-kit': { families: ['mistral'], code: 'yes', updated_at: 'x' } },
          version: 1,
        }),
      )

      await expect(h.commands.prepare.run(prepareInput(h))).rejects.toThrow(/critique-allowlist\.json: .*code/)
    })

    it.each(['ticket-review', 'feature-review'])(
      'refuses %s with one line saying why before any packet exists while it is off',
      async (stage) => {
        const h = await setup()
        await h.commands.allow.run({ repo: h.repo, families: ['mistral', 'google'] })

        const result = await h.commands.prepare.run({ ...prepareInput(h), stage, ticket: '.scratch/widgets/issues/01-widgets.md', base: 'HEAD' })

        expect(result).toEqual({
          status: 'refused',
          reason: 'code-not-allowed',
          message:
            'github.com/jorisjonkers-dev/agent-kit allows documents but not code; record the code permission with allow and "code": true',
        })
        expect(existsSync(critiqueDir(h))).toBe(false)
      },
    )

    it('lets the no-remote refusal win over a code permission', async () => {
      const h = await setup()
      await h.commands.allow.run({ repo: h.repo, families: ['mistral'], code: true })
      h.git.remoteList = []

      const result = await h.commands.prepare.run({ ...prepareInput(h), stage: 'feature-review', base: 'HEAD' })

      expect(result).toMatchObject({ status: 'refused', reason: 'no-remote' })
      expect(existsSync(critiqueDir(h))).toBe(false)
    })
  })

  describe('the history permission', () => {
    function allowlistFile(h: Harness): string {
      return join(h.home, '.config', 'agent-kit', 'critique-allowlist.json')
    }

    async function entry(h: Harness): Promise<{ families: string[]; code: boolean; history: boolean }> {
      const allowlist = JSON.parse(await h.readText(allowlistFile(h))) as {
        repositories: Record<string, { families: string[]; code: boolean; history: boolean }>
      }
      const found = allowlist.repositories['github.com/jorisjonkers-dev/agent-kit']
      if (found === undefined) throw new Error('no entry')
      return found
    }

    it('is off for a new entry', async () => {
      const h = await setup()

      const result = await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })

      expect(result).toMatchObject({ status: 'allowed', history: false })
      expect((await entry(h)).history).toBe(false)
    })

    it('is turned on by allow, apart from the code permission', async () => {
      const h = await setup()

      const result = await h.commands.allow.run({ repo: h.repo, families: ['mistral'], history: true })

      expect(result).toMatchObject({ status: 'allowed', code: false, history: true })
      expect(await entry(h)).toMatchObject({ code: false, history: true })
    })

    it('is turned off again by allow', async () => {
      const h = await setup()
      await h.commands.allow.run({ repo: h.repo, families: ['mistral'], history: true })

      await h.commands.allow.run({ repo: h.repo, families: ['mistral'], history: false })

      expect((await entry(h)).history).toBe(false)
    })

    it('stays as it was when allow changes only the families or the code permission', async () => {
      const h = await setup()
      await h.commands.allow.run({ repo: h.repo, families: ['mistral'], history: true })

      await h.commands.allow.run({ repo: h.repo, families: ['mistral', 'google'], code: true })

      expect(await entry(h)).toMatchObject({ families: ['mistral', 'google'], code: true, history: true })
    })

    it('is off for an entry recorded before the permission existed, and prepare says so', async () => {
      const h = await setup()
      await h.writeText(
        allowlistFile(h),
        JSON.stringify({
          repositories: { 'github.com/jorisjonkers-dev/agent-kit': { families: ['mistral'], code: true, updated_at: 'x' } },
          version: 1,
        }),
      )

      expect(await h.commands.prepare.run(prepareInput(h))).toMatchObject({ status: 'ready', history: false })
    })

    it('reaches prepare when it is on', async () => {
      const h = await setup()
      await h.commands.allow.run({ repo: h.repo, families: ['mistral'], history: true })

      expect(await h.commands.prepare.run(prepareInput(h))).toMatchObject({ status: 'ready', history: true })
    })

    it('refuses to trust an entry whose history permission is not true or false', async () => {
      const h = await setup()
      await h.writeText(
        allowlistFile(h),
        JSON.stringify({
          repositories: { 'github.com/jorisjonkers-dev/agent-kit': { families: ['mistral'], history: 'yes', updated_at: 'x' } },
          version: 1,
        }),
      )

      await expect(h.commands.prepare.run(prepareInput(h))).rejects.toThrow(/critique-allowlist\.json: .*history/)
    })
  })
})
