import { describe, expect, it } from 'vitest'

import { sampleCodeTree, type CodeSample, type CodeSampleRequest } from './domain/code-sample.js'
import { DEFAULT_PACKET_LIMITS, sampleRepository, type RepositorySample, type SampleRequest } from './domain/repository-packet.js'

/**
 * The facts index stays small however many files a packet leaves out (#256):
 * drops are counted per directory and reason, and only the files a critic is
 * likely to look for are named one by one.
 */

/** A file of `lines` lines of about 50 bytes each. */
function text(lines: number): string {
  return Array.from({ length: lines }, (_, i) => `export const v${String(i)} = ${String(i)} // ${'x'.repeat(30)}\n`).join('')
}

/** 700 files of 400 lines, each too big for the cap, in 10 directories, `pkg/d0/` to `pkg/d9/`. */
const MANY = Array.from({ length: 700 }, (_, i) => ({ path: `pkg/d${String(i % 10)}/f${String(i).padStart(3, '0')}.ts`, text: text(400) }))
const MANY_BYTES_PER_DIR = 70 * new TextEncoder().encode(text(400)).length

function codeSample(request: Partial<CodeSampleRequest> = {}): CodeSample {
  const sample = sampleCodeTree({
    changed: ['src/widget.ts'],
    commit: 'c9',
    diff: '',
    documentBytes: 0,
    excluded: { generated: 0, lock: 0, vendored: 0 },
    importers: [],
    leftOut: [],
    limits: { ...DEFAULT_PACKET_LIMITS, cap_bytes: 12_000 },
    namedText: '',
    repoMap: { dropped: 0, listed: 0, text: '# Repo map\n' },
    slices: [],
    tracked: [{ path: 'src/widget.ts', text: text(3) }, ...MANY],
    ...request,
  })
  if (sample.status !== 'sampled') throw new Error(sample.message)
  return sample
}

describe('the facts index of a code packet that drops 700 files in 10 directories', () => {
  it('holds one line per directory and reason, with the count and the bytes, and stays under 4 KB', () => {
    const sample = codeSample()

    expect(sample.dropped).toBe(700)
    expect(new TextEncoder().encode(sample.index).length).toBeLessThan(4096)
    const groups = sample.index.split('\n').filter((line) => line.startsWith('- `pkg/'))
    expect(groups).toEqual(
      Array.from({ length: 10 }, (_, d) => `- \`pkg/d${String(d)}/\` (70 files, ${String(MANY_BYTES_PER_DIR)} bytes): the cap dropped them`),
    )
  })
})

const encoded = (part: string) => new TextEncoder().encode(part).length

describe('a code packet that drops a file a critic is likely to look for', () => {
  it('still lists by name a file the diff, a ticket or the spec names, and an importer of a changed file', () => {
    const sample = codeSample({ importers: ['pkg/d4/f004.ts'], namedText: 'The ticket asks about `pkg/d3/f003.ts`.' })

    expect(sample.index).toContain('- `pkg/d3/f003.ts` (rest, 400 lines): the cap dropped it')
    expect(sample.index).toContain('- `pkg/d4/f004.ts` (importer, 400 lines): the cap dropped it')
    expect(sample.index).toContain('- `pkg/d3/` (69 files, ')
    expect(sample.index).toContain('- `pkg/d4/` (69 files, ')
    expect(sample.index).toContain('- `pkg/d5/` (70 files, ')
  })

  it('lists by name a file only the diff names, the diff coming apart from the tickets and the spec', () => {
    const sample = codeSample({ diff: 'diff --git a/src/widget.ts b/src/widget.ts\n--- a/src/widget.ts\n+++ b/src/widget.ts\n@@ -1 +1 @@\n-// see pkg/d6/f006.ts\n+// see pkg/d6/f006.ts.\n' })

    expect(sample.index).toContain('- `pkg/d6/f006.ts` (rest, 400 lines): the cap dropped it')
    expect(sample.index).toContain('- `pkg/d6/` (69 files, ')
  })
})

describe('a code packet that drops files in more directories than the summary has lines for', () => {
  it('counts each file under its first folders, as `dir/**`, as deep as the summary still fits', () => {
    const spread = Array.from({ length: 500 }, (_, i) => ({ path: `pkg/p${String(i % 5)}/q${String(i % 50)}/f${String(i)}.ts`, text: text(400) }))

    const sample = codeSample({ tracked: [{ path: 'README.md', text: text(400) }, { path: 'src/widget.ts', text: text(3) }, ...spread] })

    const groups = sample.index.split('\n').filter((line) => / \(\d+ files?, \d+ bytes\): /.test(line))
    expect(groups.map((line) => line.slice(0, line.indexOf(' ('))).sort()).toEqual([
      '- `./`',
      ...Array.from({ length: 5 }, (_, p) => `- \`pkg/p${String(p)}/**\``),
    ])
    expect(groups).toContain(`- \`pkg/p0/**\` (100 files, ${String(100 * encoded(text(400)))} bytes): the cap dropped them`)
    expect(groups).toContain(`- \`./\` (1 file, ${String(encoded(text(400)))} bytes): the cap dropped them`)
  })

  it('counts every file as `**` when even the top folders pass the summary', () => {
    const top = Array.from({ length: 30 }, (_, i) => ({ path: `t${String(i).padStart(2, '0')}/f.ts`, text: text(400) }))

    const sample = codeSample({ tracked: [{ path: 'src/widget.ts', text: text(3) }, ...top] })

    expect(sample.index).toContain(`- \`**\` (30 files, ${String(30 * encoded(text(400)))} bytes): the cap dropped them`)
    expect(sample.index).not.toContain('`t00/')
  })
})

function repositorySample(request: Partial<SampleRequest> = {}): RepositorySample {
  const sample = sampleRepository({
    artifactText: 'We keep `pkg/d1/f001.ts` apart.',
    commit: 'c9',
    documentBytes: 0,
    documents: [],
    excluded: { generated: 0, lock: 0, vendored: 0 },
    facts: { cycles: [MANY.map((file) => file.path)], files: [], hotspots: ['pkg/d2/f002.ts'] },
    limits: { ...DEFAULT_PACKET_LIMITS, cap_bytes: 30_000 },
    tracked: MANY,
    ...request,
  })
  if (sample.status !== 'sampled') throw new Error(sample.message)
  return sample
}

describe('the facts index of a repository packet that drops 700 files in 10 directories', () => {
  it('counts the dropped files on a cycle per directory, and stays under 4 KB', () => {
    const sample = repositorySample()

    expect(sample.dropped).toBe(700)
    expect(encoded(sample.index)).toBeLessThan(4096)
    const groups = sample.index.split('\n').filter((line) => line.startsWith('- `pkg/d') && line.includes(' files, '))
    expect(groups).toHaveLength(10)
    expect(groups[0]).toBe(`- \`pkg/d0/\` (70 files, ${String(MANY_BYTES_PER_DIR)} bytes): the cap dropped them`)
    expect(groups[1]).toMatch(/^- `pkg\/d1\/` \(69 files, \d+ bytes\): the cap dropped them$/)
  })

  it('still lists by name a dropped file the artifact names, and a dropped hotspot', () => {
    const sample = repositorySample()

    expect(sample.index).toContain('- `pkg/d1/f001.ts` (named, 400 lines): the cap dropped it\n- `pkg/d2/f002.ts` (hotspot, 400 lines): the cap dropped it\n')
  })
})

describe('a tree/MANIFEST.tsv past its cap', () => {
  it('counts its least-ranked paths per directory at its end, says how many, and fits its cap', () => {
    const churn_ranks = new Map(MANY.filter((_, i) => i % 10 === 3).map((file, i) => [file.path, i + 1]))
    const sample = repositorySample({ facts: { churn_ranks, files: [] }, limits: { ...DEFAULT_PACKET_LIMITS, cap_bytes: 30_000, manifest_cap_bytes: 8_000 } })

    expect(encoded(sample.manifest)).toBeLessThanOrEqual(8_000)
    const lines = sample.manifest.trimEnd().split('\n')
    const summary = lines.find((line) => line.startsWith('# collapsed: '))
    const collapsed = Number(/its (\d+) least-ranked paths/.exec(summary ?? '')?.[1])
    expect(summary).toBe(`# collapsed: past the manifest cap of 8000 bytes, its ${String(collapsed)} least-ranked paths are counted per directory below, as directory and paths`)
    const rows = lines.filter((line) => !line.startsWith('#')).slice(1)
    const counts = lines.filter((line) => line.startsWith('# pkg/')).map((line) => Number(line.split('\t')[1]))
    expect(rows.length + collapsed).toBe(700)
    expect(counts.reduce((sum, count) => sum + count, 0)).toBe(collapsed)
    expect(rows.filter((row) => row.startsWith('pkg/d3/'))).toHaveLength(70)
    expect(rows.find((row) => row.startsWith('pkg/d1/f001.ts\t'))).toBeDefined()
    expect(lines.at(-1)).toMatch(/^# pkg\/d9\/\t\d+$/)
    expect(sample.index).toContain(`Past its cap of 8000 bytes, it counts its ${String(collapsed)} least-ranked paths per directory at its end instead of listing them.`)
  })

  it('stays inside its cap even when not every path can be collapsed away', () => {
    const sample = repositorySample({ limits: { ...DEFAULT_PACKET_LIMITS, cap_bytes: 30_000, manifest_cap_bytes: 200 } })
    expect(encoded(sample.manifest)).toBeLessThanOrEqual(200)
    expect(sample.manifest).toBe(
      'path\tlines\tchurn_rank\n# collapsed: past the manifest cap of 200 bytes, its 700 least-ranked paths are left out, and so are their per-directory counts, which would not fit\n',
    )
    expect(sample.index).toContain('Past its cap of 200 bytes, it leaves out its 700 least-ranked paths, and their per-directory counts, which would not fit.')
  })

  it('lists every path while it fits its cap', () => {
    const sample = repositorySample({ limits: { ...DEFAULT_PACKET_LIMITS, cap_bytes: 30_000 } })

    expect(sample.manifest).not.toContain('# collapsed')
    expect(sample.manifest.trimEnd().split('\n')).toHaveLength(701)
    expect(sample.index).not.toContain('least-ranked')
  })
})

describe('a code packet that cuts files of the rest of the tree short', () => {
  const windowed = (cap: number) => codeSample({ limits: { ...DEFAULT_PACKET_LIMITS, cap_bytes: cap, window_lines: 20 } })

  it('counts the files the cap cut short apart from the ones it dropped, and every file once', () => {
    const sample = windowed(40_000)

    const lines = sample.index.split('\n')
    expect(lines.filter((line) => line.endsWith('the cap cut them short: each holds only its first lines, in windows of 20 lines')).length).toBeGreaterThan(0)
    expect(lines.filter((line) => line.endsWith('the cap dropped them')).length).toBeGreaterThan(0)
    const counts = lines.flatMap((line) => /^- `pkg\/d\d\/` \((\d+) files?, /.exec(line)?.[1] ?? [])
    expect(counts.reduce((sum, count) => sum + Number(count), 0)).toBe(700)
    expect(encoded(sample.index)).toBeLessThan(4096)
  })

  it('stays under its cap however the counted drops fall', () => {
    for (const cap of [9_000, 14_000, 25_000, 60_000, 200_000]) {
      const sample = windowed(cap)
      const size = [sample.index, '# Repo map\n', ...sample.tree.map((file) => file.text)].reduce((sum, part) => sum + encoded(part), 0)
      expect(size).toBeLessThanOrEqual(cap)
    }
  })
})
