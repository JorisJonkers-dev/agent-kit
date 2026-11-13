#!/usr/bin/env node
// Writes the corpus the Rust test `samples_corpus` replays: what the node runner's tree sampler, drop summary and
// repository sample make of generated trees, limits and pieces.
//
//   node scripts/crates/critique/tests/gen-samples-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so the functions are reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '300', SEED = '23', OUT = join(here, 'fixtures', 'samples.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-samples-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { sampleRepository, summariseDrops, dropSummaryBound, admitPieces, candidateOf, heldText, sampledFile, sentStatus, formatRanges }\n`)
const f = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const int = (lo, hi) => lo + Math.floor(rnd() * (hi - lo + 1))
const some = (make, max) => Array.from({ length: Math.floor(rnd() * (max + 1)) }, make)

const DIRS = ['src', 'lib', 'a', 'b', 'src/deep', 'src/deep/er', 'src/deep/er/est', 'docs', 'x/y/z/w']
const FILES = ['index.ts', 'main.rs', 'lib.rs', 'util.ts', 'a.py', 'b.go', 'README.md', 'Main.java', '__init__.py', 'x.txt', 'Ünï.ts', 'MANIFEST.tsv']
const treePath = () => pick('', '', `${pick(...DIRS)}/`) + pick(...FILES)
const text = () => {
  const lines = pick(0, 1, 2, 3, 7, 12, 40, 150)
  const body = Array.from({ length: lines }, (_, n) => `line ${n} ${'é'.repeat(int(0, 3))}${'x'.repeat(int(0, 12))}`).join('\n')
  return body + pick('', '\n', '\n', '\n\n')
}
const request = () => {
  const paths = [...new Set(some(treePath, 30))].sort()
  const tracked = paths.map((path) => ({ path, text: rnd() < 0.08 ? null : text() }))
  if (paths.length > 0 && rnd() < 0.15) tracked.push({ path: paths[0], text: rnd() < 0.5 ? null : text() })
  const named = some(() => pick(...paths, 'nope.ts', `./${pick(...paths, 'a.ts')}`, `tree/${pick(...paths, 'a.ts')}`, `${pick(...paths, 'a.ts')}.`, 'word', 'src/'), 5).join(pick(' ', '\n', ', ', ' and '))
  const facts = {
    files: some(() => ({ name: `f${int(0, 3)}.md`, holds: pick('hotspots', 'cycles', 'ünï'), text: 'f'.repeat(int(0, 400)) }), 3),
    ...(rnd() < 0.5 ? { hotspots: some(() => pick(...paths, 'gone.ts'), 4) } : {}),
    ...(rnd() < 0.4 ? { cycles: some(() => some(() => pick(...paths, 'gone.ts'), 3), 2) } : {}),
    ...(rnd() < 0.4 ? { churn: Object.fromEntries(paths.filter(() => rnd() < 0.6).map((path) => [path, int(1, 5)])) } : {}),
  }
  const limits = {
    cap_bytes: pick(150, 600, 1500, 4000, 20000, 200000),
    facts_cap_bytes: pick(10, 300, 5000),
    manifest_cap_bytes: pick(30, 120, 250, 400, 600, 900, 30720),
    repo_map_cap_bytes: 20480,
    window_lines: pick(1, 2, 5, 20, 400),
    exclude: { generated: [], lock: [], vendored: [] },
  }
  return {
    commit: 'c0ffee',
    limits,
    tracked,
    excluded: { vendored: int(0, 4), generated: int(0, 4), lock: int(0, 4) },
    documents: some(() => pick(...paths, 'docs/d.md'), 2),
    documentBytes: pick(0, 50, 400, 3000),
    artifactText: named,
    facts,
    ...(rnd() < 0.4 ? { rest: some(() => pick(...paths, 'gone.ts'), 5) } : {}),
    ...(rnd() < 0.3 ? { rankingNote: 'a ranking note' } : {}),
  }
}
const toNode = (r) => ({ ...r, facts: { files: r.facts.files, ...(r.facts.hotspots ? { hotspots: r.facts.hotspots } : {}), ...(r.facts.cycles ? { cycles: r.facts.cycles } : {}), ...(r.facts.churn ? { churn_ranks: new Map(Object.entries(r.facts.churn)) } : {}) } })

const drops = () => {
  const REASONS = ['the cap dropped them', 'the cap cut them short: each holds only its first lines, in windows of 5 lines', 'other reason']
  return some(() => ({ path: `${pick('', 'a/', 'b/c/', 'a/b/c/d/', 'x/y/', 'ünï/', 'a/b/', `d${int(0, 30)}/`, `d${int(0, 12)}/e${int(0, 5)}/`, `m${int(0, 9)}/n${int(0, 9)}/o${int(0, 9)}/`)}${pick('f.ts', 'g.rs', 'h')}${int(0, 40)}`, reason: pick(...REASONS), bytes: int(0, 5000) }), pick(3, 12, 60, 120))
}

const pieces = () => {
  const candidates = some(() => f.candidateOf(`p${int(0, 9)}.ts`, pick('named', 'rest'), Array.from({ length: int(1, 30) }, (_, n) => `l${n}${'z'.repeat(int(0, 9))}`).join('\n') + pick('', '\n')), 3)
  if (candidates.length === 0) return null
  const offered = []
  candidates.forEach((candidate, index) => {
    for (let i = 0; i < int(0, 4); i += 1) {
      const first = int(1, candidate.lineCount + 2)
      const window = rnd() < 0.5
      offered.push({ candidate: index, ranges: some(() => ({ first, last: first + int(0, 8) }), 2).concat([{ first, last: first + int(0, 8) }]), window, ...(rnd() < 0.5 ? { note: [`- slice ${index} in`, `- slice ${index} out`] } : {}) })
    }
  })
  return { candidates: candidates.map((c) => ({ path: c.path, tier: c.tier, text: c.text })), pieces: offered, budget: int(0, 900), describe: pick(null, 'named', 'all') }
}

const out = []
for (let i = 0; i < Number(N); i += 1) {
  const kind = pick('sample', 'sample', 'sample', 'sample', 'drops', 'pieces', 'pieces')
  if (kind === 'sample') {
    const input = request()
    const result = f.sampleRepository(toNode(input))
    out.push({ kind, input, expected: result })
  } else if (kind === 'drops') {
    const list = drops()
    out.push({ kind, input: { drops: list }, expected: { lines: f.summariseDrops(list), bound: f.dropSummaryBound(list) } })
  } else {
    const input = pieces()
    if (input === null) continue
    const candidates = input.candidates.map((c) => f.candidateOf(c.path, c.tier, c.text))
    const offered = input.pieces.map((p) => ({ candidate: candidates[p.candidate], ranges: p.ranges, window: p.window, ...(p.note ? { note: (admitted) => p.note[admitted ? 0 : 1] } : {}) }))
    const describe = input.describe === null ? undefined : (c, held) => (input.describe === 'all' || c.tier === 'named' ? `- ${c.path}: ${f.formatRanges(held)}` : undefined)
    const { held, admitted } = f.admitPieces(offered, input.budget, describe)
    out.push({
      kind,
      input,
      expected: {
        held: candidates.map((c) => held.get(c) ?? []),
        admitted: offered.map((p) => admitted.has(p)),
        texts: candidates.map((c) => f.heldText(c, held.get(c) ?? [])),
        files: candidates.map((c) => f.sampledFile(c, held.get(c) ?? []) ?? null),
        status: candidates.map((c) => f.sentStatus(c, held.get(c) ?? [])),
      },
    })
  }
}
const canon = (v) => (Array.isArray(v) ? v.map(canon) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])])) : v)
writeFileSync(OUT, JSON.stringify(canon(out)))
const kinds = {}
for (const c of out) kinds[`${c.kind}${c.expected.status === 'refused' ? ' refused' : ''}`] = (kinds[`${c.kind}${c.expected.status === 'refused' ? ' refused' : ''}`] ?? 0) + 1
console.log(`${out.length} cases`, kinds)
