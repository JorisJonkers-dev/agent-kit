#!/usr/bin/env node
// Writes the corpus the Rust test `reviews_corpus` replays: generated run directories and ledgers, and what the node
// runner's ReviewsCommand answers over them.
//
//   node scripts/crates/critique/tests/gen-reviews-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so the command is reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '300', SEED = '5', OUT = join(here, 'fixtures', 'reviews-read.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-reviews-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { ReviewsCommand }\n`)
const { ReviewsCommand } = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const STAGES = ['decisions', 'spec', 'tickets', 'ticket-review', 'feature-review', 'adr-review', 'decision-recovery', 'architecture-review', 'plan']
const REVIEW = ['adr-review', 'decision-recovery', 'architecture-review']
const KEY = 'github.com/jorisjonkers-dev/example'
const IDS = ['r1', 'r2', 'r9', 'r10', 'r11', 'R3', 'r-3', 'r09', 'x', '20261101-0001', '20261101-0002', 'a10', 'a9', 'A9']
const stage = () => (rnd() < 0.8 ? pick(...REVIEW) : pick(...STAGES))
const repository = () => (rnd() < 0.85 ? KEY : 'github.com/other/repo')
const at = () => `2026-11-0${pick(1, 2, 3)}T${pick('00:00:00.000', '12:30:00.000')}Z`
const G = (good, bad) => (rnd() < 0.97 ? good : bad)
const bad = () => pick(null, 5, true, [], {}, 'zzz', 1.5)
const hashes = { contract: 'c', preamble: 'p', role: 'r', schema: 's' }
const context = (run_id) => ({ recorded_at: at(), repository: repository(), run_id: G(run_id, bad()), stage: G(stage(), 'nope') })
const runLine = () => ({ type: 'run', ...context(pick(...IDS)), client: 'vibe', cost_usd: pick(null, 0.1), critic_id: pick('c1', 'c2'), failure: null, finding_count: 1, lens_hashes: {}, lenses: [], model: 'm', prompt_hashes: hashes, role: 'r', status: 'ok', ...(rnd() < 0.3 ? { round: pick(1, 2) } : {}) })
const findingLine = () => ({ type: 'finding', ...context(pick(...IDS)), finding_id: 'f1', pre_verdict: null, raised_by: ['c1'], severity: 'major', verdict: G(pick('accepted', 'rejected', 'needs-human', 'hint'), 'zzz') })
const invocation = () => ({ type: 'invocation', recorded_at: at(), repository: repository(), flow: 'f', workflow: 'f', round: 1, stage: 'adr-review', kind: pick('vibe', 'vibe', 'agy', 'codex'), charged_usd: pick(0, 0.1, 0.2, 0.3, 1e-7, 2.5), critic_id: 'c1', ...(rnd() < 0.9 ? { run_id: pick(...IDS) } : {}) })
const MUTATIONS = [(t) => t.slice(0, Math.floor(t.length / 2)), (t) => `${t}\r`, (t) => '', (t) => 'not json', (t) => '[1]', (t) => '{"type":"later"}', (t) => t.replace('"type"', '"type "')]
const ledger = (make) => {
  if (rnd() < 0.15) return null
  const n = Math.floor(rnd() * 9)
  const lines = Array.from({ length: n }, () => {
    const text = JSON.stringify(make())
    return rnd() < 0.03 ? pick(...MUTATIONS)(text) : text
  })
  return lines.join('\n') + pick('', '\n')
}
const manifest = (run_id) => {
  const body = {
    allowed_families: ['mistral'], artifact: { labels: [], reference: 'o/r#1', tracker: 'github' }, created_at: '2026-10-30T12:00:00.000Z', critics: [], depth: 'base',
    depth_choice: { chosen_by: 'argument', measures: {}, signals: [] }, distinct_families: true, dropped: [], flow: run_id, limits: { antigravity_quota: { daily_cap: 20, reserve_threshold: 10, reserved_stages: ['spec'] }, search: { max_bytes: 8192, max_matches: 50 } },
    orchestrator_family: 'anthropic', packet_dir: '/p', packet_files: [], repo_root: '/h/repo', repository: KEY, run_dir: '/r', run_id, stage: stage(), version: 1,
  }
  if (rnd() < 0.3) Object.assign(body, { round: 2, previous_run_id: 'r1' })
  const text = JSON.stringify(body)
  return rnd() < 0.04 ? pick(...MUTATIONS)(text) : rnd() < 0.03 ? JSON.stringify({ ...body, stage: 'zzz' }) : text
}
const dirs = () => {
  const names = [...new Set(Array.from({ length: Math.floor(rnd() * 8) }, () => pick(...IDS, 'stray', 'file.txt')))]
  return names.map((name) => ({ name, kind: rnd() < 0.05 ? 'file' : 'dir', manifest: rnd() < 0.85 ? manifest(rnd() < 0.8 ? name : pick(...IDS)) : null, report: rnd() < 0.4 }))
}
const out = []
for (let i = 0; i < Number(N); ) {
  const input = { repo: '/h/repo', ...(rnd() < 0.5 ? { limit: pick(1, 2, 3, 10, 100, 0, 1.5, '2') } : {}) }
  if (rnd() < 0.03) delete input.repo
  const record = { input, own: ledger(() => pick(runLine, runLine, findingLine, findingLine, invocation)()), gateway: ledger(invocation), dirs: dirs(), remotes: pick([['origin', 'git@github.com:JorisJonkers-dev/example.git']], [['origin', 'git@github.com:JorisJonkers-dev/example.git']], [], [['origin', 'git@github.com:nedap/secret.git']]) }
  const files = new Map([['/h/.local/share/agent-kit/critique/ledger.jsonl', record.own], ['/h/state/agent-kit/model-gateway/ledger.jsonl', record.gateway]])
  const kinds = new Map()
  for (const d of record.dirs) {
    kinds.set(`/h/repo/.git/critique/${d.name}`, d.kind)
    if (d.kind === 'dir') {
      files.set(`/h/repo/.git/critique/${d.name}/manifest.json`, d.manifest)
      files.set(`/h/repo/.git/critique/${d.name}/report.md`, d.report ? '# r\n' : null)
    }
  }
  const m = { HOME: '/h', XDG_STATE_HOME: '/h/state' }
  const deps = {
    env: { get: (k) => m[k], require: (k) => { if (m[k] === undefined) throw new Error(`required environment variable is missing: ${k}`); return m[k] } },
    clock: { now: () => new Date('2026-11-03T12:00:00Z') },
    git: { remotes: async () => record.remotes.map(([name, url]) => ({ name, url })), commonDir: async () => '/h/repo/.git', root: async () => '/h/repo' },
    files: {
      exists: async (p) => files.get(p) != null,
      readText: async (p) => files.get(p),
      listDirectories: async () => [...kinds].filter(([, kind]) => kind === 'dir').map(([p]) => p.slice('/h/repo/.git/critique/'.length)).sort(),
    },
    ledger: {},
  }
  try {
    record.expected = { ok: await new ReviewsCommand(deps).run(input) }
  } catch (e) {
    record.expected = { error: { code: e.code ?? 'unexpected', message: e.message } }
  }
  if ((record.expected.error || record.expected.ok.status === 'refused') && rnd() < 0.85) continue
  out.push(record)
  i += 1
}
const canon = (v) => (Array.isArray(v) ? v.map(canon) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])])) : v)
writeFileSync(OUT, JSON.stringify(canon(out)))
const kinds = {}
for (const r of out) { const k = r.expected.error ? `error ${r.expected.error.code}` : r.expected.ok.status ?? 'refused'; kinds[k] = (kinds[k] ?? 0) + 1 }
console.log(`${out.length} cases`, kinds)
