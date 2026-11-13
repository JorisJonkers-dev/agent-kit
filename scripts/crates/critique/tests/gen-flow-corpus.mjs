#!/usr/bin/env node
// Writes the corpus the Rust test `flow_corpus` replays: generated flow sources, and what the node runner's
// `domain/flow.ts` computes from them (the flow state, its next step, the state read back and its file name).
//
//   node scripts/crates/critique/tests/gen-flow-corpus.mjs [count] [seed] [out.json]
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '500', SEED = '29', OUT = join(here, 'fixtures', 'flow-state.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-flow-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { flowState, nextStep, parseFlowState, flowStateFileName }\n`)
const { flowState, nextStep, parseFlowState, flowStateFileName } = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const some = (make, max) => Array.from({ length: Math.floor(rnd() * (max + 1)) }, make)

const STAGES = ['decisions', 'plan', 'spec', 'tickets', 'feature-review', 'ticket-review', 'adr-review']
const RUN_IDS = ['20261101T120000Z-spec', '20261101T120000Z-spec-2', '20261101T120000Z-spec-10', '20261102T090000Z-tickets', '20261103T000000Z-feature-review', 'r1', 'r2', 'R10', 'r9']
const TICKETS = ['#1', '#2', '#3', '#4', '01', '02']
const ref = (id) => (id.startsWith('#') ? `github.com/o/r${id}` : `.scratch/f/issues/${id}-x.md`)
const ticket = () => {
  const id = pick(...TICKETS)
  return { blockedBy: some(() => pick(...TICKETS, '#9'), 2), id, open: rnd() < 0.7, reference: ref(id), title: `T ${id}` }
}
const sources = () => {
  const spec = pick(null, 'github.com/o/r#100', 'github.com/o/r#100')
  const runs = some(() => ({ recorded: rnd() < 0.75, round: pick(1, 1, 2), run_id: pick(...RUN_IDS), stage: pick(...STAGES), ticket_check: pick(null, null, 'passed', 'failed') }), 6)
  const tickets = some(ticket, 4)
  if (rnd() < 0.5) runs.push({ recorded: true, round: 1, run_id: 'z-spec', stage: 'spec', ticket_check: null }, { recorded: rnd() < 0.9, round: 1, run_id: 'z-tickets', stage: 'tickets', ticket_check: pick('passed', null, 'failed') })
  if (rnd() < 0.3) runs.push({ recorded: rnd() < 0.8, round: pick(1, 2), run_id: 'z-fr', stage: 'feature-review', ticket_check: null })
  return {
    draft: pick(null, 'my-draft'),
    events: some(() => pick({ event: 'grilled' }, { event: 'no-critique', value: rnd() < 0.5 }, { event: 'landed', ticket: ref(pick(...TICKETS)) }, { event: 'answered', finding_id: pick('F1', 'F2'), run_id: pick(...RUN_IDS) }, { draft: 'x', event: 'draft' }), 3),
    findings: some(() => ({ answered: rnd() < 0.2, finding_id: pick('F1', 'F2', 'F3', 'F4', 'F5'), run_id: runs.length > 0 && rnd() < 0.8 ? pick(...runs).run_id : pick(...RUN_IDS), severity: pick('critical', 'critical', 'major', 'minor'), stage: pick(...STAGES), verdict: pick('accepted', 'accepted', 'rejected', 'needs-human', 'hint') }), 8),
    flow: spec ?? 'draft:my-draft',
    gates: some(() => ({ gate: pick('pass', 'fail'), head_commit: pick('abc', 'def'), line: pick('GATE: PASS', 'GATE: FAIL (1 critical open)'), run_id: pick(...RUN_IDS) }), 2),
    head_commit: pick('abc', 'def'),
    now: '2026-11-03T12:00:00.000Z',
    plan: pick({ exists: false, path: null }, { exists: false, path: '.scratch/my-draft/plan.md' }, { exists: true, path: '.scratch/my-draft/plan.md' }),
    repository: 'github.com/o/r',
    reviews: some(() => ({ degraded: rnd() < 0.2, recorded_at: pick('2026-11-01T00:00:00.000Z', '2026-11-02T00:00:00.000Z'), review: pick('pass', 'fail', 'degraded'), run_id: pick(...RUN_IDS), ticket: ref(pick(...TICKETS)) }), 3),
    runs,
    skipped_reviews: some(() => ({ recorded_at: pick('2026-11-01T00:00:00.000Z', '2026-11-02T00:00:00.000Z'), run_id: pick(...RUN_IDS), ticket: ref(pick(...TICKETS)) }), 2),
    spec,
    tickets,
    vibe_spend_usd: pick(0, 0.5, 1.25),
  }
}
const FLOWS = ['github.com/o/r#100', 'draft:my-draft', 'draft:Ünï Cödé', '---', 'draft:' + 'x'.repeat(80), 'A/B c_d.e', '']
const MUTATE = [(t) => t, (t) => t.replace('"phase": "', '"phase": "x'), (t) => t.replace('"version": 1', '"version": 2'), (t) => t.slice(0, 40), (t) => t.replace('"decisions":', '"decision":'), (t) => t.replace('"severity": "', '"severity": "z'), (t) => t.replace('"review_run_ids": [', '"review_run_ids": [5, ')]
const cases = []
for (let i = 0; i < Number(N); i += 1) {
  const input = sources()
  if (i % 7 === 0) {
    // Everything landed, the feature reviewed, and a gate for the current head.
    input.spec = 'github.com/o/r#100'
    input.flow = input.spec
    input.events = input.events.filter((event) => event.event !== 'no-critique')
    input.tickets = input.tickets.length === 0 ? [{ blockedBy: [], id: '#1', open: false, reference: 'github.com/o/r#1', title: 'T' }] : input.tickets.map((ticket) => ({ ...ticket, open: false }))
    input.runs = [{ recorded: true, round: 1, run_id: 'z-spec', stage: 'spec', ticket_check: null }, { recorded: true, round: 1, run_id: 'z-tickets', stage: 'tickets', ticket_check: 'passed' }, { recorded: true, round: 2, run_id: 'z-fr', stage: 'feature-review', ticket_check: null }]
    input.findings = input.findings.filter((finding) => finding.verdict !== 'needs-human')
    input.gates = [{ gate: 'pass', head_commit: input.head_commit, line: 'GATE: PASS', run_id: 'g' }]
  }
  if (i % 11 === 0) {
    input.runs.push({ recorded: true, round: 1, run_id: 'zz-spec', stage: 'spec', ticket_check: null })
    for (const id of ['C1', 'C2', 'C3', 'C4']) input.findings.push({ answered: false, finding_id: id, run_id: 'zz-spec', severity: 'critical', stage: 'spec', verdict: 'accepted' })
  }
  const state = flowState(input)
  const text = pick(...MUTATE)(JSON.stringify(state, null, 2))
  let parsed
  try {
    const read = parseFlowState(text)
    parsed = { ok: read, next: nextStep(read) }
  } catch (error) {
    parsed = { error: error.message }
  }
  const flow = pick(...FLOWS)
  cases.push({ sources: input, text, flow, expected: { state, next: nextStep(state), parsed, file: flowStateFileName(flow) } })
}
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
const kinds = {}
for (const c of cases) kinds[c.expected.next.kind + ':' + (c.expected.next.stage ?? c.expected.next.skill ?? '')] = (kinds[c.expected.next.kind + ':' + (c.expected.next.stage ?? c.expected.next.skill ?? '')] ?? 0) + 1
console.log(`${cases.length} cases written to ${OUT}`, JSON.stringify(kinds))
