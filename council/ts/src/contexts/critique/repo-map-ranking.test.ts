import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { codeStageRouting, harness, type Harness } from '../../../test/critique/harness.js'
import type { PrepareResult, RepoMapPort } from './index.js'

/**
 * The repository stages order their tree sample by code-nav's repository map
 * (B7-14): the files most referenced come first. The map is faked here; a
 * machine without code-nav keeps the sample's own order and the packet says so.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

/** A repository map that ranks `src/db.ts` above `src/cache.ts` and an unknown path. */
const RANKED: RepoMapPort = {
  rank: () => Promise.resolve({ paths: ['src/db.ts', 'src/nowhere.ts', 'src/cache.ts'], status: 'ranked' }),
}
const MISSING: RepoMapPort = {
  rank: () => Promise.resolve({ detail: 'code-nav did not start: spawn code-nav ENOENT', status: 'unavailable' }),
}

async function recovery(repoMap: RepoMapPort | undefined): Promise<{ h: Harness; run: PrepareResult }> {
  const h = await harness(repoMap === undefined ? {} : { repoMap })
  open.push(h)
  await h.commands.allow.run({ code: true, families: ['mistral', 'deepseek'], repo: h.repo })
  await h.writeText(join(h.repo, 'src', 'a.ts'), 'export const a = 1\n')
  await h.writeText(join(h.repo, 'src', 'cache.ts'), 'export const cache = 1\n')
  await h.writeText(join(h.repo, 'src', 'db.ts'), 'export const db = 1\n')
  await h.git.commit()
  const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, stage: 'decision-recovery' })
  if (run.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(run)}`)
  return { h, run }
}

async function treeOrder(h: Harness, run: PrepareResult): Promise<string[]> {
  const manifest = JSON.parse(await h.readText(run.manifest_path)) as { packet_files: { kind: string; source: string }[] }
  return manifest.packet_files.filter((file) => file.kind === 'tree').map((file) => file.source)
}

describe('a repository stage with code-nav installed', () => {
  it('samples the ranked source files first, then the rest in the stage order, and says so in the facts index', async () => {
    const { h, run } = await recovery(RANKED)

    expect(await treeOrder(h, run)).toEqual(['src/db.ts', 'src/cache.ts', 'src/a.ts'])
    expect(await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')).toContain("code-nav's repository map ranked 3 files")
  })
})

describe('a repository stage without code-nav', () => {
  it('keeps the current order and notes that the map was not used', async () => {
    const { h, run } = await recovery(MISSING)

    expect(await treeOrder(h, run)).toEqual(['src/a.ts', 'src/cache.ts', 'src/db.ts'])
    expect(await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')).toContain("code-nav's repository map was not used (code-nav did not start: spawn code-nav ENOENT)")
  })

  it('has no ranking note when the runner has no map wired', async () => {
    const { run } = await recovery(undefined)

    expect(await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')).not.toContain('## Ranking')
  })
})

describe('a code packet with code-nav installed', () => {
  async function code(repoMap: RepoMapPort): Promise<{ h: Harness; run: PrepareResult }> {
    const h = await harness({ repoMap, routing: codeStageRouting() })
    open.push(h)
    await h.commands.allow.run({ code: true, families: ['mistral'], repo: h.repo })
    await h.writeText(join(h.repo, '.scratch/widgets/issues/01-frobnicate.md'), '# 01: Frobnicate\n\n- [ ] Counts.\n')
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widget = 1\n')
    await h.writeText(join(h.repo, 'src/cache.ts'), 'export const cache = 1\n')
    await h.writeText(join(h.repo, 'src/db.ts'), 'export const db = 1\n')
    await h.git.commit()
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widget = 2\n')
    await h.git.commit()
    const run = await h.commands.prepare.run({
      base: 'HEAD~1',
      orchestrator_family: 'anthropic',
      repo: h.repo,
      spec: '.scratch/widgets/spec.md',
      stage: 'ticket-review',
      ticket: '.scratch/widgets/issues/01-frobnicate.md',
    })
    if (run.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(run)}`)
    return { h, run }
  }

  it('orders the rest of the tree by rank after the changed files', async () => {
    const { h, run } = await code(RANKED)

    const order = (await treeOrder(h, run)).filter((path) => path.startsWith('src/'))
    expect(order).toEqual(['src/widget.ts', 'src/db.ts', 'src/cache.ts'])
    expect(await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')).toContain('## Ranking')
  })

  it('falls back to path order with a note when code-nav is missing', async () => {
    const { h, run } = await code(MISSING)

    const order = (await treeOrder(h, run)).filter((path) => path.startsWith('src/'))
    expect(order).toEqual(['src/widget.ts', 'src/cache.ts', 'src/db.ts'])
    expect(await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')).toContain('was not used')
  })
})
