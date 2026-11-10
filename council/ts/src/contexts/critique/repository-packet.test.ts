import { readdir, readFile, stat } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  fenced,
  finding,
  harness,
  isolatedExport,
  vibeStdout,
  type Harness,
} from '../../../test/critique/harness.js'
import { evidenceFound, indexPacket } from './domain/evidence.js'
import {
  DEFAULT_PACKET_LIMITS,
  sampleRepository,
  type RepositorySample,
  type SampleRequest,
} from './domain/repository-packet.js'
import type { FanOutResult, PrepareResult } from './index.js'

/**
 * The repository packet (#221): with the code permission on, an ADR review
 * sends the records and references beside a sampled `tree/`, a
 * `tree/MANIFEST.tsv` of every tracked path, and `facts/`, capped by routing.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const ADR_BILLING = [
  '# Billing stays apart',
  '',
  'Invoices live in `src/billing/invoice.ts`, and the long window table at `src/big.ts:812`.',
  '',
].join('\n')
const INVOICE = 'export function invoice(): number {\n  return 1\n}\n'

/** `count` lines of about 60 bytes each, numbered from 1, so a line's text names its own number. */
function numbered(count: number): string {
  return Array.from(
    { length: count },
    (_, i) => `export const line${String(i + 1)} = ${String(i + 1)} // ${'x'.repeat(40)}\n`,
  ).join('')
}

async function repository(
  options: Parameters<typeof harness>[0] & { readonly code?: boolean } = {},
): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  await h.commands.allow.run({ code: options.code ?? true, families: ['mistral'], repo: h.repo })
  await h.writeText(join(h.repo, 'docs', 'adr', '0001-billing.md'), ADR_BILLING)
  await h.writeText(join(h.repo, 'src', 'billing', 'invoice.ts'), INVOICE)
  await h.writeText(join(h.repo, 'src', 'billing', 'index.ts'), "export * from './invoice.js'\n")
  await h.writeText(join(h.repo, 'src', 'unnamed.ts'), 'export const unnamed = 1\n')
  await h.git.commit()
  return h
}

async function prepared(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const run = await h.commands.prepare.run({
    orchestrator_family: 'anthropic',
    repo: h.repo,
    stage: 'adr-review',
    ...extra,
  })
  if (run.status !== 'ready') throw new Error(`not ready: ${JSON.stringify(run)}`)
  return run
}

async function packetTree(dir: string): Promise<string[]> {
  const entries = await readdir(dir, { recursive: true, withFileTypes: true })
  return entries
    .filter((entry) => !entry.isDirectory())
    .map((entry) => join(entry.parentPath, entry.name).slice(dir.length + 1))
    .sort()
}

async function packetBytes(dir: string): Promise<number> {
  const sizes = await Promise.all(
    (await packetTree(dir)).map(async (path) => (await stat(join(dir, path))).size),
  )
  return sizes.reduce((sum, size) => sum + size, 0)
}

describe('an adr-review prepare with the code permission on', () => {
  it('sends the records and references beside a sampled tree, its manifest and the facts index', async () => {
    const h = await repository()

    const run = await prepared(h)

    expect(await packetTree(run.packet_dir)).toEqual([
      'artifact/docs/adr/0001-billing.md',
      'check/adrs.json',
      'facts/INDEX.md',
      'references/CONTEXT.md',
      'tree/MANIFEST.tsv',
      'tree/src/billing/index.ts',
      'tree/src/billing/invoice.ts',
    ])
    expect(
      await readFile(join(run.packet_dir, 'tree', 'src', 'billing', 'invoice.ts'), 'utf8'),
    ).toBe(INVOICE)
    expect(run.packet_files).toEqual([
      'artifact/docs/adr/0001-billing.md',
      'references/CONTEXT.md',
      'check/adrs.json',
      'tree/MANIFEST.tsv',
      'facts/INDEX.md',
      'tree/src/billing/invoice.ts',
      'tree/src/billing/index.ts',
    ])
  })

  it('lists every tracked path with its line count in tree/MANIFEST.tsv', async () => {
    const h = await repository()

    const run = await prepared(h)

    expect(await readFile(join(run.packet_dir, 'tree', 'MANIFEST.tsv'), 'utf8')).toBe(
      [
        'path\tlines\tchurn_rank',
        '.scratch/widgets/spec.md\t7\t-',
        'CONTEXT.md\t3\t-',
        'docs/adr/0001-billing.md\t3\t-',
        'docs/agents/issue-tracker.md\t3\t-',
        'src/billing/index.ts\t1\t-',
        'src/billing/invoice.ts\t3\t-',
        'src/unnamed.ts\t1\t-',
        '',
      ].join('\n'),
    )
  })

  it('records the named files in facts/INDEX.md, and leaves an entry file sent in full unlisted', async () => {
    const h = await repository()

    const run = await prepared(h)

    const index = await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')
    expect(index).toContain('- `src/billing/invoice.ts` (named, 3 lines): sent in full\n')
    expect(index).not.toContain('src/billing/index.ts')
    expect(index).toContain('No hotspot facts yet, so no hotspot was sampled.')
    expect(index).toContain('No import facts yet, so no import cycle was sampled.')
  })

  it('reports the commit it sampled and the packet size against the cap', async () => {
    const h = await repository()

    const run = await prepared(h)

    expect(run.repository).toEqual({
      cap_bytes: 153600,
      dropped_files: 0,
      head_commit: 'c2',
      packet_bytes: await packetBytes(run.packet_dir),
      tree_files: 2,
    })
  })
})

/**
 * A repository far past the 150 KB cap: the 3,000-line file the record names
 * alone is about 180 KB, and twelve modules have an 8 KB entry file each.
 */
async function bigRepository(options: Parameters<typeof harness>[0] = {}): Promise<Harness> {
  const h = await repository(options)
  await h.writeText(join(h.repo, 'src', 'big.ts'), numbered(3000))
  for (let n = 1; n <= 12; n += 1)
    await h.writeText(
      join(h.repo, 'src', `m${String(n).padStart(2, '0')}`, 'index.ts'),
      numbered(130),
    )
  await h.writeText(join(h.repo, 'node_modules', 'left-pad', 'index.js'), 'module.exports = 1\n')
  await h.writeText(join(h.repo, 'vendor', 'lib', 'index.ts'), 'export const vendored = 1\n')
  await h.writeText(join(h.repo, 'package-lock.json'), '{}\n')
  await h.writeText(join(h.repo, 'uv.lock'), 'version = 1\n')
  await h.writeText(join(h.repo, 'dist', 'index.js'), 'built\n')
  await h.writeText(
    join(h.repo, 'src', 'gen', 'index.ts'),
    '// Code generated by protoc. DO NOT EDIT.\nexport const gen = 1\n',
  )
  await h.writeText(join(h.repo, 'assets', 'logo.png'), 'PNG\u0000\u0001')
  await h.git.commit()
  return h
}

describe('a repository packet over a repository larger than the cap', () => {
  it('stays under the 150 KB cap', async () => {
    const run = await prepared(await bigRepository())

    expect(await packetBytes(run.packet_dir)).toBeLessThanOrEqual(150 * 1024)
    expect(run.repository?.packet_bytes).toBe(await packetBytes(run.packet_dir))
  })

  it('sends the named files first, every first window before any second, then the entry files', async () => {
    const run = await prepared(await bigRepository())

    const index = await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')
    const sample = index.split('\n').filter((line) => line.startsWith('- `src/'))
    expect(sample.map((line) => line.slice(0, line.indexOf(')') + 1))).toEqual([
      '- `src/billing/invoice.ts` (named, 3 lines)',
      '- `src/big.ts` (named, 3000 lines)',
      ...Array.from(
        { length: 12 },
        (_, i) => `- \`src/m${String(i + 1).padStart(2, '0')}/\` (1 file, 9014 bytes)`,
      ),
    ])
    expect(run.packet_files).toContain('tree/src/billing/index.ts')
    expect(sample[0]).toContain('sent in full')
    expect(sample[1]).toMatch(/: sent lines 1-\d+00; the cap dropped lines \d+01-3000$/)
  })

  it('lists every file the cap dropped, in whole or in part, in the facts index', async () => {
    const run = await prepared(await bigRepository())

    const index = await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')
    const sent = new Set(
      run.packet_files
        .filter((path) => path.startsWith('tree/src/'))
        .map((path) => path.slice('tree/'.length)),
    )
    const dropped = index.split('\n').filter((line) => line.includes('the cap dropped'))
    expect(dropped.length).toBeGreaterThan(1)
    const counted = dropped.flatMap((line) => /^- `[^`]+\/` \((\d+) files?, /.exec(line)?.[1] ?? [])
    expect(counted.length).toBeGreaterThan(0)
    expect(run.repository?.dropped_files).toBe(
      dropped.length - counted.length + counted.reduce((sum, count) => sum + Number(count), 0),
    )
    for (const line of dropped.filter((entry) => entry.endsWith('the cap dropped it'))) {
      expect(sent.has(line.slice(3, line.indexOf('`', 3)))).toBe(false)
    }
  })

  it('cuts the long file into windows from its first line, at its own line numbers', async () => {
    const run = await prepared(await bigRepository())

    const big = await readFile(join(run.packet_dir, 'tree', 'src', 'big.ts'), 'utf8')
    const lines = big.trimEnd().split('\n')
    expect(lines.length % 400).toBe(0)
    expect(lines.length).toBeLessThan(3000)
    expect(lines.every((line, i) => line.startsWith(`export const line${String(i + 1)} = `))).toBe(
      true,
    )
  })

  it('never sends a vendored, generated, lock or binary file, and keeps the excluded ones out of the manifest', async () => {
    const run = await prepared(await bigRepository())

    const files = await packetTree(run.packet_dir)
    for (const path of [
      'node_modules/left-pad/index.js',
      'vendor/lib/index.ts',
      'package-lock.json',
      'uv.lock',
      'dist/index.js',
      'src/gen/index.ts',
      'assets/logo.png',
    ]) {
      expect(files).not.toContain(`tree/${path}`)
    }
    const manifest = await readFile(join(run.packet_dir, 'tree', 'MANIFEST.tsv'), 'utf8')
    expect(manifest).not.toMatch(/node_modules|vendor\/|package-lock|uv\.lock|dist\/|src\/gen\//)
    expect(manifest).toContain('assets/logo.png\t-\t-\n')
    expect(await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')).toContain(
      'Never sent, and not in `tree/MANIFEST.tsv`: 2 vendored, 2 generated and 2 lock files.',
    )
  })
})

const ADR_QUOTE = {
  file: 'artifact/docs/adr/0001-billing.md',
  kind: 'quote',
  value: 'Invoices live in `src/billing/invoice.ts`',
}
const FACT_QUOTE = {
  file: 'facts/INDEX.md',
  kind: 'quote',
  value: 'No other facts are computed yet.',
}

async function fannedOut(
  h: Harness,
  run: PrepareResult,
  found: Record<string, unknown>,
): Promise<FanOutResult> {
  h.process.queue({
    exitCode: 0,
    exportJson: isolatedExport(),
    stdout: vibeStdout(fenced({ findings: [found] })),
  })
  return h.commands.fanOut.run({ run_dir: run.run_dir })
}

async function preVerdict(
  h: Harness,
  run: PrepareResult,
  found: Record<string, unknown>,
): Promise<unknown> {
  return (await fannedOut(h, run, found)).findings[0]?.pre_verdict
}

describe('the evidence check on a repository packet', () => {
  it('passes a citation of a windowed file at its last sent line, by its original number', async () => {
    const h = await bigRepository()
    const run = await prepared(h)
    const sent = (await readFile(join(run.packet_dir, 'tree', 'src', 'big.ts'), 'utf8'))
      .trimEnd()
      .split('\n').length

    const verdict = await preVerdict(
      h,
      run,
      finding({
        category: 'omission',
        evidence: [ADR_QUOTE, { kind: 'trace', value: `tree/src/big.ts:${String(sent)}` }],
        location: 'src/big.ts:812',
      }),
    )

    expect(verdict).toBeUndefined()
  })

  it('rejects a citation outside the windows the packet holds as fabricated evidence', async () => {
    const h = await bigRepository()
    const run = await prepared(h)
    const sent = (await readFile(join(run.packet_dir, 'tree', 'src', 'big.ts'), 'utf8'))
      .trimEnd()
      .split('\n').length

    const verdict = await preVerdict(
      h,
      run,
      finding({ evidence: [ADR_QUOTE], location: `tree/src/big.ts:${String(sent + 1)}` }),
    )

    expect(verdict).toEqual({
      reason: `the location cites line ${String(sent + 1)} of tree/src/big.ts, outside the lines the packet holds (1-${String(sent)} of 3000)`,
      reason_code: 'fabricated-evidence',
      verdict: 'rejected',
    })
  })

  it('rejects a citation of a file the sample left out as fabricated evidence', async () => {
    const h = await repository()
    const run = await prepared(h)

    const verdict = await preVerdict(
      h,
      run,
      finding({ evidence: [ADR_QUOTE], location: 'src/unnamed.ts:1' }),
    )

    expect(verdict).toMatchObject({
      reason: 'the location cites src/unnamed.ts, which is not in the packet',
      verdict: 'rejected',
    })
  })

  it('resolves a record cited by its repository path and line to its artifact copy', async () => {
    const h = await repository()
    const run = await prepared(h)

    expect(
      await preVerdict(
        h,
        run,
        finding({ evidence: [ADR_QUOTE, FACT_QUOTE], location: 'docs/adr/0001-billing.md:3' }),
      ),
    ).not.toMatchObject({
      verdict: 'rejected',
    })
    expect(
      await preVerdict(
        h,
        run,
        finding({ evidence: [ADR_QUOTE], location: 'docs/adr/0001-billing.md:9' }),
      ),
    ).toEqual({
      reason: 'the location cites line 9 of artifact/docs/adr/0001-billing.md, which has 3 lines',
      reason_code: 'fabricated-evidence',
      verdict: 'rejected',
    })
  })

  it('counts a quote found only in facts/ as neither spec text nor a decision', async () => {
    const h = await repository()
    const run = await prepared(h)

    expect(
      await preVerdict(
        h,
        run,
        finding({ evidence: [FACT_QUOTE], location: 'docs/adr/0001-billing.md' }),
      ),
    ).toMatchObject({
      reason_code: 'no-record-quote',
    })
    // A contradiction needs two records or references; a fact is neither.
    expect(
      await preVerdict(
        h,
        run,
        finding({ evidence: [ADR_QUOTE, FACT_QUOTE], location: 'docs/adr/0001-billing.md' }),
      ),
    ).toMatchObject({
      reason_code: 'one-file-contradiction',
    })
  })

  it('sets the fact flag, and neither the spec nor the decision flag, for a quote found only in a fact file', () => {
    const packet = indexPacket([
      { kind: 'artifact', path: 'artifact/docs/adr/0001.md', text: 'We decided.' },
      { kind: 'reference', path: 'references/CONTEXT.md', text: 'A word.' },
      { kind: 'fact', path: 'facts/INDEX.md', text: 'churn 12' },
    ])

    const quoted = {
      category: 'drift',
      claim: 'It churns.',
      evidence: [{ kind: 'quote', value: 'churn 12' }],
      location: 'x',
      severity: 'major',
    } as const

    expect(evidenceFound(quoted, packet)).toEqual({
      decision: false,
      fact: true,
      items: 1,
      scenario: false,
      spec: false,
    })
  })
})

describe('a critic of a repository packet', () => {
  it('is pointed at every packet file and told how the sample, its windows and the facts work', async () => {
    const h = await repository()
    const run = await prepared(h)

    await fannedOut(h, run, finding({ evidence: [ADR_QUOTE] }))

    const prompt = await readFile(
      join(run.run_dir, 'critics', 'c1-vibe-mistral-large', 'prompt.md'),
      'utf8',
    )
    expect(prompt).toContain(
      [
        '- `tree/MANIFEST.tsv`: a fact the runner computed from the repository: settled, so cite it and never recompute it',
        '- `facts/INDEX.md`: a fact the runner computed from the repository: settled, so cite it and never recompute it',
        '- `tree/src/billing/invoice.ts`: a file of the repository, sampled at the commit under review',
      ].join('\n'),
    )
    expect(prompt).toContain('A file cut short holds its first lines at their own numbers.')
    expect(prompt).toContain('never a recorded decision')
  })
})

describe('the packet limits routing sets', () => {
  it('ships a 150 KB cap, a 30 KB facts cap and 400-line windows', async () => {
    const run = await prepared(await repository())

    expect(await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')).toContain(
      'Packet cap 153600 bytes; facts cap 30720 bytes; windows of 400 lines.',
    )
  })

  it("takes this machine's override of the cap, the facts cap and the window size", async () => {
    const run = await prepared(
      await bigRepository({
        routingOverride: { packet: { cap_bytes: 40000, facts_cap_bytes: 1000, window_lines: 100 } },
      }),
    )

    expect(await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')).toContain(
      'Packet cap 40000 bytes; facts cap 1000 bytes; windows of 100 lines.',
    )
    expect(await packetBytes(run.packet_dir)).toBeLessThanOrEqual(40000)
    const big = (await readFile(join(run.packet_dir, 'tree', 'src', 'big.ts'), 'utf8'))
      .trimEnd()
      .split('\n').length
    expect(big % 100).toBe(0)
    expect(await readFile(join(run.packet_dir, 'facts', 'INDEX.md'), 'utf8')).toContain(
      `- \`src/big.ts\` (named, 3000 lines): sent lines 1-${String(big)}; the cap dropped lines ${String(big + 1)}-3000`,
    )
    expect(run.repository?.cap_bytes).toBe(40000)
  })

  it('refuses a packet whose documents and manifest alone pass the cap, before any packet exists', async () => {
    const h = await repository({ routingOverride: { packet: { cap_bytes: 500 } } })

    const result = await h.commands.prepare.run({
      orchestrator_family: 'anthropic',
      repo: h.repo,
      stage: 'adr-review',
    })

    expect(result).toMatchObject({ reason: 'packet-over-cap', status: 'refused' })
    expect((result as { message: string }).message).toMatch(
      /past the packet cap of 500; review fewer records or raise packet.cap_bytes in routing$/,
    )
  })
})

/**
 * The sample tiers that need facts, at the sampler itself: no scan computes
 * facts yet (#232, #233), so prepare always samples without them.
 */
describe('a sample with facts present', () => {
  const tracked = [
    'src/a.ts',
    'src/b.ts',
    'src/c.ts',
    'src/d.ts',
    'src/e/index.ts',
    'src/hot.ts',
  ].map((path) => ({ path, text: `// ${path}\n` }))

  function sampled(request: Partial<SampleRequest> = {}): RepositorySample {
    const sample = sampleRepository({
      artifactText: 'We keep `./src/a.ts` apart.',
      commit: 'c9',
      documentBytes: 0,
      documents: [],
      excluded: { generated: 0, lock: 0, vendored: 0 },
      facts: {
        churn_ranks: new Map([['src/hot.ts', 1]]),
        cycles: [
          ['src/c.ts', 'src/d.ts'],
          ['src/d.ts', 'src/a.ts'],
        ],
        files: [
          { holds: 'churn per file', name: 'churn.tsv', text: 'x'.repeat(40) },
          { holds: 'hotspots', name: 'hotspots.tsv', text: 'y'.repeat(80) },
          { holds: 'coupling', name: 'coupling.tsv', text: 'z'.repeat(20) },
        ],
        hotspots: ['src/hot.ts', 'src/a.ts', 'src/gone.ts'],
      },
      limits: { ...DEFAULT_PACKET_LIMITS, facts_cap_bytes: 100 },
      tracked,
      ...request,
    })
    if (sample.status !== 'sampled') throw new Error(sample.message)
    return sample
  }

  it('samples the named files, then the hotspots, the entry files and the files on each cycle, each once', () => {
    const sample = sampled()

    expect(sample.tree.map((file) => file.path)).toEqual([
      'src/a.ts',
      'src/hot.ts',
      'src/e/index.ts',
      'src/c.ts',
      'src/d.ts',
    ])
    expect(sample.index).toContain(
      '- `src/a.ts` (named, 1 lines): sent in full\n- `src/hot.ts` (hotspot, 1 lines): sent in full\n',
    )
    expect(sample.index).not.toMatch(/\((entry|cycle), /)
    expect(sample.index).not.toContain('No hotspot facts yet')
  })

  it('sends the fact files that fit under the facts cap, and records the one it dropped', () => {
    const sample = sampled()

    expect(sample.facts.map((file) => file.name)).toEqual(['churn.tsv', 'coupling.tsv'])
    expect(sample.index).toContain(
      [
        '- `facts/churn.tsv`: churn per file',
        '- `facts/hotspots.tsv`: hotspots; the facts cap dropped it',
        '- `facts/coupling.tsv`: coupling',
      ].join('\n'),
    )
    expect(sample.index).not.toContain('No other facts are computed yet.')
  })

  it('gives each path its churn rank in the manifest', () => {
    expect(sampled().manifest).toContain('src/hot.ts\t1\t1\n')
  })

  it('says when nothing was sampled', () => {
    expect(sampled({ artifactText: '', facts: { files: [] }, tracked: [] }).index).toContain(
      'Nothing was sampled.',
    )
  })

  it('sends an empty named file as an empty file', () => {
    const sample = sampled({
      artifactText: 'src/empty.ts',
      facts: { files: [] },
      tracked: [{ path: 'src/empty.ts', text: '' }],
    })

    expect(sample.tree).toEqual([{ path: 'src/empty.ts', text: '' }])
  })
})

describe('an adr-review prepare with the code permission off', () => {
  it('sends no tree/ and no facts/, only the documents and their pre-check', async () => {
    const h = await repository({ code: false })

    const run = await prepared(h)

    expect(await readdir(run.packet_dir)).toEqual(['artifact', 'check', 'references'])
    expect(run.repository).toBeUndefined()
  })
})
