import { readdir, readFile, stat } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { ASSETS_ROOT, codeStageRouting, fenced, finding, harness, isolatedExport, vibeStdout, type Harness, type VibeScript } from '../../../test/critique/harness.js'
import type { PrepareResult } from './index.js'

/**
 * Code packets send what a reviewer needs instead of the whole tree (#254):
 * a diff without generated, vendored and lock files, `tree/REPO-MAP.md`,
 * context slices around the change, and a `tree/` capped the way a
 * repository packet is.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const TICKET = '.scratch/widgets/issues/01-frobnicate.md'
const SPEC = '.scratch/widgets/spec.md'

/** One function whose body runs `lines` lines of about 45 bytes, so its file grows past any window the cap allows. */
function filler(name: string, lines: number): string {
  const rows = Array.from({ length: lines }, (_, i) => `    ${String(i)}, // ${'x'.repeat(30)}\n`).join('')
  return `export function ${name}(): number[] {\n  return [\n${rows}  ]\n}\n`
}

const COUNT_TS = `export function count(widgets: number[]): number {\n  return widgets.length\n}\n\n${filler('tally', 150)}`
const WIDGET_TS = (body: string) =>
  `import { count } from './count.js'\n\nexport function frobnicate(widgets: number[]): number {\n  ${body}\n}\n\nexport function polish(): number {\n  return 1\n}\n`
const REPORT_TS = `import { count } from './count.js'\nimport { frobnicate } from './widget.js'\n\nexport function report(widgets: number[]): string {\n  return \`\${String(frobnicate(widgets))} of \${String(count(widgets))}\`\n}\n\n${filler('header', 150)}\nexport function summary(widgets: number[]): number {\n  return frobnicate(widgets)\n}\n`

const SMALL_CAP = { cap_bytes: 9000, facts_cap_bytes: 1000, window_lines: 400 }

/** One Vibe critic on each code stage, with the shipped packet exclusions under these limits. */
async function cappedRouting(limits: Record<string, number> = SMALL_CAP): Promise<unknown> {
  const shipped = JSON.parse(await readFile(join(ASSETS_ROOT, 'routing.json'), 'utf8')) as { readonly packet: object }
  return { ...(codeStageRouting() as object), packet: { ...shipped.packet, ...limits } }
}

/**
 * A repository whose last commit rewrites one function, `frobnicate` in
 * `src/widget.ts`: it calls `count` in `src/count.ts`, and `report` in
 * `src/report.ts` calls it. Every source file is padded far past what a
 * 9,000-byte cap leaves room for, so only slices of them fit.
 */
async function sliceRepository(options: Parameters<typeof harness>[0] = {}): Promise<Harness> {
  const h = await harness({ routing: await cappedRouting(), ...options })
  open.push(h)
  await h.commands.allow.run({ code: true, families: ['mistral'], repo: h.repo })
  await h.writeText(join(h.repo, TICKET), '# 01: Frobnicate widgets\n')
  await h.writeText(join(h.repo, 'src/count.ts'), COUNT_TS)
  await h.writeText(join(h.repo, 'src/widget.ts'), WIDGET_TS('return count(widgets)'))
  await h.writeText(join(h.repo, 'src/report.ts'), REPORT_TS)
  await h.git.commit()
  await h.writeText(join(h.repo, 'src/widget.ts'), WIDGET_TS('return count(widgets) - 1'))
  await h.git.commit()
  return h
}

async function prepareCode(h: Harness, input: Record<string, unknown> = {}): Promise<PrepareResult> {
  const result = await h.commands.prepare.run({
    base: 'HEAD~1',
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: SPEC,
    stage: 'ticket-review',
    ticket: TICKET,
    ...input,
  })
  if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
  return result
}

async function packetTree(dir: string): Promise<string[]> {
  const entries = await readdir(dir, { recursive: true, withFileTypes: true })
  return entries.filter((entry) => !entry.isDirectory()).map((entry) => join(entry.parentPath, entry.name).slice(dir.length + 1)).sort()
}

async function packetBytes(dir: string): Promise<number> {
  const sizes = await Promise.all((await packetTree(dir)).map(async (path) => (await stat(join(dir, path))).size))
  return sizes.reduce((sum, size) => sum + size, 0)
}

interface CodeManifest {
  readonly code: {
    readonly changed_files: readonly string[]
    readonly slices: readonly { readonly path: string; readonly first: number; readonly last: number; readonly reason: string }[]
  }
}

async function manifestOf(h: Harness, result: PrepareResult): Promise<CodeManifest> {
  return JSON.parse(await h.readText(result.manifest_path)) as CodeManifest
}

function critic(findings: readonly unknown[]): VibeScript {
  return {
    exitCode: 0,
    exportJson: isolatedExport(),
    stdout: vibeStdout(fenced(findings.length === 0 ? { findings, no_findings_reason: 'I read the diff.' } : { findings })),
  }
}

const FAILING_TEST = { kind: 'failing-test', value: 'expect(frobnicate([1, 2])).toBe(2)' }

function bug(location: string, ...evidence: readonly { kind: string; value: string }[]): Record<string, unknown> {
  return finding({ category: 'bug', claim: 'frobnicate drops one widget.', evidence, location })
}

async function preVerdictOf(h: Harness, run: PrepareResult, found: Record<string, unknown>): Promise<unknown> {
  h.process.queue(critic([found]))
  return (await h.commands.fanOut.run({ run_dir: run.run_dir })).findings[0]?.pre_verdict
}

async function promptOf(h: Harness, run: PrepareResult): Promise<string> {
  h.process.queue(critic([]))
  await h.commands.fanOut.run({ run_dir: run.run_dir })
  return h.readText(join(run.run_dir, 'critics', 'c1-vibe-mistral-large', 'prompt.md'))
}

describe('the context slices of a code packet', () => {
  it("holds the changed function's whole body, its callee and its caller as windows at their own line numbers", async () => {
    const h = await sliceRepository()

    const run = await prepareCode(h)

    expect((await manifestOf(h, run)).code.slices).toEqual([
      { first: 3, last: 5, path: 'tree/src/widget.ts', reason: '`frobnicate`, which the change touches' },
      { first: 1, last: 3, path: 'tree/src/count.ts', reason: '`count`, which `frobnicate` calls' },
      { first: 4, last: 6, path: 'tree/src/report.ts', reason: '`report`, which calls `frobnicate`' },
      { first: 163, last: 165, path: 'tree/src/report.ts', reason: '`summary`, which calls `frobnicate`' },
    ])
    const widget = (await readFile(join(run.packet_dir, 'tree/src/widget.ts'), 'utf8')).split('\n')
    expect(widget.slice(2, 5)).toEqual(['export function frobnicate(widgets: number[]): number {', '  return count(widgets) - 1', '}'])
    const report = (await readFile(join(run.packet_dir, 'tree/src/report.ts'), 'utf8')).split('\n')
    expect(report.slice(0, 7)).toEqual([
      '… lines 1-3 are not in the packet; facts/INDEX.md says why',
      '',
      '',
      'export function report(widgets: number[]): string {',
      '  return `${String(frobnicate(widgets))} of ${String(count(widgets))}`',
      '}',
      '… lines 7-162 are not in the packet; facts/INDEX.md says why',
    ])
    expect(report.slice(162)).toEqual(['export function summary(widgets: number[]): number {', '  return frobnicate(widgets)', '}', ''])
  })

  it('are listed in the prompt instead of the whole changed files', async () => {
    const h = await sliceRepository()

    const prompt = await promptOf(h, await prepareCode(h))

    expect(prompt).toContain(
      [
        '- `tree/src/widget.ts:3-5`: `frobnicate`, which the change touches',
        '- `tree/src/count.ts:1-3`: `count`, which `frobnicate` calls',
        '- `tree/src/report.ts:4-6`: `report`, which calls `frobnicate`',
        '- `tree/src/report.ts:163-165`: `summary`, which calls `frobnicate`',
      ].join('\n'),
    )
    expect(prompt).not.toContain('- `tree/src/widget.ts`:')
    expect(prompt).not.toContain('- `tree/src/report.ts`:')
    expect(prompt).toContain('a run of lines it does not hold is blank but for a marker on its first line')
  })

  it('let a citation inside a window resolve', async () => {
    const h = await sliceRepository()
    const trace = { kind: 'trace', value: 'tree/src/report.ts:5 calls tree/src/widget.ts:4, which returns count(widgets) - 1.' }

    expect(await preVerdictOf(h, await prepareCode(h), bug('tree/src/widget.ts:3-5', trace))).toBeUndefined()
  })

  it('reject a citation outside every window mechanically', async () => {
    const h = await sliceRepository()

    expect(await preVerdictOf(h, await prepareCode(h), bug('tree/src/report.ts:2', FAILING_TEST))).toEqual({
      reason: 'the location cites line 2 of tree/src/report.ts, outside the lines the packet holds (4-6, 163-165 of 165)',
      reason_code: 'fabricated-evidence',
      verdict: 'rejected',
    })
  })

  it('reject a range that crosses a run of lines the packet does not hold', async () => {
    const h = await sliceRepository()

    expect(await preVerdictOf(h, await prepareCode(h), bug('tree/src/report.ts:4-165', FAILING_TEST))).toEqual({
      reason: 'the location cites line 7 of tree/src/report.ts, outside the lines the packet holds (4-6, 163-165 of 165)',
      reason_code: 'fabricated-evidence',
      verdict: 'rejected',
    })
  })

  it('reject a range that runs past the lines a window holds', async () => {
    const h = await sliceRepository()

    expect(await preVerdictOf(h, await prepareCode(h), bug('tree/src/count.ts:2-9', FAILING_TEST))).toMatchObject({
      reason: 'the location cites line 9 of tree/src/count.ts, outside the lines the packet holds (1-3 of 158)',
    })
  })
})

describe('a code packet over a repository larger than its cap', () => {
  it('stays under its cap and lists every file it dropped or cut short in the facts index', async () => {
    const h = await sliceRepository()

    const run = await prepareCode(h)

    expect(await packetBytes(run.packet_dir)).toBeLessThanOrEqual(9000)
    expect(run.code?.packet_bytes).toBe(await packetBytes(run.packet_dir))
    const index = await readFile(join(run.packet_dir, 'facts/INDEX.md'), 'utf8')
    const cut = index.split('\n').filter((line) => /^- `[^`]+` \(\w+, \d+ lines\): /.test(line) && !line.endsWith('sent in full'))
    expect(cut.length).toBe(run.code?.dropped_files)
    expect(cut).toEqual(
      expect.arrayContaining([
        '- `src/report.ts` (importer, 165 lines): sent lines 4-6, 163-165; the cap dropped lines 1-3, 7-162',
        '- `src/count.ts` (related, 158 lines): sent lines 1-3; the cap dropped lines 4-158',
      ]),
    )
    const tree = await packetTree(run.packet_dir)
    for (const line of cut.filter((entry) => entry.endsWith('the cap dropped it'))) {
      expect(tree).not.toContain(`tree/${line.slice(3, line.indexOf('`', 3))}`)
    }
    expect(index).toContain('- `tree/src/widget.ts:3-5`: `frobnicate`, which the change touches: sent')
    expect(index).toContain('- `src/widget.ts` (changed, 9 lines): sent in full')
  })

  it('records a slice the cap dropped', async () => {
    const h = await sliceRepository({ routing: await cappedRouting({ ...SMALL_CAP, cap_bytes: 4300 }) })

    const run = await prepareCode(h)

    const index = await readFile(join(run.packet_dir, 'facts/INDEX.md'), 'utf8')
    expect(index).toContain('- `tree/src/report.ts:4-6`: `report`, which calls `frobnicate`: sent')
    expect(index).toContain('- `tree/src/report.ts:163-165`: `summary`, which calls `frobnicate`: the cap dropped it')
    expect(await packetBytes(run.packet_dir)).toBeLessThanOrEqual(4300)
    const prompt = await promptOf(h, run)
    expect(prompt).toContain('- `tree/src/report.ts:4-6`')
    expect(prompt).not.toContain('`tree/src/report.ts:163-165`')
  })

  it('records a file the cap dropped whole, and rejects a citation of it', async () => {
    const h = await sliceRepository({ routing: await cappedRouting({ ...SMALL_CAP, cap_bytes: 4000 }) })

    const run = await prepareCode(h)

    expect(await readFile(join(run.packet_dir, 'facts/INDEX.md'), 'utf8')).toContain('- `src/report.ts` (importer, 165 lines): the cap dropped it')
    expect(await packetTree(run.packet_dir)).not.toContain('tree/src/report.ts')
    expect(await preVerdictOf(h, run, bug('tree/src/report.ts:5', FAILING_TEST))).toMatchObject({
      reason: 'the location cites tree/src/report.ts, which is not in the packet',
    })
  })

  it('sends the files the change touched, then their importers, then the rest of the tree as the cap allows', async () => {
    const h = await sliceRepository({ routingOverride: { packet: { cap_bytes: 60000 } } })

    const run = await prepareCode(h)

    const index = await readFile(join(run.packet_dir, 'facts/INDEX.md'), 'utf8')
    expect(index).toContain('- `src/widget.ts` (changed, 9 lines): sent in full\n- `src/report.ts` (importer, 165 lines): sent in full')
    expect(await readFile(join(run.packet_dir, 'tree/src/count.ts'), 'utf8')).toBe(COUNT_TS)
    expect(index).toContain('- `src/count.ts` (related, 158 lines): sent in full')
    expect(index).not.toContain('`CONTEXT.md` (')
    expect(await packetTree(run.packet_dir)).toContain('tree/CONTEXT.md')
  })

  it('refuses a packet whose diff, tickets, spec, repo map and facts index alone pass the cap', async () => {
    const h = await sliceRepository({ routingOverride: { packet: { cap_bytes: 1500 } } })

    const result = await h.commands.prepare.run({ base: 'HEAD~1', orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, stage: 'ticket-review', ticket: TICKET })

    expect(result).toMatchObject({ reason: 'packet-over-cap', status: 'refused' })
    expect((result as { message: string }).message).toMatch(/past the packet cap of 1500; review a smaller change or raise packet.cap_bytes in routing$/)
  })

  it('follows the same rules for a feature review', async () => {
    const h = await sliceRepository()

    const run = await prepareCode(h, { stage: 'feature-review', ticket: undefined })

    expect(await packetBytes(run.packet_dir)).toBeLessThanOrEqual(9000)
    expect((await manifestOf(h, run)).code.slices.map((slice) => slice.path)).toEqual(['tree/src/widget.ts', 'tree/src/count.ts', 'tree/src/report.ts', 'tree/src/report.ts'])
    expect(run.packet_files[0]).toBe('tree/REPO-MAP.md')
    expect(await preVerdictOf(h, run, bug('tree/src/report.ts:2', FAILING_TEST))).toMatchObject({ reason_code: 'fabricated-evidence' })
  })
})

describe('the repo map of a code packet', () => {
  it('lists each source file with its top-level symbols and signatures, ranked by references', async () => {
    const h = await sliceRepository({ routingOverride: { packet: { cap_bytes: 60000 } } })

    const run = await prepareCode(h)

    const map = await readFile(join(run.packet_dir, 'tree/REPO-MAP.md'), 'utf8')
    expect(map.split('\n').filter((line) => line.startsWith('## '))).toEqual([
      '## `src/count.ts` (referenced by 2 files)',
      '## `src/widget.ts` (referenced by 1 files)',
      '## `src/report.ts` (referenced by 0 files)',
    ])
    expect(map).toContain(
      '- `count` (function, line 1, 2 refs): `export function count(widgets: number[]): number`\n- `tally` (function, line 5, 0 refs): `export function tally(): number[]`',
    )
    expect(map).toContain('- `report` (function, line 4, 0 refs): `export function report(widgets: number[]): string`')
  })

  it('is named first in the prompt', async () => {
    const h = await sliceRepository()

    const prompt = await promptOf(h, await prepareCode(h))

    const list = prompt.slice(prompt.indexOf('# Files'))
    expect(list.split('\n').find((line) => line.startsWith('- '))).toMatch(/^- `tree\/REPO-MAP\.md`: /)
  })

  it('stays within its cap, leaving out the least referenced files and saying how many', async () => {
    const h = await sliceRepository({ routingOverride: { packet: { cap_bytes: 60000, repo_map_cap_bytes: 1000 } } })

    const run = await prepareCode(h)

    const map = await readFile(join(run.packet_dir, 'tree/REPO-MAP.md'), 'utf8')
    expect(Buffer.byteLength(map)).toBeLessThanOrEqual(1000)
    expect(map).toMatch(/\nThe map's cap of 1000 bytes left out the \d least referenced files\.\n$/)
    expect(map).toContain('## `src/count.ts`')
    expect(map).not.toContain('## `src/report.ts`')
    expect(await readFile(join(run.packet_dir, 'facts/INDEX.md'), 'utf8')).toMatch(/its cap left out the \d least referenced files\./)
  })
})

describe('the diff of a code packet', () => {
  const BUNDLE = (n: number) => `#!/usr/bin/env node\n// Generated by scripts/build.mjs. Do not edit.\n${'var x = 1;\n'.repeat(n)}`

  it('leaves out the hunks of a staged generated bundle, keeping one line that names it and says why', async () => {
    const h = await sliceRepository({ routingOverride: { packet: { cap_bytes: 60000 } } })
    await h.writeText(join(h.repo, 'skills/critique/critique.mjs'), BUNDLE(10))
    await h.git.commit()
    await h.writeText(join(h.repo, 'skills/critique/critique.mjs'), BUNDLE(700))
    await h.writeText(join(h.repo, 'src/widget.ts'), WIDGET_TS('return count(widgets) - 2'))
    await h.git.stage()

    const run = await prepareCode(h, { base: 'HEAD', staged: true })

    const diff = await readFile(join(run.packet_dir, 'diff/change.diff'), 'utf8')
    expect(diff).not.toContain('var x = 1;')
    expect(diff).toContain(
      'Left out of this diff: skills/critique/critique.mjs, a generated file: its first lines say a tool generated it; its 714 changed lines are not shown.\n',
    )
    expect(diff).toContain('+  return count(widgets) - 2\n')
    expect(run.packet_files.join('\n')).not.toContain('critique.mjs')
    expect((await manifestOf(h, run)).code.changed_files).toEqual(['tree/src/widget.ts'])
    expect(await packetTree(run.packet_dir)).not.toContain('tree/skills/critique/critique.mjs')
    expect(await promptOf(h, run)).not.toContain('critique.mjs')
    expect(await readFile(join(run.packet_dir, 'facts/INDEX.md'), 'utf8')).toContain(
      '- `skills/critique/critique.mjs`: a generated file: its first lines say a tool generated it; 714 changed lines',
    )
  })

  it('leaves lock files and vendored paths routing matches out of the diff, the tree and the changed files', async () => {
    const h = await sliceRepository({ routingOverride: { packet: { cap_bytes: 60000 } } })
    await h.writeText(join(h.repo, 'package-lock.json'), '{"lockfileVersion": 3}\n')
    await h.writeText(join(h.repo, 'vendor/pad/index.ts'), 'export const pad = 1\n')
    await h.git.commit()

    const run = await prepareCode(h)

    const diff = await readFile(join(run.packet_dir, 'diff/change.diff'), 'utf8')
    expect(diff).toContain(
      'Left out of this diff: package-lock.json, a lock file: it matches the routing glob `**/package-lock.json`; its 1 changed lines are not shown.\n',
    )
    expect(diff).toContain('Left out of this diff: vendor/pad/index.ts, a vendored file: it matches the routing glob `**/vendor/**`; its 1 changed lines are not shown.\n')
    expect(diff).not.toContain('lockfileVersion')
    const files = await packetTree(run.packet_dir)
    expect(files).not.toContain('tree/package-lock.json')
    expect(files).not.toContain('tree/vendor/pad/index.ts')
    expect((await manifestOf(h, run)).code.changed_files).toEqual([])
    expect(await readFile(join(run.packet_dir, 'facts/INDEX.md'), 'utf8')).toContain('Never sent under `tree/`: 1 vendored, 0 generated and 1 lock files, and 0 binary files.')
  })
})
