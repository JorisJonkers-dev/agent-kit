#!/usr/bin/env node
// Writes the corpus the Rust test `roster_corpus` replays: what the node runner's depth choice and roster resolution
// make of generated routing files, requests and readiness, as the JSON the manifest freezes (so key order counts).
//
//   node scripts/crates/critique/tests/gen-roster-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so the functions are reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const skill = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique')
const [N = '300', SEED = '53', OUT = join(here, 'fixtures', 'roster.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(join(skill, 'critique.mjs'), 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-roster-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { parseRouting, resolveRoster, candidateClients, resolveWorkRoster, workCandidateClients, workGrantClient, chooseDepth, criticIdentity }\n`)
const f = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
const shipped = JSON.parse(readFileSync(join(skill, 'routing.json'), 'utf8'))
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const int = (lo, hi) => lo + Math.floor(rnd() * (hi - lo + 1))
const some = (make, max) => Array.from({ length: Math.floor(rnd() * (max + 1)) }, make)
const attempt = (run) => {
  try {
    return { ok: run() }
  } catch (e) {
    return { error: { code: e.code ?? 'unexpected', message: e.message } }
  }
}
const FAMILIES = ['mistral', 'zhipu', 'deepseek', 'google', 'anthropic', 'openai']
const ORCHESTRATORS = ['anthropic', 'openai', 'mistral', 'zhipu', 'deepseek', 'google']
const STAGES = Object.keys(shipped.stages)
const OPEN = Object.keys(shipped.clients).filter((name) => !['codex', 'claude'].includes(shipped.clients[name].kind))
const WORK = ['codex-work', 'claude-work']
const MEASURES = { 'architecture-review': ['context_maps', 'source_lines'], 'adr-review': ['adrs'], 'decision-recovery': ['context_maps', 'source_lines'], plan: ['steps', 'stored_data_steps'], decisions: ['decisions', 'words'], 'feature-review': [], spec: ['user_stories', 'words'], 'ticket-review': ['changed_lines', 'risk_path_hits'], tickets: ['tickets'] }

const slot = (clients) => ({ client: rnd() < 0.02 ? 'ghost' : pick(...clients), role: pick('spec-redteam', 'adr-critic'), lenses: some(() => pick('fidelity', 'security', 'simplicity'), 3), max_price_usd: pick(0.25, 0.5, 1), time_limit_s: pick(300, 600) })
const routing = (keep) => {
  const r = JSON.parse(JSON.stringify(shipped))
  for (const client of Object.values(r.clients)) {
    if (client.gateway && rnd() < 0.3) delete client.gateway
    if (client.prices && rnd() < 0.2) delete client.prices
  }
  r.fallbacks = Object.fromEntries(some(() => [pick(...OPEN), some(() => pick(...OPEN), 3)], 3))
  for (const [name, stage] of Object.entries(r.stages)) {
    if (rnd() < 0.7) stage.base = some(() => slot(OPEN), 4).concat([slot(OPEN)])
    if (rnd() < 0.5) stage.deep = some(() => slot(OPEN), 3)
    if (rnd() < 0.5) stage.adr = some(() => slot(OPEN), 2)
    if (rnd() < 0.7) stage.work = some(() => slot(WORK), 3)
    stage.distinct_families = rnd() < 0.7
  }
  if (rnd() < 0.08) r.stages[pick(...Object.keys(r.stages))].base.push(slot(WORK))
  for (const name of Object.keys(r.stages)) if (!keep.includes(name)) delete r.stages[name]
  for (const key of ['triage', 'cold_read', 'gate', 'stats']) if (rnd() < 0.8) delete r[key]
  if (rnd() < 0.05) r.fallbacks = { [pick(...OPEN)]: [pick(...WORK)] }
  return JSON.stringify(r)
}
const availability = (names) => Object.fromEntries(names.filter(() => rnd() < 0.35).map((name) => [name, pick({ ready: false, detail: 'not installed', ...{} }, { ready: false, detail: 'quota used', reason: 'quota' }, { ready: true })]))

const text = () =>
  some(
    () =>
      pick(
        '# User Stories', '## User stories ##', '### Other', '1. As a user I want it', '2) Another story', '   3. indented', '- 4. not a story', '**Term**: defined', '**Bold** : spaced', '- a decision', '* another', '+ plus', '10. ten', 'Migration of the schema', 'a MIGRATION_plan', 'straße Straße STRASSE', 'ǅ ǆ Ǆ', 'ſecurity security', 'K kelvin', 'naïve café', 'data migration', 'x', '', '  ', '#   Spaced  Title  ###', '\r',
      ),
    14,
  ).join(pick('\n', '\r\n', '\n\n'))
const cases = []
for (let i = 0; i < Number(N); i += 1) {
  const kind = pick('roster', 'roster', 'depth', 'depth')
  if (kind === 'depth') {
    const stage = pick(...STAGES)
    const known = MEASURES[stage]
    const config = {
      thresholds: Object.fromEntries(known.filter(() => rnd() < 0.7).map((name) => [name, int(0, 12)])),
      risk_terms: some(() => pick('migration', 'Migration plan', 'straße', 'ǆ', 'security', 'k', 'data migration', '  padded term  ', '', 'a.b', 'c+d', 'naïve', 'café', 'x', 'i', 'İ', 'ı'), 3),
      labels: some(() => pick('risk', 'Security', 'big'), 2),
    }
    const evidence = {
      text: text(), labels: some(() => pick('risk', 'security', 'SECURITY', 'small'), 3),
      ...(rnd() < 0.5 ? { change: { changed_lines: int(0, 500), risk_path_hits: int(0, 5) } } : {}),
      ...(rnd() < 0.5 ? { tickets: int(0, 20) } : {}), ...(rnd() < 0.5 ? { adrs: int(0, 9) } : {}), ...(rnd() < 0.5 ? { sourceLines: int(0, 60000) } : {}), ...(rnd() < 0.5 ? { contextMaps: int(0, 4) } : {}), ...(rnd() < 0.5 ? { planSteps: int(0, 30) } : {}), ...(rnd() < 0.5 ? { storedDataSteps: int(0, 5) } : {}),
    }
    const explicit = pick(undefined, undefined, 'base', 'deep')
    const result = f.chooseDepth(stage, config, evidence, explicit)
    cases.push({ kind, input: { stage, config, evidence, ...(explicit === undefined ? {} : { explicit }) }, expected: JSON.stringify(result) })
    continue
  }
  const stage = pick(...STAGES)
  const text_ = routing(rnd() < 0.9 ? [stage, pick(...STAGES)] : [pick(...STAGES)])
  const parsed = attempt(() => f.parseRouting(text_))
  const request = { stage, depth: pick('base', 'deep'), allowedFamilies: FAMILIES.filter(() => rnd() < 0.7), orchestratorFamily: pick(...ORCHESTRATORS), ...(rnd() < 0.5 ? { adr: true } : {}) }
  const readiness = availability([...OPEN, ...WORK, 'unknown'])
  const grant = pick({ live: true, workflow: 'wf-1' }, { live: false, detail: 'no live grant covers workflow wf for codex' })
  const work = { stage, orchestratorFamily: request.orchestratorFamily, grant }
  const map = new Map(Object.entries(readiness))
  const names = (list) => list.map(([name]) => name)
  const expected = parsed.error
    ? parsed
    : {
        ok: {
          candidates: attempt(() => names(f.candidateClients(parsed.ok, request))),
          roster: attempt(() => JSON.stringify(f.resolveRoster(parsed.ok, request, map))),
          identities: attempt(() => f.resolveRoster(parsed.ok, request, map).critics.map(f.criticIdentity)),
          workCandidates: attempt(() => names(f.workCandidateClients(parsed.ok, work))),
          workGrant: attempt(() => f.workGrantClient(parsed.ok, stage)),
          workRoster: attempt(() => JSON.stringify(f.resolveWorkRoster(parsed.ok, work, map))),
          workIdentities: attempt(() => f.resolveWorkRoster(parsed.ok, work, map).critics.map(f.criticIdentity)),
          limits: JSON.stringify(parsed.ok.limits),
        },
      }
  cases.push({ kind: 'roster', input: { routing: text_, request, readiness, grant }, expected })
}
writeFileSync(OUT, JSON.stringify(cases))
const kinds = {}
for (const c of cases) kinds[`${c.kind}${c.expected?.error ? ' error' : ''}`] = (kinds[`${c.kind}${c.expected?.error ? ' error' : ''}`] ?? 0) + 1
console.log(`${cases.length} cases`, kinds)
