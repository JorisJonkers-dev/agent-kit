#!/usr/bin/env node
// Writes the corpus the Rust test `readers_corpus` replays: what the node runner's plan-file, decision-docs,
// decisions, architecture, recovery and tracked-tree readers make of generated input.
//
//   node scripts/crates/critique/tests/gen-readers-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so the functions are reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '300', SEED = '17', OUT = join(here, 'fixtures', 'readers.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-readers-corpus-'))
const names = 'draftOf, planFilePathOrNull, namedPlanPath, namedPaths2 as namedPaths, isDecisionRecord, featureStatement, decisionSummary, buildConfigs, isLayerConfig, renderHistoryLog, citedIssues, sourceSample, sourceLines, isSourcePath, DecisionDocs, readTrackedTree, rankingNote, rankedPaths, rankIndex'
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { ${names} }\n`)
const f = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const some = (make, max) => Array.from({ length: Math.floor(rnd() * (max + 1)) }, make)
const attempt = async (run) => {
  try {
    return { ok: await run() }
  } catch (e) {
    return { error: { code: e.code ?? 'unexpected', message: e.message } }
  }
}

const SEGMENTS = ['docs', 'adr', 'src', 'a', 'b.md', 'CONTEXT.md', 'GLOSSARY.md', 'x_y', 'v1.2', '..', '.', '', 'ünï', '-', 'ctx']
const path = () => some(() => pick(...SEGMENTS), 4).join('/')
const PATHS = () => pick('docs/adr', 'docs/adr/0001.md', 'docs/glossary.md', './docs/x.md', 'docs/adr/', 'src/<context>/docs/adr/', '/abs/path', '../up', 'a/../b', '/domain-modeling', 'CONTEXT.md', 'CONTEXT-MAP.md', 'ctx/CONTEXT.md', 'ctx', 'ctx/docs/adr', path())
const markdown = () =>
  some(
    () =>
      pick(
        () => `\`${PATHS()}\``,
        () => `[link](${PATHS()})`,
        () => `[l](${PATHS()} "t")`,
        () => '```\n`docs/fenced.md`\n```',
        () => '~~~\n[x](hidden.md)\n~~~',
        () => '```\nunterminated `a.md`',
        () => `see ${pick('`', '``', '` `', '`a\nb`')}${PATHS()}${pick('`', '')}`,
        () => pick('text', '# heading', '\r\n', '  `  padded.md  `', '[a](b c)', ']( x.md)', '`-`', '`.`', '`..`'),
      )(),
    7,
  ).join(pick('\n', ' ', '\r\n'))

const cases = []
const add = (kind, input, expected) => cases.push({ kind, input, expected })
const count = Number(N)
for (let i = 0; i < count; i += 1) {
  const kind = pick('paths', 'paths', 'plan', 'named', 'history', 'configs', 'decisions', 'docs', 'docs', 'tree', 'ranking')
  if (kind === 'paths') {
    const text = markdown()
    const record = [...some(PATHS, 2), pick('docs/adr/0001.md', 'adr/x.md', 'ctx/docs/adr/a.md', 'adr/x.txt', 'adr.md', 'x/adr/y/z.md')]
    add(kind, { text, record }, { paths: f.namedPaths(text), record: record.map((p) => f.isDecisionRecord(p)) })
  } else if (kind === 'plan') {
    const repository = { root: pick('/h/repo', '/h/repo/'), commonDir: pick('/h/repo/.git', '/h/repo/../other/.git') }
    const tracker = pick('github', 'local-markdown')
    const draft = pick('a', 'a/b', 'a//b', '.', '..', 'a/../b', '', 'x y', 'ünï/ç', '/a', 'a/')
    const flow = pick(undefined, `draft:${draft}`, 'draft:', 'draft', 'run-1', `x draft:${draft}`)
    add(kind, { repository, tracker, draft, flow }, { path: f.planFilePathOrNull(repository, tracker, draft), draft: f.draftOf(flow) ?? null })
  } else if (kind === 'named') {
    const repository = { root: '/h/repo', commonDir: pick('/h/repo/.git', '/h/common/.git', '/h/repo2/.git') }
    const named = pick('plan.md', './plan.md', '../plan.md', '/h/repo/p.md', '/h/repo/../x/p.md', '/h/common/.git/critique/plans/a/refactoring-plan.md', '/h/repo2', '/h/repo/', '/', '', '.', '..', '/h/repo/a/../../h/repo/q', 'a/../../repo2/x', '/h/repository/x')
    add(kind, { repository, path: named }, await attempt(() => f.namedPlanPath(repository, named)))
  } else if (kind === 'history') {
    const entry = () => ({ commit: pick('abc123', 'def456', 'a'.repeat(40)), date: '2026-11-0' + pick(1, 2, 3), subject: pick('fix #12', 'x (#7)', 'see #12 and #9', 'a#3', 'a/#4', '##5', '#6a', '#7_', '#8-b', '#8 #8', 'plain', '#99999999999999999999999', '#', 'Ünï #5'), body: pick('', '', 'Closes #1.\nrefs #2,#3', 'ünï\n#4', ...[0, 1, 2].map(() => 'x'.repeat(pick(10, 2500)))), files: some(() => pick('src/a.ts', 'b.rs', 'docs/c.md'), 3) })
    const entries = some(entry, pick(2, 30, 90))
    const commit = 'f'.repeat(40)
    const rendered = f.renderHistoryLog(entries, commit)
    add(kind, { entries, commit }, { text: rendered.text, left_out: rendered.left_out, cited: f.citedIssues(entries) })
  } else if (kind === 'configs') {
    const FILES = ['package.json', 'tsconfig.json', 'tsconfig.build.json', 'tsconfig.json5', 'Cargo.toml', 'Makefile', 'makefile', 'justfile', 'go.mod', 'pom.xml', 'build.gradle', 'build.gradle.kts', 'settings.gradle.kts', 'docker-compose.yml', 'docker-compose.yaml', '.dependency-cruiser.js', 'dependency-cruiser.config.cjs', '.dependency-cruiser.json', 'dependency-cruiser.mjs', '.importlinter', 'tach.toml', 'archunit.json', '.arch-unit.yaml', 'arch-unit.properties', '.go-arch-lint.yml', 'deptrac.yaml', 'deptrac.yml', 'README.md', 'src', 'x.ts', 'a.TS', 'tsconfig.a b.json']
    const files = [...new Set(some(() => pick(...FILES), 16))]
    const tracked = some(() => ({ path: pick('a.ts', 'src/b.py', 'docs/c.md', 'x/y/z.rs', 'q.TS', '.hidden', 'Makefile', 'lib/x.nix', 'a/.sh', 'ünï.ts', 'k.Kotlin'), text: pick(null, '', 'a\nb\n', 'one', '\n', '\n\n', 'a\r\nb') }), 6)
    add(kind, { files, tracked }, { configs: f.buildConfigs(files), layers: files.map((name) => f.isLayerConfig(name)), sample: f.sourceSample(tracked), lines: f.sourceLines(tracked), source: tracked.map((file) => f.isSourcePath(file.path)) })
  } else if (kind === 'decisions') {
    const text = pick('x', '  x  ', 'a\nb\nc\nd\ne', 'a\nb\nc\nd\ne\nf', 'a\r\nb\r\nc\r\nd\r\ne\r\nf', 'a\rb\rc\rd\re\rf\rg', '\n\nx\n\n', '', '   ', Array.from({ length: pick(39, 40, 41, 45) }, (_, n) => `l${n}`).join('\n'), ' x ', 'x y')
    add(kind, { text }, { feature: await attempt(() => f.featureStatement(text)), summary: await attempt(() => f.decisionSummary(text)) })
  } else if (kind === 'docs') {
    const files = {}
    const doc = (status) => pick('', '# Title\n', `Status: ${status}\n`, `---\nstatus: ${status}\n---\n`, `# t\n## Status\n\n${status}\n`)
    const put = (p, text) => (files[`/r/${p}`] = text)
    const hasDomain = rnd() < 0.7
    const mentioned = []
    const status = () => pick('accepted', 'proposed', 'superseded', 'rejected', 'deprecated', 'Accepted')
    for (let n = 0; n < 4; n += 1) put(`docs/adr/000${n}-${pick('a', 'b', 'c')}.md`, doc(status()))
    if (rnd() < 0.6) put(pick('GLOSSARY.md', 'CONTEXT.md'), doc(status()))
    if (rnd() < 0.4) put('CONTEXT.md', doc(status()))
    if (rnd() < 0.3) put('docs/notes.txt', 'x')
    if (rnd() < 0.5) {
      put('CONTEXT-MAP.md', `${pick('', '# map\n')}\`ctx/CONTEXT.md\` and [b](other/CONTEXT.md) and \`ctx\`\n${pick('', '```\n`fenced/CONTEXT.md`\n```')}`)
      put('ctx/CONTEXT.md', doc(status()))
      put('ctx/docs/adr/0001-x.md', doc(status()))
      put('other/CONTEXT.md', doc(status()))
      put('other/docs/adr/0002-y.md', doc(status()))
    }
    if (hasDomain) {
      const named = [pick('`docs/adr`', '`docs/adr/0001-a.md`', '`CONTEXT-MAP.md`', '[g](GLOSSARY.md)', '`docs/missing.md`', '`docs/notes.txt`', '`ctx`', '`CONTEXT.md`', '`docs/adr/`', '`./docs/adr/0000-a.md`'), pick('', '`docs/adr`', '`CONTEXT-MAP.md`')]
      put('docs/agents/domain.md', `# domain\n${named.join('\n')}\n`)
    }
    for (const key of Object.keys(files)) if (rnd() < 0.02) delete files[key]
    const listFiles = (dir) => [...new Set(Object.keys(files).filter((p) => p.startsWith(`${dir}/`) && !p.slice(dir.length + 1).includes('/')).map((p) => p.slice(dir.length + 1)))].sort()
    const fake = {
      exists: async (p) => p in files,
      readText: async (p) => {
        if (!(p in files)) throw new Error(`ENOENT: no such file or directory, open '${p}'`)
        return files[p]
      },
      listFiles: async (p) => listFiles(p.replace(/\/$/, '')),
    }
    add(kind, { files }, await attempt(() => new f.DecisionDocs(fake).resolve('/r')))
  } else if (kind === 'tree') {
    const limits = { cap_bytes: 1, exclude: { generated: pick([], ['**/*.gen.ts']), lock: pick([], ['*.lock', '**/package-lock.json']), vendored: pick([], ['vendor/**', 'node_modules/**']) } }
    const tree = some(() => ({ path: pick('a.ts', 'b.lock', 'x/y.gen.ts', 'vendor/z.js', 'package-lock.json', 'docs/c.md', 'bin/blob', 'src/gen.ts', 'src/do-not-edit.ts', 'a/b/c.rs'), text: pick('x', 'a\u0000b', '// @generated\nx', '/* DO NOT EDIT */', 'one\ntwo\nthree\nfour\nfive\n@generated', '', 'ünï') }), 8)
    const unique = [...new Map(tree.map((entry) => [entry.path, entry])).values()]
    const texts = new Map(unique.map((entry) => [entry.path, entry.text]))
    const git = { treeFiles: async () => unique.map((entry) => entry.path), fileAt: async (_root, _commit, p) => texts.get(p) }
    add(kind, { limits, tree: unique }, await attempt(() => f.readTrackedTree({ git }, '/r', 'c0ffee', limits)))
  } else {
    const ranking = pick({ status: 'unavailable', detail: pick('not installed', 'ünï') }, { status: 'ranked', files: some(() => ({ path: pick('a.ts', 'b.ts', 'c.ts', 'a.ts'), refs: 1 }), 5), symbols: [] })
    const ranked = f.rankedPaths(ranking)
    const rank = f.rankIndex(ranked)
    add(kind, { ranking }, { note: f.rankingNote(ranking), ranked, places: ['a.ts', 'b.ts', 'c.ts', 'd.ts'].map((p) => (rank(p) === Infinity ? null : rank(p))) })
  }
}
const canon = (v) => (Array.isArray(v) ? v.map(canon) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])])) : v)
writeFileSync(OUT, JSON.stringify(canon(cases)))
const kinds = {}
for (const c of cases) kinds[c.kind] = (kinds[c.kind] ?? 0) + 1
console.log(`${cases.length} cases`, kinds)
