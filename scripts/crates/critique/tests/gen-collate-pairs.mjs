#!/usr/bin/env node
// Writes every ordered pair of one-character ASCII strings and what `localeCompare` (the `en` locale) says of
// them, with and without `numeric`, for the Rust test `every_ascii_pair_collates_as_node_does`. One line per
// setting, 128 x 128 signs: `-` before, `0` equal, `+` after, row `a` then column `b`.
//
//   node scripts/crates/critique/tests/gen-collate-pairs.mjs [out.txt]
import { writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const [OUT = join(here, 'fixtures', 'collate-pairs.txt')] = process.argv.slice(2)
const chars = Array.from({ length: 128 }, (_, code) => String.fromCharCode(code))
const sign = (n) => (n < 0 ? '-' : n > 0 ? '+' : '0')
const line = (options) => chars.map((a) => chars.map((b) => sign(a.localeCompare(b, 'en', options))).join('')).join('')
writeFileSync(OUT, `${line(undefined)}\n${line({ numeric: true })}\n`)
