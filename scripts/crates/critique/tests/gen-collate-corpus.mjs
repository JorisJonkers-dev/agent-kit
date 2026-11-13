#!/usr/bin/env node
// Writes the corpus the Rust test `collate_corpus` replays: pairs of ASCII strings and what `localeCompare` (the
// `en` locale, with and without `numeric`) says of them.
//
//   node scripts/crates/critique/tests/gen-collate-corpus.mjs [count] [seed] [out.json]
import { writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const [N = '3000', SEED = '11', OUT = join(here, 'fixtures', 'collate.json')] = process.argv.slice(2)
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const all = Array.from({ length: 128 }, (_, code) => String.fromCharCode(code))
const small = 'aAbBzZ0019 -_.\t\u0001:/'.split('')
const word = () => {
  const alphabet = rnd() < 0.7 ? small : all
  return Array.from({ length: Math.floor(rnd() * 7) }, () => alphabet[Math.floor(rnd() * alphabet.length)]).join('')
}
const sign = (n) => (n < 0 ? -1 : n > 0 ? 1 : 0)
const cases = []
for (let i = 0; i < Number(N); i += 1) {
  const a = word()
  const b = rnd() < 0.3 ? `${a}${word()}` : word()
  const numeric = rnd() < 0.5
  cases.push([a, b, numeric, sign(a.localeCompare(b, 'en', numeric ? { numeric: true } : undefined))])
}
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
