#!/usr/bin/env node
// Writes the corpus the Rust test `scan_corpus` replays: generated sources, and what the node runner's
// import scan, source scan and code facts read from them.
//
//   node scripts/crates/critique/tests/gen-scan-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so its domain functions are reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, posix } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '120', SEED = '11', OUT = join(here, 'fixtures', 'scan-corpus.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-scan-corpus-'))
writeFileSync(
  join(scratch, 'runner.mjs'),
  `${source.slice(0, source.indexOf(entry))}\nexport { languageOf, scanImports, resolveImport2, historyFacts, codeFacts2, scanSource, resolveImport, mentions, mentionLines }\n`,
)
const node = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const between = (low, high) => low + Math.floor(rnd() * (high - low + 1))

const SPACE = ['', '', '', ' ', '  ', '\t', ' ', ' ', '﻿', '　']
const BREAK = ['\n', '\n', '\n', '\r\n', '\r', ' ', '\n\n', ' ']
const TS_SPEC = ['./x', './y.js', './z', '../lib/q', './sub/m', '../../x', 'pkg', '@scope/pkg', './x.js', './w.jsx', '.', '..', './', './sub/', 'node:fs', "./é", './a b']
const quote = () => pick("'", "'", '"')
const spec = (pool) => pick(...pool)
const ts = () => {
  const q = quote()
  const name = pick('a', 'bC', '$d', '_e', 'f1')
  return pick(
    () => `import { ${name}, b as c } from ${q}${spec(TS_SPEC)}${q}`,
    () => `import ${name}, * as y from ${q}${spec(TS_SPEC)}${q};`,
    () => `export * from ${q}${spec(TS_SPEC)}${q}`,
    () => `export { ${name} } from ${q}${spec(TS_SPEC)}${q}`,
    () => `import ${q}${spec(TS_SPEC)}${q}`,
    () => `const m = require(${q}${spec(TS_SPEC)}${q})`,
    () => `const m = await import ( ${q}${spec(TS_SPEC)}${q} ) ;`,
    () => `import type { T } from ${q}${spec(TS_SPEC)}${q}`,
    () => `import {\n  ${name},\n  b\n} from ${q}${spec(TS_SPEC)}${q}`,
    () => `import${pick('', ' ')}{}${pick('', ' ')}from${pick('', ' ')}${q}${spec(TS_SPEC)}${q}`,
    () => `export const ${name} = (a: number, b: string): string => {`,
    () => `export async function ${name}<T>(x: T, y = (1)): Promise<T> {`,
    () => `export default class ${name} extends B<T> {`,
    () => `type Foo<T> = {`,
    () => `export interface ${name} {`,
    () => `declare const enum E {`,
    () => `const ${name}: Map<string, number> = new Map()`,
    () => `let ${name} = async x => x`,
    () => `var ${name} = <T>(x: T) => x`,
    () => `const ${name} = x => { return x }`,
    () => `const ${name} = async (a) =>`,
    () => `export abstract class ${name}`,
    () => `function* ${name}() {`,
    () => `namespace N {`,
    () => `const ${name} = ${pick('1', "'a'", '{', '[', 'foo(bar)', '(1 + 2)', 'a == b', 'a >= b', '=')}`,
    () => `  nested = ${pick('1', "import x from './in'")}`,
    () => `export function longName(${'parameter, '.repeat(between(15, 30))}last): void {`, () => `const ${'longValue'.repeat(between(12, 30))} = (a) =>`,
    () => '}', () => ')', () => '});', () => '', () => '   ', () => '// import x from "./commented"', () => '/* import y from "./block" */', () => '/*', () => '*/',
    () => `x = ${q}from ${q}`, () => `from ${q}./loose${q}`, () => `importx from ${q}./y${q}`, () => `reimport(${q}./z${q})`, () => `require (${q}./z${q}) ; require(${q}./w${q})`,
    () => `${q}from ${q}./string${q}`, () => `export${pick('', ' ')}default ${name}`, () => `\`import x from './tpl'\``,
  )()
}
const PY_MOD = ['n', '.n', '..n', '...n', '.', '..', 'pkg.n', 'a.b', '.a.b', 'x', '..a..b', 'pkg', 'os', '.n.', '']
const py = () => {
  const name = pick('f', 'g', 'N', '_h', 'x1')
  return pick(
    () => `from ${spec(PY_MOD)} import ${name}`,
    () => `from ${spec(PY_MOD)} import ${name}, g as h`,
    () => `from ${spec(PY_MOD)} import (${name},\n    g as h,\n)`,
    () => `from ${spec(PY_MOD)} import ()`,
    () => `from ${spec(PY_MOD)} import *`,
    () => `from ${spec(PY_MOD)} import ${name}  # c, d`,
    () => `from  ${pick('', ' ')}import x`,
    () => `import ${spec(PY_MOD)}`, () => `import ${spec(PY_MOD)}, ${spec(PY_MOD)}`, () => `import os , sys ,pkg.n`,
    () => `import ${spec(PY_MOD)} as z`,
    () => `def ${name}(x: int) -> int:`, () => `async  def ${name}():`, () => `class ${name}(Base):`, () => `class ${name}:`,
    () => `${name}: int = 3`, () => `${name} = 5`, () => `${name} == 3`, () => `${name}: Dict[str, int]`, () => `${name} : x = (1)`,
    () => `def long_name(${'parameter, '.repeat(between(15, 30))}last):`, () => `    indented = 1`, () => `# from . import commented`, () => `    # nested comment`, () => '', () => ')', () => `x = "import y"`, () => `if x: import z`,
  )()
}
const KT_NAMES = ['a.b.C', 'a.B', 'a.b.*', 'x.Y', 'kotlin.io', 'a.b.C.', 'C', 'a.b.C as D']
const kt = () => pick(() => `import ${spec(KT_NAMES)}`, () => `import${pick(' ', '  ')}${spec(KT_NAMES)}`, () => `package a.b`, () => `// import a.b.C`, () => `class A`, () => '', () => `  import a.B`, () => `importa.B`)()
const GO_NAMES = ['fmt', 'os', 'a/b', 'github.com/x/y', '']
const go = () => pick(() => `import "${spec(GO_NAMES)}"`, () => `import io "${spec(GO_NAMES)}"`, () => `import (`, () => `)`, () => `\t"${spec(GO_NAMES)}"`, () => `\t_ "${spec(GO_NAMES)}"`, () => `\tio "${spec(GO_NAMES)}" `, () => `import ( "a" )`, () => `var s = "not/import"`, () => `  "indented"`, () => 'package main', () => '', () => '// import "c"', () => `import. "x"`)()
const RS_NAMES = ['a', 'a::b', 'crate::a', 'std::io::', 'self::x::', 'a:', 'b', 'c', 'missing', 'r#x']
const rs = () => pick(() => `use ${spec(RS_NAMES)};`, () => `pub use ${spec(RS_NAMES)}::{y, z};`, () => `pub  use ${spec(RS_NAMES)}`, () => `mod ${spec(RS_NAMES.filter((n) => /^\w+$/.test(n)))};`, () => `pub mod ${spec(RS_NAMES.filter((n) => /^\w+$/.test(n)))} ;`, () => `mod c {`, () => `pub(crate) mod d;`, () => `}`, () => '', () => '// mod x;', () => `fn main() {}`)()
const LANGS = [
  ['ts', ts, ['a.ts', 'src/a.ts', 'src/a.tsx', 'lib/b.mts', 'src/c.cts', 'src/d.js', 'src/e.jsx', 'src/f.mjs', 'src/g.cjs', 'a.ts', 'src/deep/er/h.ts']],
  ['py', py, ['a.py', 'pkg/m.py', 'pkg/sub/m.py', 'src/pkg/m.py', 'pkg/__init__.py']],
  ['kt', kt, ['a/b/A.kt', 'src/main/a/B.kt', 'x.kts']],
  ['go', go, ['main.go', 'cmd/x/main.go']],
  ['rs', rs, ['src/lib.rs', 'src/main.rs', 'src/a.rs', 'src/a/mod.rs', 'src/a/b.rs', 'mod.rs', 'x/mod.rs']],
]
const NOISE = ['\'', '"', '(', ')', '{', '}', '[', ']', ',', '.', ';', ':', '*', '/', ' ', '\n', '\\', '#', '$', '=', '>', '<', '😀', 'é']
const mutate = (text) => {
  if (text.length === 0 || rnd() > 0.35) return text
  const at = between(0, text.length - 1)
  return pick(() => text.slice(0, at) + text.slice(at + 1), () => text.slice(0, at) + pick(...NOISE) + text.slice(at), () => text.slice(0, at) + pick(...NOISE) + text.slice(at + 1))()
}
const sourceText = (gen, long = false) => {
  const lines = Array.from({ length: between(0, long ? 40 : 14) }, () => SPACE[0] + (rnd() < 0.15 ? pick(...SPACE) : '') + mutate(gen()))
  return lines.map((line) => line + pick(...BREAK)).join('').replace(/(.)$/, (c) => (rnd() < 0.3 ? '' : c))
}
const POOL = [
  'src/x.ts', 'src/x.tsx', 'src/y.ts', 'src/y.js', 'src/z/index.ts', 'src/z/index.js', 'lib/q.ts', 'src/sub/m.ts', 'x.ts', 'src/w.jsx', 'src/w.ts', 'src/sub/index.mjs', 'src/é.ts', 'src/a b.ts',
  'pkg/n.py', 'pkg/n/__init__.py', 'pkg/a/b.py', 'a/b.py', 'a.py', 'x.py', 'pkg/__init__.py', 'pkg/sub/n.py', 'n.py', 'os.py', 'other/pkg/n.py', 'src/pkg/n.py',
  'a/b/C.kt', 'other/a/b/C.kt', 'src/main/a/B.kt', 'x/Y.kt', 'a/B.kt',
  'src/lib.rs', 'src/main.rs', 'src/a.rs', 'src/a/mod.rs', 'src/a/b.rs', 'src/b.rs', 'src/b/mod.rs', 'src/c.rs', 'src/a/c.rs', 'mod.rs', 'x/mod.rs', 'x/a.rs', 'a.rs', 'missing.rs', 'cmd/x/main.go', 'README.md',
]
const subset = (extra = []) => [...new Set([...POOL.filter(() => rnd() < 0.55), ...extra])]
const IDENT = ['a', 'bC', '$d', '_e', 'f1', 'x', 'E', 'N', 'foo', 'é', 'a$', 'T']

const cases = []
function scanCase(fixed) {
  const [, gen, paths] = pick(...LANGS)
  const path = fixed?.path ?? pick(...paths, ...(rnd() < 0.1 ? ['README.md', 'noext', '.ts', 'dir.d/file', 'a.TS'] : []))
  const text = fixed?.text ?? sourceText(gen, rnd() < 0.2)
  const tree = fixed?.tree ?? subset([path])
  const language = node.languageOf(path)
  const imports = language === null ? [] : node.scanImports(language, text)
  const resolved = language === null ? [] : imports.map((found) => node.resolveImport2(language, path, found, new Set(tree)))
  const scanned = node.scanSource(path, text) ?? null
  const sourceResolved = scanned === null ? [] : scanned.imports.map((specifier) => node.resolveImport(path, specifier, new Set(tree)) ?? null)
  cases.push({ kind: 'scan', path, text, tree, expected: { language, imports, resolved, source: scanned, source_resolved: sourceResolved } })
}
function factsCase() {
  const count = between(0, 5)
  const sources = Array.from({ length: count }, () => {
    const [, gen, paths] = pick(...LANGS)
    const path = rnd() < 0.1 ? pick('README.md', 'x.txt') : pick(...paths, ...POOL)
    return { path, text: rnd() < 0.1 ? null : sourceText(gen) }
  })
  if (count > 1 && rnd() < 0.2) sources.push({ ...sources[0], text: rnd() < 0.5 ? null : sourceText(ts) })
  const tree = subset(sources.map((one) => one.path))
  const histories = Array.from({ length: between(0, 3) }, (_, i) => ({ path: `p${i}`, commits: Array.from({ length: between(0, 3) }, () => ({ commit: pick('c1', 'c2', 'c3'), date: pick('2026-01-01T00:00:00Z', '2026-02-02T01:02:03+02:00') })) }))
  cases.push({ kind: 'facts', sources, tree, histories, expected: { facts: node.codeFacts2(sources, new Set(tree)), history: node.historyFacts(histories) } })
}
function graphCase() {
  const names = ['a', 'b', 'c', 'd', 'e', 'f', 'é', '\uffff', '\u{10000}', 'B', 'a b'].slice(0, between(2, 11))
  const paths = names.map((name) => `src/${name}.ts`)
  const sources = paths.map((path) => ({ path, text: Array.from({ length: between(0, 4) }, () => `import x from './${pick(...names, 'zz')}${pick('', '.js')}'`).join('\n') }))
  if (rnd() < 0.5) sources.reverse()
  const tree = [...paths, ...(rnd() < 0.5 ? ['src/zz.ts'] : [])]
  cases.push({ kind: 'facts', sources, tree, histories: [], expected: { facts: node.codeFacts2(sources, new Set(tree)), history: node.historyFacts([]) } })
}
function posixCase() {
  const part = () => pick('a', 'b', '.', '..', '/', '//', 'x.ts', '.ts', '', '...', 'c.d.e', '.hidden', 'a.', 'é', '..a')
  const path = Array.from({ length: between(0, 6) }, part).join(pick('', '/', '/', '//'))
  const other = Array.from({ length: between(1, 5) }, part).filter((one) => one !== '').join('/') || 'z'
  const base = path === '' ? 'q' : path
  cases.push({ kind: 'posix', path, other, base, expected: { normalize: posix.normalize(path), dirname: posix.dirname(path), extname: posix.extname(path), join: posix.join(base, other) } })
}
function mentionCase() {
  const name = pick(...IDENT)
  const text = Array.from({ length: between(0, 6) }, () => Array.from({ length: between(0, 5) }, () => pick(name, ' ', '(', '.', 'x', '$', '_', '1', name + 'x', 'x' + name, 'é' + name, '-')).join('')).join(pick('\n', '\r\n'))
  cases.push({ kind: 'mention', text, name, expected: { mentions: node.mentions(text, name), lines: node.mentionLines(text, name) } })
}
// A fixed block the random draw rarely reaches: forms of import a scan may or may not read, and signatures.
const FIXED = [
  { path: 'pkg/n.py', text: 'import os\nfrom . import cache\nimport sys\nfrom pkg import a, b\nfrom .sub import *\nfrom m import y as z\n', tree: ['pkg/n.py', 'pkg/cache.py', 'pkg/__init__.py', 'pkg/a.py', 'pkg/sub/__init__.py', 'os.py'] },
  { path: 'a.py', text: 'import foo\nimport foo.bar\n', tree: ['a.py', 'foo.py', 'foo/__init__.py', 'foo/bar.py'] },
  { path: 'cmd/x/main.go', text: 'package main\nimport (\n"fmt"\n  alias "os/exec"\n)\nimport ("a"; "b")\nimport "c"\n', tree: ['cmd/x/main.go'] },
  { path: 'src/lib.rs', text: 'pub(crate) mod a;\npub(super) mod b;\npub(in crate::x) mod c;\nmod cache;\nuse cache;\nuse crate::a::thing;\n', tree: ['src/lib.rs', 'src/a.rs', 'src/b.rs', 'src/c.rs', 'src/cache.rs'] },
  { path: 'src/x.tsx', text: 'export const a = <div>{items.map(i => i)}</div>\nexport const f = <T,>(x: T) => x\nexport const enum E { A, B }\nexport const g = async (y) => y\nimport { q } from \'./y\'\n', tree: ['src/x.tsx', 'src/y.ts'] },
]
for (const fixed of FIXED) scanCase(fixed)
for (let i = 0; i < Number(N); i += 1) pick(scanCase, scanCase, factsCase, mentionCase, graphCase, posixCase)()
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
console.log(`${cases.length} cases to ${OUT}`)
