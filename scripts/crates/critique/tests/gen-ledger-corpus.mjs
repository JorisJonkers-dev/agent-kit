#!/usr/bin/env node
// Writes the corpus the Rust test `ledger_corpus` replays: generated ledger texts (critique's own and the
// gateway's), and what the node runner's LedgerSource and FlowBudget read from them.
//
//   node scripts/crates/critique/tests/gen-ledger-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so its read half is reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '150', SEED = '7', OUT = join(here, 'fixtures', 'ledger-read.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-ledger-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { LedgerSource, FlowBudget }\n`)
const { LedgerSource, FlowBudget } = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const STAGES = ['decisions', 'spec', 'tickets', 'ticket-review', 'feature-review', 'adr-review', 'decision-recovery', 'architecture-review', 'plan']
const STATUS = ['ok', 'failed', 'violated-readonly', 'budget-skip', 'skipped-low-risk']
const day = () => pick('2026-11-03', '2026-11-02', '2026-11-04', '2026-11-03')
const at = () => `${day()}T${pick('00:00:00.000', '12:30:00.000', '23:59:59.999')}Z`
const str = () => pick('a', 'b', 'run-1', 'x y', '', ' ', 'é😀', 'acme/w')
const num = () => pick(0, 1, 0.1, 0.2, 0.3, 2.5, -1, 1e21, 1e-7, 123456789.125)
const bad = () => pick(null, 5, true, [], {}, 'x', 1.5)
const G = (good, badv) => (rnd() < .96 ? good : badv)
const hashes = () => ({ contract: 'c', preamble: 'p', role: 'r', schema: 's' })
function maybe(o, k, v, p = 0.97) { if (rnd() < p) o[k] = v; return o }
function inv() {
  const o = { type: 'invocation' }
  maybe(o, 'charged_usd', rnd() < .97 ? num() : bad()); maybe(o, 'critic_id', rnd() < .97 ? pick('c1', 'c2', 'sec') : bad())
  maybe(o, 'flow', rnd() < .97 ? pick('f1', 'f2', 'f1') : bad()); maybe(o, 'kind', rnd() < .98 ? pick('vibe', 'agy', 'codex', 'claude') : pick('x', 5, null))
  maybe(o, 'recorded_at', rnd() < .985 ? at() : bad()); maybe(o, 'repository', rnd() < .97 ? pick('acme/w', 'acme/w', 'o/r') : bad())
  if (rnd() < .6) o.run_id = rnd() < .97 ? pick('r1', 'r2', 'r3') : bad()
  return o
}
function gw() {
  const o = inv(); o.workflow = rnd() < .97 ? pick('f1', 'f2') : bad(); delete o.flow
  maybe(o, 'round', rnd() < .97 ? pick(1, 2, 3) : bad()); maybe(o, 'stage', rnd() < .97 ? pick(...STAGES) : pick('nope', 5))
  if (!o.run_id && rnd() < .8) o.run_id = 'r1'
  if (rnd() < .6) { o.prompt_hashes = rnd() < .97 ? hashes() : pick({}, { contract: 1 }, null, 'x'); maybe(o, 'critic_client', 'vibe'); maybe(o, 'critic_cost_usd', pick(null, 0.1, 'x')); maybe(o, 'critic_model', 'm'); maybe(o, 'status', pick(...STATUS.slice(0, 4), 'weird'))
    if (rnd() < .5) o.finding_count = pick(0, 3, 'x'); if (rnd() < .5) o.lens_hashes = pick({ a: 'b' }, {}, { a: 1 }, []); if (rnd() < .5) o.lenses = pick(['a'], [], [1], 'x'); if (rnd() < .4) o.role = pick('r', '', 5); if (rnd() < .4) o.failure = pick('timeout', 5, null) ; if (rnd() < .3) o.spec = G('s.md', 5) }
  return o
}
function runLine() {
  const o = { type: 'run' }
  maybe(o, 'recorded_at', at()); maybe(o, 'repository', pick('acme/w', 'o/r')); maybe(o, 'run_id', G(pick('r1', 'r2', 'r3'), 5)); maybe(o, 'stage', G(pick(...STAGES), 'zzz'), .9)
  if (rnd() < .5) o.spec = G('s.md', 5); if (rnd() < .6) o.flow = G(pick('f1','f2'), 5); if (rnd() < .5) o.round = G(pick(1,2), 'x')
  if (rnd() < .15) { o.status = 'skipped-low-risk'; o.ticket = G('t1', 5); return o }
  maybe(o, 'client', 'vibe', .985); maybe(o, 'cost_usd', G(pick(null, 0.2), 'x'), .985); maybe(o, 'critic_id', pick('c1', 'c2'), .985); maybe(o, 'failure', G(pick(null, 'timeout'), 5), .985)
  if (rnd() < .6) o.finding_count = G(pick(0, 2), 'x'); maybe(o, 'lens_hashes', G(pick({ a: 'b' }, {}), { a: 1 }), .985); maybe(o, 'lenses', G(pick(['a', 'b'], []), [1]), .985); maybe(o, 'model', 'm', .99)
  maybe(o, 'prompt_hashes', rnd() < .97 ? hashes() : pick({}, null, 5), .99); maybe(o, 'role', G('r', 5), .99); maybe(o, 'status', G(pick(...STATUS.filter(x => x !== 'skipped-low-risk')), 'weird'), .985)
  return o
}
function findingLine() {
  const o = { type: 'finding' }
  maybe(o, 'recorded_at', at()); maybe(o, 'repository', 'acme/w'); maybe(o, 'run_id', pick('r1', 'r2')); maybe(o, 'stage', pick(...STAGES), .99)
  if (rnd() < .5) o.spec = 's.md'; if (rnd() < .5) o.flow = 'f1'; if (rnd() < .5) o.round = pick(1, 2)
  maybe(o, 'finding_id', G(pick('f-1', 'f-2'), 5), .99); maybe(o, 'severity', G(pick('critical', 'major', 'minor'), 'zzz'), .985); maybe(o, 'raised_by', G(pick(['c1'], ['c1', 'c2'], []), [1]), .99)
  maybe(o, 'verdict', G(pick('accepted', 'rejected', 'needs-human', 'hint'), 'zzz'), .985); if (rnd() < .5) o.pre_verdict = pick(null, 'x', 5); if (rnd() < .5) o.answered = G(pick(true, false), 'x')
  return o
}
function flowLine() {
  const o = { type: 'flow' }
  maybe(o, 'flow', 'f1', .99); maybe(o, 'recorded_at', at(), .99); maybe(o, 'repository', 'acme/w', .99)
  maybe(o, 'event', G(pick('grilled', 'no-critique', 'landed', 'answered', 'draft'), 'zzz'), .99)
  if (rnd() < .6) o.value = G(pick(true, false), 'x'); if (rnd() < .6) o.ticket = G('t1', 5); if (rnd() < .5) o.run_id = 'r1'; if (rnd() < .5) o.finding_id = 'f-1'; if (rnd() < .5) o.draft = G('d', 5)
  return o
}
const MUT = [(t) => t, (t) => t, (t) => t, (t) => t.slice(0, Math.floor(t.length / 2)), (t) => ` ${t} `, (t) => `${t}\r`, (t) => '', (t) => '   ', (t) => 'not json', (t) => '[1]', (t) => 'null', (t) => '7', (t) => '"x"', (t) => '{"type":"later","x":1}', (t) => '{}', (t) => t.replace('"type"', '"type "'), (t) => ' ' + t, (t) => t.replace(/:/, ' :'), (t) => t.replace(/"charged_usd":[^,}]*/, '"charged_usd":1e999'), (t) => t.replace(/"round":[^,}]*/, '"round":2.0'), (t) => t.replace('{', '{"type":"x",'), (t) => t.replace(/"recorded_at":"[^"]*"/, '"recorded_at":"\\u00e92026-11-03T10:00Z"')]
function text(kind) {
  const lines = []
  const n = Math.floor(rnd() * 8)
  for (let i = 0; i < n; i++) {
    let o = kind === 'gw' ? (rnd() < .9 ? gw() : pick(runLine(), { type: 'delegation', at: at() })) : pick(inv, inv, runLine, runLine, findingLine, flowLine)()
    let t = JSON.stringify(o)
    if (rnd() < .04) t = pick(...MUT)(t)
    lines.push(t)
  }
  const nl = pick('\n', '\n', '\n', '\r\n')
  const joined = lines.join(nl)
  return rnd() < .1 ? '' : joined + pick('', nl, nl + nl)
}
const out = []
for (let i = 0; i < Number(N);) {
  const own = rnd() < .08 ? null : text('own')
  const gateway = rnd() < .15 ? null : text('gw')
  const files = { '/h/.local/share/agent-kit/critique/ledger.jsonl': own, '/h/state/agent-kit/model-gateway/ledger.jsonl': gateway }
  const m = { HOME: '/h', XDG_STATE_HOME: '/h/state' }
  const env = { get: (k) => m[k], require: (k) => { if (m[k] === undefined) throw new Error(`required environment variable is missing: ${k}`); return m[k] } }
  const deps = { env, clock: { now: () => new Date('2026-11-03T12:00:00Z') }, files: {
    exists: async (p) => files[p] != null, readText: async (p) => files[p] }, ledger: {} }
  const flows = pick(['f1'], ['f1', 'f2'], ['f2', 'f1', 'zz'], [])
  const repository = pick('acme/w', 'o/r', 'none')
  const rec = { own, gateway, flows, repository, now: '2026-11-03T12:00:00.000Z' }
  try {
    const read = await new LedgerSource(deps).read()
    const budget = new FlowBudget(deps)
    rec.expected = { ok: { read, spend: await budget.vibeSpendByFlow(repository, flows), agy: await budget.agyInvocationsToday() } }
  } catch (e) { rec.expected = { error: { code: e.code ?? 'unexpected', message: e.message } } }
  if (rec.expected.error && rnd() < .85) continue
  out.push(rec); i++
}
const canon = (v) => Array.isArray(v) ? v.map(canon) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])])) : v
writeFileSync(OUT, JSON.stringify(canon(out)))
console.log(`${out.length} cases; ${out.filter((r) => r.expected.error).length} errors`)
