#!/usr/bin/env node
// Writes the corpus the Rust test `code_corpus` replays: what the node runner's source graph, repo map and context
// slices make of generated modules.
//
//   node scripts/crates/critique/tests/gen-code-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so the functions are reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '200', SEED = '41', OUT = join(here, 'fixtures', 'code.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-code-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { contextSlices, SourceGraph, buildRepoMap, renderRepoMap, renderCodeNavMap, changedLines }\n`)
const f = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const int = (lo, hi) => lo + Math.floor(rnd() * (hi - lo + 1))
const some = (make, max) => Array.from({ length: Math.floor(rnd() * (max + 1)) }, make)

const NAMES = ['alpha', 'beta', 'gamma', 'delta', 'eps', 'zeta', 'eta']
const FILES = ['a', 'b', 'c', 'd', 'e']
const ts = (i) => {
  const imports = FILES.filter((_, j) => j !== i && rnd() < 0.4).map((name) => `import { ${pick(...NAMES)}${pick('', 'X')} } from './${name}'`)
  const decls = some(() => {
    const name = pick(...NAMES) + int(0, 2)
    const calls = some(() => `${pick(...NAMES)}${int(0, 2)}()`, 3)
    return pick(
      `export function ${name}(x: number) {\n  return ${calls.join(' + ') || 'x'}\n}\n`,
      `export const ${name} = (y: string) => {\n  ${calls.join(';\n  ')}\n}\n`,
      `class ${name[0].toUpperCase()}${name.slice(1)} {\n  run() { return ${calls[0] ?? '1'} }\n}\n`,
    )
  }, 5)
  return `${imports.join('\n')}\n${decls.join('\n')}`
}
const py = () => `from .a import ${pick(...NAMES)}0\n\ndef ${pick(...NAMES)}1(x):\n    return ${pick(...NAMES)}0(x)\n\nclass K:\n    pass\n`
const module = (name, i) => (name.endsWith('.py') ? py() : ts(i))
const modules = () => {
  const names = [...FILES.map((n) => `src/${n}.ts`), ...some(() => pick('src/p.py', 'lib/q.js', 'docs/readme.md', 'src/deep/r.ts', 'data.json', 'tests/a.test.ts', 'README.md'), 4)]
  const docsOnly = rnd() < 0.08
  const unique = [...new Set(docsOnly ? ['README.md', 'data.json'] : names)].filter(() => rnd() < 0.85)
  const made = unique.map((path, i) => ({ path, text: path.endsWith('.md') || path.endsWith('.json') ? pick('# t\n', '{}\n', 'x\n'.repeat(int(1, 30))) : module(path, i % FILES.length) }))
  return made.length > 0 && rnd() < 0.25 ? [...made, { path: made[0].path, text: made[0].path.endsWith('.ts') ? ts(0) : 'again\n' }] : made
}
const edit = (text) => {
  const lines = text.split('\n')
  if (lines.length > 2 && rnd() < 0.5) lines.splice(int(0, lines.length - 1), int(0, 2))
  if (rnd() < 0.6) lines.splice(int(0, lines.length), 0, ...some(() => `// edit ${int(0, 99)} ${pick(...NAMES)}0()`, 3))
  if (rnd() < 0.2) lines[int(0, lines.length - 1)] = `export function ${pick(...NAMES)}0() { return 1 }`
  return lines.join('\n')
}
const changedFiles = (texts) => texts.filter(() => rnd() < 0.35).map(({ path, text }) => ({ path, text: pick(edit(text), edit(text), text), before: pick(text === '' ? '' : edit(text), undefined, '', text) }))

const cases = []
for (let i = 0; i < Number(N); i += 1) {
  const kind = pick('graph', 'graph', 'lines')
  const texts = modules()
  if (kind === 'lines') {
    const old = Array.from({ length: int(0, 14) }, (_, n) => `line ${int(0, 4)}`)
    const next = rnd() < 0.5 ? old.filter(() => rnd() < 0.5) : some(() => `line ${int(0, 6)}`, 12)
    const before = pick(undefined, old.join('\n') + pick('', '\n'))
    const after = next.join('\n') + pick('', '\n')
    cases.push({ kind, input: { before, after }, expected: f.changedLines(before, after) })
  } else if (kind === 'graph') {
    const changed = changedFiles(texts)
    const graph = f.SourceGraph.build(texts)
    const ranking = { files: texts.filter(() => rnd() < 0.6 && rnd() < 0.9).map(({ path }) => ({ path, refs: int(0, 9) })), symbols: some(() => ({ path: pick(...texts.map((t) => t.path), 'src/a.ts'), line: int(1, 40), kind: pick('function', 'class'), name: pick(...NAMES), refs: int(0, 5) }), 8) }
    const capBytes = pick(200, 900, 20480)
    cases.push({
      kind,
      input: { texts, changed, ranking, capBytes },
      expected: {
        slices: f.contextSlices(graph, changed),
        map: f.buildRepoMap(graph),
        scan: f.renderRepoMap(f.buildRepoMap(graph), { commit: 'c0ffee', capBytes }),
        nav: f.renderCodeNavMap(ranking, { commit: 'c0ffee', capBytes }),
        importers: texts.map(({ path }) => f.SourceGraph.build(texts).importers(path)),
        lines: changed.map((file) => f.changedLines(file.before, file.text)),
      },
    })
  }
}
const huge = (n, edit) => Array.from({ length: n }, (_, k) => `row ${k}${edit && k % 2 === 0 ? ' edited' : ''}`).join('\n')
for (const edit of [true, false]) {
  const [before, after] = [huge(2100, false), huge(2100, true)]
  cases.push({ kind: 'lines', input: edit ? { before, after } : { before: huge(2100, false), after: huge(2100, false).split('\n').reverse().join('\n') }, expected: f.changedLines(edit ? before : huge(2100, false), edit ? after : huge(2100, false).split('\n').reverse().join('\n')) })
}
const canon = (v) => (Array.isArray(v) ? v.map(canon) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])])) : v)
writeFileSync(OUT, JSON.stringify(canon(cases)))
const kinds = {}
for (const c of cases) kinds[c.kind] = (kinds[c.kind] ?? 0) + 1
console.log(`${cases.length} cases`, kinds)
