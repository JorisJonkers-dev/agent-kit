#!/usr/bin/env node
// Writes the corpus the Rust test `prompt_corpus` replays: generated prompt sources, packet file lists, stages, context
// slices and toolsets, and the prompt text and hashes the node runner assembles for each (domain/prompt.ts).
//
//   node scripts/crates/critique/tests/gen-prompt-corpus.mjs [count] [seed] [out.json]
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '300', SEED = '3', OUT = join(here, 'fixtures', 'prompt-corpus.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-prompt-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { assemblePrompt, READ_ONLY_TOOLSET }\n`)
const { assemblePrompt, READ_ONLY_TOOLSET } = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })

let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const int = (lo, hi) => lo + Math.floor(rnd() * (hi - lo + 1))
const chance = (p) => rnd() < p
const STAGES = ['decisions', 'spec', 'tickets', 'ticket-review', 'feature-review', 'adr-review', 'decision-recovery', 'architecture-review', 'plan']
const KINDS = ['artifact', 'reference', 'tree', 'diff', 'ticket', 'spec', 'check', 'feature', 'fact', 'history']
const text = () => pick('', ' ', '  lead', 'trail  ', '\n\nbody\n', 'é → “q”', 'a\r\nb', 'line one\nline two\n\n', '{"type":"object"}', '  {\n "a": 1\n}\n')
const file = () => ({ kind: pick(...KINDS), path: pick('artifact/spec.md', 'check/adrs.json', 'check/tickets.json', 'facts/INDEX.md', 'tree/REPO-MAP.md', 'tree/src/a.ts', 'diff/change.diff', 'tickets/01.md', 'references/x.md', 'history/log.md', 'é/ü.md') })
const toolset = () => pick(
  () => ({ kind: 'vibe-read' }),
  () => ({ kind: 'agy', search: false }),
  () => ({ kind: 'agy', search: true }),
  () => ({ kind: 'vibe-search', search: pick(['grep'], ['grep', 'list'], ['grep', 'list', 'symbol'], ['symbol']), limits: { max_matches: pick(20, 50), max_bytes: pick(4000, 20000) }, server: { command: 'critique', args: ['search-server'] } }),
)()
const cases = []
for (let i = 0; i < Number(N); i++) {
  const sources = { preamble: text(), role: text(), lenses: Array.from({ length: int(0, 3) }, (_, k) => ({ name: pick(`lens-${k}`, String(k), 'a', '10'), text: text() })), contract: text(), schema: text() }
  const files = Array.from({ length: int(0, 6) }, file)
  const stage = pick(...STAGES)
  const slices = chance(0.5) ? [] : Array.from({ length: int(1, 3) }, () => ({ path: 'tree/src/a.ts', first: int(1, 9), last: int(10, 99), reason: pick('changed', 'calls `x`') }))
  const ts = toolset()
  const real = ts.kind === 'vibe-read' ? READ_ONLY_TOOLSET : ts
  const prompt = assemblePrompt(sources, files, stage, slices, real)
  cases.push({ sources, files, stage, slices, toolset: ts, expected: prompt })
}
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
console.log(`${cases.length} cases`)
