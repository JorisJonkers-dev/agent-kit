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
// A fixed block the random draw rarely reaches: digit runs with leading zeros against each other and against
// letters, and each letter against its case twin at the start, middle and end of a word.
const add = (a, b) => {
  for (const numeric of [false, true]) cases.push([a, b, numeric, sign(a.localeCompare(b, 'en', numeric ? { numeric: true } : undefined))])
}
const runs = ['0', '00', '007', '7', '07', '70', '10', '9', '0010', '100', '000']
for (const a of runs) for (const b of [...runs, 'a', 'A', '-', '']) { add(`r${a}`, `r${b}`); add(`${a}x`, `${b}x`) }
for (const letter of 'abcdefghijklmnopqrstuvwxyz') {
  const up = letter.toUpperCase()
  add(letter, up); add(`${letter}a`, `${up}a`); add(`a${letter}a`, `a${up}a`); add(`aa${letter}`, `aa${up}`); add(`${letter}b`, `${up}a`)
}
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
