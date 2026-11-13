#!/usr/bin/env node
// Writes the corpus the Rust test `stats_corpus` replays: generated ledgers, thresholds and filters, and what the node
// runner's StatsCommand answers over them.
//
//   node scripts/crates/critique/tests/gen-stats-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so the command is reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const shipped = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique')
const [N = '300', SEED = '9', OUT = join(here, 'fixtures', 'stats-read.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(join(shipped, 'critique.mjs'), 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-stats-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { StatsCommand }\n`)
const { StatsCommand } = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
const routing = readFileSync(join(shipped, 'routing.json'), 'utf8')
const CLIENTS = ['vibe-mistral-large', 'vibe-mistral-large', 'agy-gemini-flash-high', 'vibe-deepseek-flash', 'vibe-glm', 'ghost', 'ghost', 'claude-work']
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const STAGES = ['decisions', 'spec', 'tickets', 'ticket-review', 'feature-review', 'adr-review', 'decision-recovery', 'architecture-review', 'plan']
const KEY = 'github.com/jorisjonkers-dev/example'
const day = () => pick('2026-10-30', '2026-11-01', '2026-11-02', '2026-11-03', '2026-11-03')
const at = () => `${day()}T${pick('00:00:00.000', '09:15:00.000', '12:30:00.000', '23:59:59.999')}Z`
const G = (good, bad) => (rnd() < 0.985 ? good : bad)
const runs = Array.from({ length: Math.floor(rnd() * 26) }, (_, i) => ({
  type: 'run', recorded_at: at(), repository: pick(KEY, KEY, 'github.com/Other/Repo'), run_id: `r${1 + (i % 7)}`, stage: rnd() < 0.7 ? pick('spec', 'spec', 'tickets', 'adr-review') : pick(...STAGES),
  ...(rnd() < 0.7 ? { spec: pick('s1', 's2', 's3', 's4') } : {}), client: pick(...CLIENTS), cost_usd: pick(null, 0.1, 0.2, 0.3, 0.125, 0),
  critic_id: pick('c1', 'c2', 'c3'), failure: pick(null, null, 'timeout', 'print-timeout', 'stopped-early', 'exit-124', 'other'), finding_count: pick(0, 1, 2),
  lens_hashes: pick({}, { a: 'h1' }, { b: 'h2', a: 'h1' }, { 10: 'x', 9: 'y', Z: 'z', z: 'w' }), lenses: pick([], ['a']), model: 'm', role: 'r',
  prompt_hashes: { contract: 'c', preamble: pick('p1', 'p2'), role: 'r', schema: 's' }, status: pick('ok', 'ok', 'failed', 'violated-readonly', 'budget-skip'),
}))
const critics = (id) => [...new Set(runs.filter((r) => r.run_id === id).map((r) => r.critic_id))]
const raisers = (run) => { const all = critics(run.run_id); return rnd() < 0.6 ? [run.critic_id] : all.filter(() => rnd() < 0.6) }
const findings = () => {
  if (runs.length === 0) return []
  return Array.from({ length: Math.floor(rnd() * 70) }, (_, i) => {
    const run = pick(...runs)
    return {
      type: 'finding', recorded_at: rnd() < 0.8 ? run.recorded_at : at(), repository: run.repository, run_id: G(run.run_id, 'r99'), stage: run.stage, ...(run.spec ? { spec: run.spec } : {}),
      finding_id: `f${i}`, pre_verdict: pick(null, null, 'fabricated-evidence'), raised_by: G(raisers(run), ['nobody']),
      severity: pick('critical', 'major', 'minor'), verdict: pick('accepted', 'accepted', 'rejected', 'rejected', 'hint', 'needs-human'),
    }
  })
}
const filter = () => ({
  ...(rnd() < 0.2 ? { since: pick('2026-11-01', '2026-11-03', '2026-10-30') } : {}),
  ...(rnd() < 0.2 ? { until: pick('2026-11-02', '2026-11-03', '2026-10-31') } : {}),
  ...(rnd() < 0.15 ? { repository: pick('github.com/jorisjonkers-dev/example', 'GITHUB.com/Other/repo') } : {}),
  ...(rnd() < 0.1 ? { repo: '/h/repo' } : {}),
})
const lines = (objects) => objects.map((o) => JSON.stringify(o)).join('\n') + pick('', '\n')
const out = []
for (let i = 0; i < Number(N); ) {
  const input = filter()
  if (rnd() < 0.04) Object.assign(input, pick({ since: '2026-02-30' }, { until: 'x' }, { since: '2026-12-01', until: '2026-01-01' }, { repo: '/h/repo', repository: 'x' }, { since: '2024-02-29' }, { until: '0000-02-29' }, { since: '1900-02-29' }, { since: 5 }))
  const own = rnd() < 0.1 ? null : lines([...runs, ...findings()])
  const override = pick(null, { stats: { min_adjudicated_findings: 1, removal_features: 1 } }, { stats: { min_adjudicated_findings: 2, removal_features: 2 } }, { stats: { min_adjudicated_findings: 3, removal_features: 1 } }, { stats: null })
  const record = { input, own, override: override === null ? null : JSON.stringify(override), remotes: pick([['origin', 'git@github.com:JorisJonkers-dev/example.git']], [['origin', 'git@github.com:JorisJonkers-dev/example.git']], []) }
  const files = new Map([
    ['/h/.local/share/agent-kit/critique/ledger.jsonl', record.own], ['/assets/routing.json', routing], ['/h/.config/agent-kit/critique-routing.json', record.override],
  ])
  const m = { HOME: '/h', XDG_STATE_HOME: '/h/state' }
  const deps = {
    env: { get: (k) => m[k], require: (k) => { if (m[k] === undefined) throw new Error(`required environment variable is missing: ${k}`); return m[k] } },
    clock: { now: () => new Date('2026-11-03T12:00:00Z') }, assetsRoot: '/assets',
    git: { remotes: async () => record.remotes.map(([name, url]) => ({ name, url })), commonDir: async () => '/h/repo/.git', root: async () => '/h/repo' },
    files: { exists: async (p) => files.get(p) != null, readText: async (p) => files.get(p) },
    ledger: {},
  }
  try {
    record.expected = { ok: await new StatsCommand(deps).run(input) }
  } catch (e) {
    record.expected = { error: { code: e.code ?? 'unexpected', message: e.message } }
  }
  if ((record.expected.error || record.expected.ok.status === 'refused') && rnd() < 0.7) continue
  out.push(record)
  i += 1
}
const canon = (v) => (Array.isArray(v) ? v.map(canon) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])])) : v)
writeFileSync(OUT, JSON.stringify(canon(out)))
const kinds = {}
for (const r of out) { const k = r.expected.error ? `error ${r.expected.error.code}` : r.expected.ok.status; kinds[k] = (kinds[k] ?? 0) + 1 }
console.log(`${out.length} cases`, kinds, 'with cells:', out.filter((r) => r.expected.ok?.cells?.length).length, 'with suggestions:', out.filter((r) => r.expected.ok?.suggestions?.length).length, 'with removal:', out.filter((r) => r.expected.ok?.removal_candidates?.length).length, 'with calibration', out.filter((r) => r.expected.ok?.cells?.some((c) => c.calibration)).length)
