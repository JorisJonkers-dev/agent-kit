#!/usr/bin/env node
// Writes the corpus the Rust test `merge_corpus` replays: generated manifests, conflicts and diffs, and what the node
// runner's `domain/merge-commit.ts` says of them.
//
//   node scripts/crates/critique/tests/gen-merge-corpus.mjs [count] [seed] [out.json]
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '500', SEED = '23', OUT = join(here, 'fixtures', 'merge-commit.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-merge-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { diffOfPaths, differsOnlyInPins, isManifest }\n`)
const { diffOfPaths, differsOnlyInPins, isManifest } = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const some = (make, max) => Array.from({ length: Math.floor(rnd() * (max + 1)) }, make)
const hex = (n) => Array.from({ length: n }, () => '0123456789abcdef'[Math.floor(rnd() * 16)]).join('')
const pin = () => pick(`  sha256: ${hex(64)}`, `- sha256: ${hex(64)}`, `    -   sha256:${hex(64)}  `, `sha256: ${hex(63)}`, `sha256: ${hex(64).toUpperCase()}`, `x sha256: ${hex(64)}`, `\tsha256:\t${hex(64)}\t`)
const plain = () => pick('name: a', 'version: 1', '  path: x/y', '', 'files:', '# comment')
const lines = () => some(() => (rnd() < 0.4 ? pin() : plain()), 6)
const conflict = (a, b) => ['<<<<<<< ours', ...a, '=======', ...b, '>>>>>>> theirs']
const manifestPair = () => {
  const base = lines()
  const shape = rnd()
  const repinned = base.map((line) => (line.includes('sha256') && rnd() < 0.7 ? line.replace(/[0-9a-f]{64}/, hex(64)) : line))
  const auto =
    shape < 0.3
      ? [...base.slice(0, 2), ...conflict(lines(), pick(lines(), base.slice(0, 2))), ...base.slice(2)]
      : shape < 0.4
        ? [...base, '<<<<<<< open', ...lines()]
        : shape < 0.5
          ? [...base, '<<<<<<< x', 'a', '=======', 'a']
          : base
  const merged = rnd() < 0.7 ? repinned : [...repinned, pick('extra: 1', pin())]
  return [auto.join(pick('\n', '\n')), merged.join('\n') + pick('', '\n')]
}
const section = (path, to = path) => `diff --git a/${path} b/${to}\nindex 1..2 100644\n--- a/${path}\n+++ b/${to}\n@@ -1 +1 @@\n-a\n+b\n`
const PATHS = ['a.ts', 'src/b.ts', 'manifest.yaml', 'x/manifest.yaml', 'a b.ts', 'c.ts']
const diff = () => some(() => (rnd() < 0.2 ? section(pick(...PATHS), pick(...PATHS)) : section(pick(...PATHS))), 4).join(pick('', '', 'junk\n', '\r')) + pick('', 'trailer\n')
const cases = []
for (let i = 0; i < Number(N); i += 1) {
  const [auto, merged] = manifestPair()
  const text = diff()
  const paths = [...new Set(some(() => pick(...PATHS, 'none.ts'), 3))]
  const path = pick('manifest.yaml', 'a/manifest.yaml', 'manifest.yml', 'xmanifest.yaml', 'a/manifest.yaml/b', '')
  cases.push({ auto, merged, diff: text, paths, path, expected: { pins: differsOnlyInPins(auto, merged), diff: diffOfPaths(text, paths), manifest: isManifest(path) } })
}
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
const t = cases.filter((c) => c.expected.pins).length
console.log(`${cases.length} cases written to ${OUT}; ${t} differ only in pins`)
