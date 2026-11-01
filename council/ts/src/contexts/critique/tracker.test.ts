import { access, readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { harness, SPEC_CANARY, specIssue, type Harness } from '../../../test/critique/harness.js'
import type { PrepareResult, Refusal } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const AGENT_KIT = 'github.com/jorisjonkers-dev/agent-kit'

async function optedIn(...args: Parameters<typeof harness>): Promise<Harness> {
  const h = await harness(...args)
  open.push(h)
  await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })
  return h
}

async function prepare(h: Harness, spec: string, extra: Record<string, unknown> = {}): Promise<PrepareResult | Refusal> {
  return h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec, stage: 'spec', ...extra })
}

async function ready(h: Harness, spec: string, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const result = await prepare(h, spec, extra)
  if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
  return result
}

describe('prepare with a GitHub tracker', () => {
  it('freezes the spec issue body into the packet through gh', async () => {
    const h = await optedIn({ issues: { [`${AGENT_KIT}#167`]: specIssue() }, tracker: 'github' })

    const result = await ready(h, '#167')

    expect(result.packet_files[0]).toBe('artifact/issue-167.md')
    expect(await readFile(join(result.packet_dir, 'artifact', 'issue-167.md'), 'utf8')).toBe(
      `# Widgets\n\n## User Stories\n\n1. As the owner, I want widgets.\n\n3. ${SPEC_CANARY}\n`,
    )
  })

  it('freezes the issue comments under the body when the caller asks for them', async () => {
    const h = await optedIn({ issues: { [`${AGENT_KIT}#167`]: specIssue() }, tracker: 'github' })

    const result = await ready(h, '#167', { comments: true })

    expect(await readFile(join(result.packet_dir, 'artifact', 'issue-167.md'), 'utf8')).toMatch(
      /\n## Comments\n\n### joris, 2026-10-30T09:00:00Z\n\nAmended: the window is seven seconds\.\n$/,
    )
  })

  it('freezes a title alone when the issue has no body', async () => {
    const h = await optedIn({ issues: { [`${AGENT_KIT}#167`]: specIssue({ body: '', comments: [] }) }, tracker: 'github' })

    const result = await ready(h, '#167', { comments: true })

    expect(await readFile(join(result.packet_dir, 'artifact', 'issue-167.md'), 'utf8')).toBe('# Widgets\n')
  })

  it('records the issue and its labels as the artifact of the run', async () => {
    const h = await optedIn({ issues: { [`${AGENT_KIT}#167`]: specIssue() }, tracker: 'github' })

    const result = await ready(h, '#167')

    const expected = { labels: ['type:feature'], reference: `${AGENT_KIT}#167`, tracker: 'github' }
    expect(result.artifact).toEqual(expected)
    const manifest = JSON.parse(await readFile(result.manifest_path, 'utf8')) as { artifact: unknown; packet_files: unknown[] }
    expect(manifest.artifact).toEqual(expected)
    expect(manifest.packet_files[0]).toMatchObject({ kind: 'artifact', path: 'artifact/issue-167.md', source: `${AGENT_KIT}#167` })
  })

  it.each(['167', 'JorisJonkers-dev/agent-kit#167', 'https://github.com/JorisJonkers-dev/agent-kit/issues/167#issuecomment-1'])(
    'reads the issue named as %s',
    async (reference) => {
      const h = await optedIn({ issues: { [`${AGENT_KIT}#167`]: specIssue() }, tracker: 'github' })

      expect((await ready(h, reference)).artifact.reference).toBe(`${AGENT_KIT}#167`)
    },
  )

  it('asks gh for the issue in the repository the allowlist is keyed by', async () => {
    const h = await optedIn({ issues: { [`${AGENT_KIT}#167`]: specIssue() }, tracker: 'github' })

    await ready(h, '#167')

    expect(h.gh.requests).toEqual([{ comments: false, cwd: h.repo, number: 167, repo: AGENT_KIT }])
  })

  it('refuses in one line, and writes nothing, when gh fails', async () => {
    const h = await optedIn({
      issues: { [`${AGENT_KIT}#167`]: 'gh: To get started with GitHub CLI, please run:  gh auth login\nAlternatively, set GH_TOKEN.' },
      tracker: 'github',
    })

    expect(await prepare(h, '#167')).toEqual({
      message: `cannot read issue ${AGENT_KIT}#167 through gh: gh: To get started with GitHub CLI, please run:  gh auth login`,
      reason: 'tracker-unreachable',
      status: 'refused',
    })
    await expect(access(join(h.repo, '.git', 'critique'))).rejects.toThrow()
  })

  it('refuses an issue gh cannot find', async () => {
    const h = await optedIn({ tracker: 'github' })

    expect(await prepare(h, '#9999')).toMatchObject({
      message: expect.stringMatching(/^cannot read issue .*#9999 through gh: .*Could not resolve to an issue/) as unknown,
      reason: 'tracker-unreachable',
    })
  })

  it('rejects an issue of another repository, whose content the allowlist does not cover', async () => {
    const h = await optedIn({ issues: { 'github.com/nedap/secret#1': specIssue() }, tracker: 'github' })

    await expect(prepare(h, 'https://github.com/nedap/secret/issues/1')).rejects.toThrow(
      /is an issue of github\.com\/nedap\/secret, not of github\.com\/jorisjonkers-dev\/agent-kit/,
    )
    expect(h.gh.requests).toEqual([])
  })

  it('rejects a spec named by a path', async () => {
    const h = await optedIn({ tracker: 'github' })

    await expect(prepare(h, '.scratch/widgets/spec.md')).rejects.toThrow(/is not an issue reference/)
  })
})

describe('prepare finds the tracker', () => {
  it('reads GitHub issues when there is no tracker doc and the remote is on GitHub', async () => {
    const h = await optedIn({ issues: { [`${AGENT_KIT}#167`]: specIssue() }, tracker: 'none' })

    expect((await ready(h, '#167')).artifact.tracker).toBe('github')
  })

  it('reads local markdown when there is no tracker doc and the remote is not on GitHub', async () => {
    const h = await optedIn({ remotes: [{ name: 'origin', url: 'git@gitlab.example.com:joris/widgets.git' }], tracker: 'none' })

    expect((await ready(h, '.scratch/widgets/spec.md')).artifact).toEqual({
      labels: [],
      reference: '.scratch/widgets/spec.md',
      tracker: 'local-markdown',
    })
  })

  it('reads local markdown when the tracker doc says so, though the remote is on GitHub', async () => {
    const h = await optedIn({ tracker: 'local-markdown' })

    expect((await ready(h, '.scratch/widgets/spec.md')).artifact.tracker).toBe('local-markdown')
    expect(h.gh.requests).toEqual([])
  })

  it('refuses a tracker the critique cannot read', async () => {
    const h = await optedIn({ tracker: 'gitlab' })

    expect(await prepare(h, '#167')).toEqual({
      message: 'docs/agents/issue-tracker.md names GitLab; the critique reads specs from GitHub issues or local markdown only',
      reason: 'unsupported-tracker',
      status: 'refused',
    })
  })

  it('refuses a tracker doc that names no tracker', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, 'docs', 'agents', 'issue-tracker.md'), 'We track work in Linear.\n')

    expect(await prepare(h, '#167')).toMatchObject({ message: expect.stringContaining('names no tracker') as unknown, reason: 'unsupported-tracker' })
  })

  it('rejects a local-markdown spec that is not a file in the repository', async () => {
    const h = await optedIn()

    await expect(prepare(h, '#167')).rejects.toThrow(/#167 is not a file in the repository/)
  })
})
