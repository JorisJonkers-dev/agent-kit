import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { ASSETS_ROOT, codeStageRouting, harness, type Harness } from '../../../test/critique/harness.js'
import type { PrepareResult, RepoMapPort } from './index.js'

/**
 * The packet caps hold when `tree/REPO-MAP.md` is built from code-nav's map
 * (#529), end to end. The map is faked here.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

/** One Vibe critic on each code stage, with the shipped packet limits and exclusions. */
async function routing(): Promise<unknown> {
  const shipped = JSON.parse(await readFile(join(ASSETS_ROOT, 'routing.json'), 'utf8')) as { readonly packet: object }
  return { ...(codeStageRouting() as object), packet: shipped.packet }
}

interface NavMap {
  readonly files: readonly { path: string; refs: number }[]
  readonly symbols: readonly { path: string; line: number; kind: string; name: string; refs: number }[]
}

function answering(map: NavMap): RepoMapPort {
  return { rank: () => Promise.resolve({ ...map, paths: map.files.map((file) => file.path), status: 'ranked' }) }
}

async function prepared(repoMap: RepoMapPort | undefined, packet: Record<string, number> = {}, extra: readonly string[] = []): Promise<{ h: Harness; run: PrepareResult }> {
  const h = await harness({ ...(repoMap === undefined ? {} : { repoMap }), routing: await routing(), ...(Object.keys(packet).length === 0 ? {} : { routingOverride: { packet } }) })
  open.push(h)
  await h.commands.allow.run({ code: true, families: ['mistral'], repo: h.repo })
  await h.writeText(join(h.repo, '.scratch/widgets/issues/01-frobnicate.md'), '# 01: Frobnicate\n\n- [ ] Counts.\n')
  await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widget = 1\n')
  await h.writeText(join(h.repo, 'src/cache.ts'), 'export const cache = 1\nexport const other = 1\n')
  await h.writeText(join(h.repo, 'src/db.ts'), 'export const db = 1\n')
  for (const path of extra) await h.writeText(join(h.repo, path), `export const x = '${path}'\n`)
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

const read = (run: PrepareResult, path: string): Promise<string> => readFile(join(run.packet_dir, path), 'utf8')

describe("the packet caps with code-nav's map", () => {
  const many = Array.from({ length: 40 }, (_, i) => `src/m${String(i).padStart(2, '0')}.ts`)
  const big: NavMap = {
    files: many.map((path, i) => ({ path, refs: 40 - i, symbols: 1 })),
    symbols: many.map((path, i) => ({ kind: 'const', line: 1, name: 'x', path, refs: 40 - i })),
  }

  it('keeps the map within repo_map_cap_bytes, leaves out the least referenced files and says how many', async () => {
    const { run } = await prepared(answering(big), { cap_bytes: 60_000, repo_map_cap_bytes: 1000 }, many)

    const map = await read(run, 'tree/REPO-MAP.md')
    expect(Buffer.byteLength(map)).toBeLessThanOrEqual(1000)
    expect(map).toMatch(/\nThe map's cap of 1000 bytes left out the \d+ least referenced files\.\n$/)
    expect(map).toContain('## `src/m00.ts`')
    expect(map).not.toContain('## `src/m39.ts`')
    expect(await read(run, 'facts/INDEX.md')).toMatch(/code-nav's map ranks.*; its cap left out the \d+ least referenced files\./)
  })

  it('keeps the whole packet within cap_bytes', async () => {
    const { h, run } = await prepared(answering(big), { cap_bytes: 12_000, repo_map_cap_bytes: 2000 }, many)

    const manifest = JSON.parse(await h.readText(run.manifest_path)) as { packet_files: { path: string }[] }
    const sizes = await Promise.all(manifest.packet_files.map(async (file) => Buffer.byteLength(await read(run, file.path))))
    expect(sizes.reduce((sum, size) => sum + size, 0)).toBeLessThanOrEqual(12_000)
    expect(Buffer.byteLength(await read(run, 'tree/REPO-MAP.md'))).toBeLessThanOrEqual(2000)
  })
})
