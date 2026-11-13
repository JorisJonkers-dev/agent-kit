#!/usr/bin/env node
// Writes the corpus the Rust test `packets_corpus` replays: what the node runner's ADR, decisions, recovery and
// architecture packet readers, and code-nav's map, make of a generated repository (its working tree, its git tree,
// its history, its issues, and what code-nav answers).
//
//   node scripts/crates/critique/tests/gen-packets-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so the classes are reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '300', SEED = '31', OUT = join(here, 'fixtures', 'packets.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-packets-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { sampleTree, sourceSample, AdrPacketReader, DecisionsPacketReader, RecoveryPacketReader, ArchitecturePacketReader, TrackerResolver, CodeNavRepoMap }\n`)
const f = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const int = (lo, hi) => lo + Math.floor(rnd() * (hi - lo + 1))
const some = (make, max) => Array.from({ length: Math.floor(rnd() * (max + 1)) }, make)
const ROOT = '/r'

const status = () => pick('', 'accepted', 'proposed', 'superseded', 'rejected', 'deprecated', 'Accepted')
const record = () => pick('', '# Title\n', `Status: ${status()}\n`, `---\nstatus: ${status()}\n---\n`, `# t\n## Status\n\n${status()}\n`) + pick('', 'We use `src/a.ts` and [x](docs/adr/0001-a.md).\n', 'ünï € text\n') + pick('', 'x'.repeat(int(0, 200)))
const big = () => (rnd() < 0.07 ? pick('€'.repeat(int(2800, 2900)), 'a'.repeat(int(8000, 9000))) : pick('{}', '{}', 'x\n'.repeat(int(1, 20))))

const worktree = () => {
  const files = {}
  for (let n = 0; n < int(0, 4); n += 1) files[`docs/adr/000${n}-${pick('a', 'b')}.md`] = record()
  if (rnd() < 0.2) files['docs/adr/notes.txt'] = 'notes'
  if (rnd() < 0.5) files[pick('GLOSSARY.md', 'CONTEXT.md')] = record()
  if (rnd() < 0.3) files['CONTEXT.md'] = record()
  if (rnd() < 0.3) files['CONTEXT-MAP.md'] = record()
  if (rnd() < 0.6) files[pick('README.md', 'readme.md', 'README')] = big()
  if (rnd() < 0.3) files.README = big()
  for (const name of some(() => pick('package.json', 'tsconfig.json', 'tsconfig.build.json', 'Cargo.toml', 'Makefile', '.dependency-cruiser.js', 'tach.toml', 'go.mod', 'x.txt', 'docker-compose.yml'), 6)) files[name] = big()
  if (rnd() < 0.5) files['docs/agents/domain.md'] = `# domain\n${pick('`docs/adr`', '`CONTEXT-MAP.md`', '`docs/adr/0000-a.md`', '`docs/missing.md`')}\n`
  return files
}
const treeFiles = () => {
  const paths = [...new Set(some(() => `${pick('', 'src/', 'src/deep/', 'lib/', 'docs/')}${pick('index.ts', 'main.rs', 'a.py', 'b.go', 'README.md', 'x.txt', 'util.ts', 'gen.ts', 'blob.bin', 'k.lock')}`, 10))]
  return paths.map((path) => ({ path, text: pick('', 'one\ntwo\n', Array.from({ length: int(1, 60) }, (_, n) => `line ${n} é`).join('\n'), 'a\u0000b', '// @generated\nx', 'import x from "./a"\n') }))
}
const nav = (tree) => {
  const mapLine = (p) => `${pick(`${ROOT}/`, '', './')}${p} (${int(1, 20)} refs, ${int(0, 9)} symbols)`
  const files = tree.filter(() => rnd() < 0.6).map((file) => mapLine(file.path))
  const symbols = tree.filter(() => rnd() < 0.4).map((file) => `${pick(`${ROOT}/`, '')}${file.path}:${int(1, 50)} ${pick('function', 'class')} ${pick('go', 'run it', 'x')} (${int(0, 9)} refs)`)
  const text = [...pick([], ['preamble.ts (1 refs, 1 symbols)']), pick('files:', 'files:'), ...files, ...pick([], ['garbage line', `${ROOT}/../out.ts (1 refs, 1 symbols)`, `/elsewhere/x.ts (2 refs, 1 symbols)`, `${ROOT} (1 refs, 1 symbols)`, `${ROOT}/a (x refs, 1 symbols)`]), 'symbols:', ...symbols, ...pick([], ['nope', '/elsewhere/y.ts:3 function go (1 refs)', `${ROOT}/../up.ts:3 class Up (1 refs)`, `${ROOT}/ok.ts:3 class Ok (1 refs)`])].join('\n')
  const reply = (id, result) => `${JSON.stringify({ jsonrpc: '2.0', id, result })}\n`
  const ok = `${reply(1, { capabilities: {} })}${reply(2, { content: [{ type: 'text', text }] })}`
  return pick(
    { outcome: 'ok', stdout: ok },
    { outcome: 'ok', stdout: ok },
    { outcome: 'ok', stdout: `noise\n[1]\n"x"\n${reply(2, { content: [{ text: 'files:\n' }, { type: 'image' }, { text: tree[0] ? `${ROOT}/${tree[0].path} (3 refs, 1 symbols)` : '' }] })}` },
    { outcome: 'ok', stdout: reply(2, { isError: true, content: [] }) },
    { outcome: 'ok', stdout: reply(2, 'not an object') },
    { outcome: 'ok', stdout: reply(2, [1]) },
    { outcome: 'ok', stdout: reply(2, { content: 'x' }) },
    { outcome: 'ok', stdout: reply(1, {}) },
    { outcome: 'ok', stdout: '' },
    { outcome: 'throws', message: pick('spawn code-nav ENOENT', 'boom') },
    { outcome: 'timeout' },
    { outcome: 'exit', code: int(1, 3), stderr: pick('', '  first line\nsecond', '\n\nlate') },
  )
}
const history = () => some(() => ({ commit: `c${int(0, 99)}`, date: '2026-11-02T10:00:00Z', subject: pick('fix #12', 'x (#7)', 'plain', '#99 and #12', 'ref #3'), body: pick('', 'Closes #4', 'ü'), files: some(() => pick('src/a.ts', 'b.rs'), 2) }), pick(3, 40))
const issues = () => Object.fromEntries(some(() => [String(int(1, 12)), { title: pick('Issue', 'Ünï title'), body: pick('body', '€'.repeat(int(1, 1500)), ''), labels: some(() => pick('type: bug', 'x'), 2) }], 5))
const limits = () => ({ cap_bytes: pick(120, 700, 2000, 9000, 40000, 200000, 400000), facts_cap_bytes: 5000, manifest_cap_bytes: pick(300, 30720), repo_map_cap_bytes: 20480, window_lines: pick(5, 40, 400), exclude: { generated: [], lock: ['*.lock'], vendored: ['vendor/**'] } })
const paths = () => some(() => pick('docs/adr/0000-a.md', 'docs/adr/0001-a.md', 'docs/adr/0001-b.md', 'CONTEXT.md', 'README.md', '../outside.md', 'docs/missing.md', '/r/CONTEXT.md', 'docs/adr/0002-b.md'), 3)

const repoOf = () => {
  const tree = treeFiles()
  return { worktree: worktree(), tree, history: history(), issues: issues(), nav: nav(tree) }
}
const fakes = (model) => {
  const calls = []
  const files = {
    exists: async (p) => p.startsWith(`${ROOT}/`) && p.slice(ROOT.length + 1) in model.worktree,
    readText: async (p) => {
      const name = p.slice(ROOT.length + 1)
      if (!(name in model.worktree)) throw new Error(`ENOENT: no such file or directory, open '${p}'`)
      return model.worktree[name]
    },
    listFiles: async (p) => {
      const dir = p.slice(ROOT.length + 1)
      const prefix = dir === '' ? '' : `${dir}/`
      return Object.keys(model.worktree).filter((name) => name.startsWith(prefix) && !name.slice(prefix.length).includes('/')).map((name) => name.slice(prefix.length)).sort()
    },
  }
  const git = {
    resolveCommit: async (_root, head) => {
      if (head === 'HEAD' || head === 'bad') throw new Error('git rev-parse failed')
      return `${head}-commit`
    },
    treeFiles: async () => model.tree.map((file) => file.path),
    fileAt: async (_root, _commit, p) => model.tree.find((file) => file.path === p).text,
    lastChange: async () => null,
    commitLog: async (_root, commit, paths, limit) => [...model.history, { commit: 'args', date: commit, subject: `${paths.join(',')}|${limit}`, body: '', files: [] }],
  }
  const gh = {
    viewIssue: async ({ number, comments }) => {
      const issue = model.issues[String(number)]
      if (issue === undefined) throw new Error('gh issue view exited 1: not found')
      return { number, url: `https://github.com/o/r/issues/${number}`, comments: [], ...issue, ...(comments ? {} : {}) }
    },
  }
  const process = {
    exec: async (call) => {
      calls.push(call)
      const { nav: outcome } = model
      if (outcome.outcome === 'throws') throw new Error(outcome.message)
      if (outcome.outcome === 'timeout') return { stdout: '', stderr: '', exitCode: 0, timedOut: true }
      if (outcome.outcome === 'exit') return { stdout: '', stderr: outcome.stderr, exitCode: outcome.code }
      return { stdout: outcome.stdout, stderr: '', exitCode: 0 }
    },
  }
  return { calls, files, git, gh, repoMap: new f.CodeNavRepoMap(process) }
}
const attempt = async (run) => {
  try {
    return { ok: await run() }
  } catch (e) {
    return { error: { code: e.code ?? 'unexpected', message: e.message } }
  }
}
const REPOSITORY = { root: ROOT, commonDir: `${ROOT}/.git`, key: 'github.com/o/r' }

const cases = []
for (let i = 0; i < Number(N); i += 1) {
  const kind = pick('adr', 'adr', 'decisions', 'decisions', 'recovery', 'recovery', 'architecture', 'nav', 'sample')
  const model = repoOf()
  const deps = fakes(model)
  const head = pick('main', 'main', 'main', 'main', 'main', 'main', 'bad')
  let request
  let run
  if (kind === 'adr') {
    request = { ...(rnd() < 0.3 ? { adrs: paths() } : {}), ...(rnd() < 0.3 ? { references: paths() } : {}), ...(rnd() < 0.6 ? { code: { head, limits: limits() } } : {}) }
    run = () => new f.AdrPacketReader(deps).read(ROOT, request)
  } else if (kind === 'decisions') {
    request = { feature: pick('a feature', 'a feature', 'a feature', 'two\nlines', undefined, 'x\ny\nz\n1\n2\n3'), summary: pick(undefined, undefined, 'the summary', 'the summary', 'l\n'.repeat(45)), references: pick(undefined, undefined, undefined, paths()) }
    run = () => new f.DecisionsPacketReader(deps.files).read(ROOT, request)
  } else if (kind === 'recovery') {
    request = { head, limits: limits(), history: pick(null, { tracker: true }, { tracker: false }) }
    run = async () => {
      const tracker = request.history?.tracker ? await new f.TrackerResolver(deps).resolve(REPOSITORY) : undefined
      return new f.RecoveryPacketReader(deps).read(ROOT, { head: request.head, limits: request.limits, history: request.history === null ? undefined : { tracker } })
    }
  } else if (kind === 'architecture') {
    request = { head, limits: limits() }
    run = () => new f.ArchitecturePacketReader(deps).read(ROOT, request)
  } else if (kind === 'sample') {
    request = { head, limits: limits(), ranked: rnd() < 0.5, rest: rnd() < 0.7 }
    const documents = [{ kind: 'reference', path: 'references/CONTEXT.md', source: 'CONTEXT.md', text: 'See src/a.ts and index.ts.\n' }]
    run = () => f.sampleTree(deps, ROOT, { artifactText: documents[0].text, files: documents }, { head: request.head, limits: request.limits }, request.rest ? f.sourceSample : undefined, request.ranked)
  } else {
    request = {}
    run = () => deps.repoMap.rank(ROOT)
  }
  const expected = await attempt(run)
  cases.push({ kind, model, request, expected, calls: deps.calls })
}
const canon = (v) => (Array.isArray(v) ? v.map(canon) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])])) : v)
writeFileSync(OUT, JSON.stringify(canon(cases)))
const kinds = {}
for (const c of cases) {
  const k = `${c.kind} ${c.expected.error ? 'error' : (c.expected.ok.status ?? '')}`
  kinds[k] = (kinds[k] ?? 0) + 1
}
console.log(`${cases.length} cases`, kinds)
