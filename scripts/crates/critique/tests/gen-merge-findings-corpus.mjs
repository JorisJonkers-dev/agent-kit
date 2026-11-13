#!/usr/bin/env node
// Writes the corpus the Rust test `merge_corpus` replays: generated findings of several critics, and the merged findings
// the node runner's `mergeFindings` (domain/merge.ts) makes of them, as JSON text, so the order of every key is compared.
//
//   node scripts/crates/critique/tests/gen-merge-findings-corpus.mjs [count] [seed] [out.json]
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '300', SEED = '9', OUT = join(here, 'fixtures', 'merge-findings-corpus.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-merge-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { mergeFindings }\n`)
const { mergeFindings } = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })

let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const int = (lo, hi) => lo + Math.floor(rnd() * (hi - lo + 1))
const chance = (p) => rnd() < p
const LOCATIONS = ['a.ts:1', 'A.TS:1', ' a.ts:1 ', '"a.ts:1"', 'a.ts:2', 'spec.md', 'b/c.ts:9.', '`x`', 'É.md', 'é.md', '“q”', 'tab\tsep', 'two  spaces']
const QUOTES = ['The retry is unbounded', 'the retry is unbounded.', 'retry is', 'other text', 'Σίσυφος', 'ΣΊΣΥΦΟΣ', 'a\nb', 'a b', '']
const CLAIMS = ['It fails.', 'it fails', '"It fails"', 'It breaks;', 'It breaks', '..', '“quoted”', 'x']
const finding = (id) => ({
  category: pick('bug', 'bug', 'omission', 'security'), ...(chance(0.5) ? { check: pick('c1', 'c2') } : {}), claim: pick(...CLAIMS),
  evidence: Array.from({ length: int(0, 3) }, () => ({ kind: pick('quote', 'quote', 'input', 'trace'), value: pick(...QUOTES), ...(chance(0.2) ? { file: 'f.md' } : {}) })),
  location: pick(...LOCATIONS), severity: pick('critical', 'major', 'minor'), ...(chance(0.4) ? { suggestion: pick('fix it', 'other fix') } : {}), id,
})
const cases = []
for (let i = 0; i < Number(N); i++) {
  const critics = Array.from({ length: int(0, 4) }, (_, k) => ({ critic_id: `c${k + 1}`, findings: Array.from({ length: int(0, 4) }, (_, j) => finding(`k${k + 1}-${j + 1}`)) }))
  cases.push({ critics, expected: JSON.stringify(mergeFindings(critics)) })
}
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
console.log(`${cases.length} cases`)
