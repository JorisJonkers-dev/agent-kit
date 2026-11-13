#!/usr/bin/env node
// Writes the corpus the Rust test `search_corpus` replays: generated packets (files, symlinks out of the packet, a
// repo map), the JSON-RPC lines a critic sends, and what the node runner's `search-server` printed and exited with.
//
//   node scripts/crates/critique/tests/gen-search-corpus.mjs [count] [seed] [out.json]
//
// Each packet sits in a fresh directory as `packet/`, beside an `outside/` the symlinks point into; the packet's
// real path is written @ROOT@ in the arguments and in what the server printed.
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '160', SEED = '13', OUT = join(here, 'fixtures', 'search-server.json')] = process.argv.slice(2)
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const some = (make, max) => Array.from({ length: Math.floor(rnd() * (max + 1)) }, make)

const LINES = ['export function parseRouting(text) {', 'const a = 1', '  return a', '// TODO: fix', 'Ünïcode línea', 'x'.repeat(320), '😀 emoji 😀', 'tab\there', 'end\r', '', 'parseRouting(x) // call', 'AbC abc ABC', 'reach proof: inside the packet', 'été']
const text = () => some(() => pick(...LINES), 6).join('\n') + pick('', '\n')
const FILES = ['tree/src/a.ts', 'tree/src/b.ts', 'tree/src/deep/c.ts', 'tree/README.md', 'tree/Ünï.md', 'notes.md', 'inside.md', 'tree/z.js', 'tree/src/a b.ts', 'artifact/spec.md']
const MAP = '# Repo map\n\n## `src/a.ts` (referenced by 3 files)\n\n- `parseRouting` (function, line 1, 4 refs): `export function parseRouting(text)`\n- `Routing` (interface, line 9, 2 refs): `interface Routing`\n- `bad line\n## `src/b.ts` (referenced by 1 file)\n- `routingOf` (const, line 3, 0 refs): `const routingOf = 1`\n- `x` (type, line 4, 1 refs): `' + 'y'.repeat(300) + '`\n'
const PATTERNS = ['parseRouting\\(', 'a', '^\\s+return', 'TODO|FIXME', '[A-Z][a-z]+', '\\bconst\\b', 'x{300}', 'routing', 'Ünï', '\\d+', '(a|b)+c', 'é', '^$', '\\.', '(', 'a)', '[a', 'a\\', '*a', '[z-a]', 'reach proof', 'a{2,1}', '\\u00e9t', '[^\\x00-\\x7f]']
const GLOBS = ['**', 'tree/**', 'tree/**/*.ts', '*.md', 'tree/src/*.ts', '{notes,inside}.md', '../**', '/**', 'tree/src/**/c.ts', '**/*.{ts,js}', 'tree/[a', 'nothing/**']
const PATHS = ['tree', 'tree/src', 'tree/src/a.ts', '../outside', '/etc', 'escape', 'escape.md', 'tree/../notes.md', '', 'missing', 'tree/./src', 'esc-dir/x.md']
const NAMES = ['parseRouting', 'routing', 'ROUTING', 'x', 'nothing', '']
const call = (name, args) => ({ jsonrpc: '2.0', id: pick(1, 2, 'a', 3.5, null, -1), method: 'tools/call', params: { name, arguments: args } })
const request = () =>
  pick(
    () => JSON.stringify({ jsonrpc: '2.0', id: 0, method: 'initialize', params: pick({}, { protocolVersion: '2024-11-05' }, { protocolVersion: 5 }) }),
    () => JSON.stringify({ jsonrpc: '2.0', id: pick(1, 'x'), method: 'ping' }),
    () => JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }),
    () => JSON.stringify({ jsonrpc: '2.0', id: 4, method: 'tools/list', params: pick(undefined, [], 5) }),
    () => JSON.stringify({ jsonrpc: '2.0', id: 5, method: 'resources/list' }),
    () => pick('not json', '[1,2]', '{"id":1}', '{"method":5,"id":1}', '   ', '{"id":true,"method":"ping"}', '{"id":{},"method":"ping"}'),
    () => JSON.stringify(call('grep', { pattern: pick(...PATTERNS), ...(rnd() < 0.3 ? { ignore_case: pick(true, false, 'yes') } : {}), ...(rnd() < 0.4 ? { glob: pick(...GLOBS) } : {}), ...(rnd() < 0.4 ? { path: pick(...PATHS) } : {}) })),
    () => JSON.stringify(call('grep', pick({}, { pattern: '' }, { pattern: 5 }, { pattern: 'a', glob: 5 }, { pattern: 'a', path: [] }))),
    () => JSON.stringify(call('list', pick({ glob: pick(...GLOBS) }, {}, { glob: '' }))),
    () => JSON.stringify(call('symbol', { name: pick(...NAMES) })),
    () => JSON.stringify(call(pick('nope', 'Grep', ''), {})),
    () => JSON.stringify({ jsonrpc: '2.0', id: 9, method: 'tools/call', params: pick({ name: 'list', arguments: 'x' }, { name: 5 }, []) }),
  )()

const cases = []
for (let i = 0; i < Number(N); i += 1) {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'critique-search-corpus-')))
  const root = join(dir, 'packet')
  const files = Object.fromEntries([...new Set(some(() => pick(...FILES), 6))].map((path) => [path, text()]))
  if (rnd() < 0.5) files['tree/REPO-MAP.md'] = MAP
  const links = rnd() < 0.6 ? [['escape', '../outside'], ['escape.md', '../outside/secret.md'], ['tree/link.ts', 'src/a.ts']] : []
  mkdirSync(root, { recursive: true })
  mkdirSync(join(dir, 'outside'), { recursive: true })
  writeFileSync(join(dir, 'outside', 'secret.md'), 'reach proof: OUTSIDE-THE-PACKET\n')
  for (const [path, body] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true })
    writeFileSync(join(root, path), body)
  }
  for (const [path, target] of links) {
    mkdirSync(dirname(join(root, path)), { recursive: true })
    symlinkSync(target, join(root, path))
  }
  const kind = rnd()
  const args =
    kind < 0.06
      ? pick(['--root'], ['--max-matches', '3'], ['--root', '@ROOT@', '--max-matches', '0'], ['--root', '@ROOT@', '--max-bytes', '1.5'], ['--root', '@ROOT@', '--bogus', '1'], [])
      : kind < 0.1
        ? pick(['--root', '@ROOT@/missing'], ['--root', '@ROOT@/notes-file'])
        : ['--root', pick('@ROOT@', '@ROOT@/', '@ROOT@/./'), ...(rnd() < 0.4 ? ['--max-matches', pick('2', '5', ' 3 ', '0x4', '1e1')] : []), ...(rnd() < 0.3 ? ['--max-bytes', pick('40', '200', '1000')] : [])]
  if (args.includes('@ROOT@/notes-file')) writeFileSync(join(root, 'notes-file'), 'x\n')
  const lines = some(request, 8)
  const input = lines.join(pick('\n', '\r\n', '\n\n')) + pick('\n', '', '\r')
  const run = spawnSync(process.execPath, [bundle, 'search-server', ...args.map((arg) => arg.split('@ROOT@').join(root))], { input, encoding: 'utf8' })
  const mask = (out) => out.split(root).join('@ROOT@')
  cases.push({ args, files, links, input, expected: { code: run.status, stdout: mask(run.stdout), stderr: mask(run.stderr) } })
  rmSync(dir, { recursive: true, force: true })
}
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
console.log(`${cases.length} cases written to ${OUT}`)
