#!/usr/bin/env node
// Writes the corpus the Rust test `record_corpus` replays: generated runs (manifest, fan-out result, verdicts, refute
// round and gate) and what the node runner adjudicates, reports and writes to the ledger for each.
//
//   node scripts/crates/critique/tests/gen-record-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so its pieces are reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '400', SEED = '11', OUT = join(here, 'fixtures', 'record-corpus.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-record-corpus-'))
const names = [
  'JsonFields', 'JsonShapeError', 'CritiqueError', 'parseManifest', 'parseFanOutResult', 'parseRefuteFile', 'parseGateRecord', 'readVerdict',
  'stageVerdictError', 'verdictProblem', 'severityOverrideProblem', 'verdictsByFinding', 'adjudicate', 'applyRefutes', 'candidateQuestions',
  'countVerdicts', 'secondRoundDue', 'renderReport', 'renderSummary', 'ledgerLines', 'specReference',
]
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { ${names.join(', ')} }\n`)
const node = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
const { JsonFields, JsonShapeError, CritiqueError } = node

let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const int = (lo, hi) => lo + Math.floor(rnd() * (hi - lo + 1))
const chance = (p) => rnd() < p
const STAGES = ['decisions', 'spec', 'tickets', 'ticket-review', 'feature-review', 'adr-review', 'decision-recovery', 'architecture-review', 'plan']
const WEIGHTED = [...STAGES, 'architecture-review', 'architecture-review', 'decision-recovery', 'decision-recovery', 'feature-review', 'ticket-review', 'tickets', 'spec']
const CATEGORIES = ['unasked-question', 'contradiction', 'omission', 'invention', 'untestable', 'failure-mode', 'security', 'structure', 'bug', 'integration', 'drift', 'stale', 'undocumented-decision']
const FAMILIES = ['mistral', 'zhipu', 'deepseek', 'google', 'anthropic', 'openai']
const SEVERITIES = ['critical', 'major', 'minor']
const MECH = ['fabricated-evidence', 'no-code-trace', 'single-site', 'invented-rationale']
const HINTS = ['no-evidence', 'no-spec-quote', 'needs-tree', 'no-check', 'unknown-check', 'one-file-contradiction', 'no-trace']
const WORDS = ['the', 'spec', 'says', 'retry', 'budget', 'is', 'unbounded', 'ticket', 'cycle', 'é', '→', 'naïve', 'ünï', '“quoted”', 'cache', 'index', 'gate', 'owner', 'record', 'stale', '`code`', '<b>', '*x*', 'CRLF\r\n', 'tab\t']
const text = (min = 1, max = 12) => Array.from({ length: int(min, max) }, () => pick(...WORDS)).join(' ')
const longText = () => (chance(0.2) ? Array.from({ length: int(25, 60) }, () => pick(...WORDS)).join(' ') : text())
const slug = () => pick('rule-holds', 'test-fails', 'cross-ticket-trace', 'trace-walked', 'out-of-scope', 'fabricated-evidence', 'dup', 'skipped-by-owner', 'a1-b2')
const hash = () => Array.from({ length: 8 }, () => pick(...'0123456789abcdef')).join('')
const hashes = () => ({ contract: hash(), preamble: hash(), role: hash(), schema: hash() })

const criticRun = (id, family, retryOf) => {
  const status = pick('ok', 'ok', 'ok', 'failed', 'violated-readonly', 'budget-skip')
  return {
    charged_usd: pick(0, 0.0123, 0.5, 1e-7), client: pick('vibe-small', 'agy-pro', 'codex-x'), cost_usd: pick(null, 0.00005, 0.03125, 0.1234567, 12.5, 0), critic_id: id,
    duration_ms: pick(0, 1234, 600000.5), exit_code: pick(null, 0, 1, 124), family, finding_count: pick(0, 1, 2, 5), lens_hashes: chance(0.5) ? {} : { 'lens-a': hash(), b: hash(), 2: hash(), 1: hash() }, lenses: pick([], ['lens-a'], ['lens-a', 'b']),
    model: pick('m-small', 'm-large'), prompt_hashes: hashes(), prompt_path: `/r/${id}/prompt.md`, role: pick('spec-reviewer', 'adr-reviewer', 'bug-hunter'), status,
    ...(status !== 'ok' && chance(0.7) ? { failure: pick('timeout', 'exit-1', 'exit-137', 'empty-output', 'unparseable-findings', 'infrastructure-failure') } : {}),
    ...(chance(0.3) ? { detail: text() } : {}), ...(chance(0.25) ? { no_findings_reason: longText() } : {}), ...(retryOf ? { retry_of: retryOf } : {}),
    ...(chance(0.5) ? { stdout_path: `/r/${id}/out` } : {}), ...(chance(0.3) ? { export_path: `/r/${id}/export.json` } : {}),
    ...(chance(0.4) ? { vibe_cost_usd: pick(null, 0.01) } : {}), ...(chance(0.3) ? { tokens: { cached: 1, input: 2, output: 3 } } : {}),
    ...(chance(0.5) ? { tools: pick(['read_file'], ['read_file', 'grep']) } : {}), ...(chance(0.15) ? { search_failure: text() } : {}),
  }
}

const finding = (n, critics, stage, count) => {
  const raisers = [...new Set(Array.from({ length: pick(1, 1, 2, 3) }, () => pick(...critics.map((c) => c.critic_id), 'ghost')))]
  const declared = chance(0.15) ? pick(...SEVERITIES) : undefined
  const pre = count <= 30 && chance(stage === 'architecture-review' || stage === 'decision-recovery' ? 0.04 : 0.3) ? (chance(0.5) ? { verdict: 'rejected', reason_code: pick(...MECH), reason: text() } : { verdict: 'hint', reason_code: pick(...HINTS), reason: text() }) : undefined
  return {
    id: pick(`f${n}`, `F${n}`, `${n}`), critic_finding_id: `c${n}`, raised_by: raisers,
    severity: declared ?? pick('critical', 'critical', 'major', 'minor'), category: stage === 'architecture-review' && chance(0.4) ? 'undocumented-decision' : pick(...CATEGORIES), location: pick('spec#L1', 'src/a.rs:10', 'ticket 03', 'docs/adr/0001.md'),
    claim: count > 30 ? `${text(15, 25)}${text(15, 25)}` : longText(), evidence: Array.from({ length: int(0, 3) }, () => ({ kind: pick('quote', 'input', 'failing-test', 'trace'), value: text(), ...(chance(0.5) ? { file: pick('spec.md', 'tree/a.rs') } : {}) })),
    ...(chance(0.5) ? { suggestion: text() } : {}), ...(chance(0.4) ? { check: pick('some-check', 'refute', 'not a check', 'a--b') } : {}),
    ...(declared ? { declared_severity: declared } : {}), ...(pre ? { pre_verdict: pre } : {}),
  }
}

const measures = () => pick({}, { sections: { threshold: 5, value: 3 } }, { sections: { threshold: 5, value: 9 }, '3': { threshold: 1, value: 2 }, words: { threshold: 1.5, value: 0.25 } })
const signals = () => pick([], [{ kind: 'threshold', measure: 'sections', threshold: 5, value: 9 }], [{ kind: 'risk-term', term: 'auth' }, { kind: 'label', label: 'risky' }])

const manifest = (id, stage, round, tickets) => {
  const body = {
    allowed_families: ['mistral'], artifact: { labels: [], reference: pick('o/r#1', 'docs/spec.md'), tracker: 'github' }, created_at: '2026-10-30T12:00:00.000Z', critics: [], depth: pick('base', 'deep'),
    depth_choice: { chosen_by: pick('argument', 'signals'), measures: measures(), signals: signals() }, distinct_families: true,
    dropped: Array.from({ length: int(0, 2) }, () => ({ client: 'agy-pro', family: pick('google', 'zhipu'), reason: pick('quota', 'unavailable', 'same-family'), ...(chance(0.4) ? { detail: text() } : {}), ...(chance(0.4) ? { replaced_by: 'c9' } : {}), ...(chance(0.3) ? { deep: true } : {}) })),
    flow: 'flow-1', limits: { antigravity_quota: { daily_cap: 20, reserve_threshold: 10, reserved_stages: ['spec'] }, search: { max_bytes: 8192, max_matches: 50 } },
    orchestrator_family: 'anthropic', packet_dir: '/r/packet', packet_files: tickets.map((path) => ({ kind: 'ticket', path, source: 'o/r#9', sha256: hash() })), repo_root: '/h/repo', repository: 'github.com/jorisjonkers-dev/example', run_dir: '/r', run_id: id, stage, version: 1,
    ...(round === 2 ? { round: 2, previous_run_id: 'r1' } : {}), ...(chance(0.3) ? { plan_warnings: [text()] } : {}),
  }
  if (stage === 'tickets' && chance(0.8)) {
    body.ticket_check = {
      status: pick('passed', 'failed'), ticket_ids: pick([], ['#1'], ['#1', '#2', '#3']), cycles: pick([], [['#1', '#2']], [['#1'], ['#2', '#3', '#4']], [[]]),
      dangling_blockers: Array.from({ length: pick(0, 0, 1, 7) }, (_, i) => ({ ticket: `#${i}`, blocker: `#9${i}` })), uncited_stories: pick(null, [], [4], [1, 2]), ...(chance(0.5) ? { coverage_reason: text() } : {}),
    }
  }
  return body
}

const gateText = (id, findings) => {
  const followUps = findings.filter(() => chance(0.5)).map((f) => ({ finding_id: f.id, open: chance(0.5), reference: `o/r#${int(1, 9)}`, severity: f.severity, ticket: `#${int(1, 9)}` }))
  const open = Array.from({ length: pick(0, 0, 1, 2) }, () => ({ finding_id: 'f1', reference: 'o/r#4', run_id: pick(id, 'r7'), ticket: '#4' }))
  return JSON.stringify({ base_commit: 'aaa', critical_open: open.length, follow_ups: followUps, gate: open.length === 0 ? 'pass' : 'fail', head_commit: 'bbb', line: open.length === 0 ? 'GATE: PASS' : `GATE: FAIL (${open.length} critical)`, open_critical: open, recorded_at: '2026-11-03T12:00:00.000Z', repository: 'k', run_id: id, spec: 'o/r#1', stage: 'feature-review', version: 1 })
}

/** A verdict for the finding that mostly fits its stage; the node runner decides whether it does. */
const verdictOf = (stage, f, tickets, safe) => {
  const recovery = stage === 'architecture-review' || stage === 'decision-recovery'
  const kind = recovery && chance(0.7) ? 'needs-human' : pick('accepted', 'accepted', 'rejected', 'needs-human', 'hint')
  const v = { finding_id: f.id, verdict: kind }
  if (kind === 'rejected' || chance(0.4)) v.reason = text()
  if (chance(0.5)) v.reason_code = slug()
  if (stage === 'feature-review' && kind === 'accepted') {
    v.reason_code = pick('test-fails', 'cross-ticket-trace')
    v.follow_up = '#' + int(1, 9)
    if (v.reason_code === 'cross-ticket-trace') v.trace_tickets = tickets.slice(0, 2)
  }
  if (stage === 'ticket-review' && kind === 'accepted') {
    v.reason_code = pick('test-fails', 'trace-walked')
    if (v.reason_code === 'test-fails') v.regression_test = 'tests/a.rs'
    if (chance(0.5)) v.fixed = true
  }
  if (kind === 'needs-human' && chance(recovery ? 0.02 : 0.5)) v.answered = true
  if (stage === 'decisions' && kind === 'accepted' && chance(0.5)) v.answered = true
  if (f.declared_severity && !safe && chance(0.5)) {
    v.severity = pick(...SEVERITIES)
    v.severity_reason = 'check-wrong'
  }
  if (!safe && chance(0.01)) v.severity_reason = 'x'
  return v
}

const refuteText = (id, findings, critics) => {
  const results = findings.filter((f) => f.severity === 'critical' && chance(0.7)).map((f) => {
    const outcome = pick('no-objection', 'refute', 'downgrade', 'stands', 'skipped')
    return {
      finding_id: chance(0.02) && findings.length < 30 ? 'nope' : f.id, outcome,
      ...(outcome === 'stands' && chance(0.6) ? { answer: pick('refute', 'downgrade') } : outcome === 'refute' || outcome === 'downgrade' ? { answer: outcome } : {}),
      ...(outcome === 'stands' ? { reason_code: pick('invalid-answer', 'no-review', ...HINTS) } : outcome === 'skipped' ? { reason_code: 'no-eligible-client' } : {}),
      ...(chance(0.5) ? { detail: text() } : {}), ...(outcome === 'downgrade' ? { severity: pick('major', 'minor') } : {}),
      ...(outcome !== 'skipped' && chance(0.8) ? { reply: { category: 'bug', claim: text(), evidence: [], location: 'x', severity: 'major' } } : {}),
      ...(outcome !== 'skipped' && chance(0.8) ? { refuter: criticRun(`r-${f.id}`, pick(...FAMILIES)) } : {}),
    }
  })
  return JSON.stringify({ counts: {}, refute_path: '/r/refute.json', results, run_id: id, stage: 'spec', status: 'checked', ...(chance(0.3) ? { ledger_warnings: ['disk full'] } : {}) })
}

const MUT = [(t) => t.slice(0, Math.floor(t.length / 2)), () => '', () => '[1]', () => 'not json', (t) => t.replace('"status"', '"state"')]
const canon = (v) => (Array.isArray(v) ? v.map(canon) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])])) : v)

const decode = (code, where, read) => {
  try {
    return read()
  } catch (error) {
    throw error instanceof JsonShapeError || error instanceof SyntaxError ? new CritiqueError(code, `${where}: ${error.message}`) : error
  }
}

/** What record computes from the files and the verdicts before it writes anything, as the node runner's RecordCommand does. */
const adjudicateRun = (c) => {
  const input = decode('invalid-input', 'record input', () => JsonFields.of({ verdicts: c.verdicts }, '').list('verdicts', node.readVerdict))
  const m = decode('invalid-run', '/r/manifest.json', () => node.parseManifest(c.manifest))
  const merged = decode('invalid-run', '/r/findings.json', () => node.parseFanOutResult(c.findings))
  for (const verdict of input) {
    const error = node.stageVerdictError(m.stage, verdict, merged.findings.find((f) => f.id === verdict.finding_id)?.category) ?? node.verdictProblem(m.stage, verdict, m.packet_files)
    if (error !== undefined) throw new CritiqueError('invalid-input', error)
  }
  const verdicts = node.verdictsByFinding(input, new Set(merged.findings.map((f) => f.id)))
  for (const f of merged.findings) {
    const given = verdicts.get(f.id)
    const error = given === undefined ? undefined : node.severityOverrideProblem(f, given)
    if (error !== undefined) throw new CritiqueError('invalid-input', error)
  }
  const refuted = c.refute === null ? undefined : decode('invalid-run', '/r/refute.json', () => node.parseRefuteFile(c.refute))
  const unknown = refuted?.results.find((r) => !merged.findings.some((f) => f.id === r.finding_id))
  if (unknown !== undefined) throw new CritiqueError('invalid-run', `/r/refute.json refutes ${unknown.finding_id}, which the run does not have`)
  const refutes = refuted?.results
  const warnings = [...(merged.ledger_warnings ?? []), ...(refuted?.ledger_warnings ?? [])]
  const adjudicated = node.applyRefutes(merged.findings.map((f) => node.adjudicate(verdicts, f)), refutes ?? [])
  const gate = c.gate === null ? undefined : node.parseGateRecord(c.gate)
  const run = {
    critics: merged.critics, degraded: merged.degraded, depth: m.depth, depth_choice: m.depth_choice, dropped: m.dropped, findings: adjudicated, flow: m.flow,
    ...(gate === undefined ? {} : { gate }), ...(warnings.length === 0 ? {} : { ledger_warnings: warnings }), repository: m.repository, round: m.round,
    ...(m.previous_run_id === undefined ? {} : { previous_run_id: m.previous_run_id }), ...(m.plan_warnings === undefined ? {} : { plan_warnings: m.plan_warnings }),
    ...(refutes === undefined ? {} : { refutes }), run_id: m.run_id, ...(merged.skipped_checks === undefined ? {} : { skipped_checks: merged.skipped_checks }),
    spec: node.specReference(m), stage: m.stage, ...(m.ticket_check === undefined ? {} : { ticket_check: m.ticket_check }),
  }
  const candidates = node.candidateQuestions(run)
  const unasked = candidates?.deferred.find((f) => f.verdict.answered === true)
  if (unasked !== undefined) throw new CritiqueError('invalid-input', `${unasked.id} is past the ${String(candidates?.asked.length)} questions a run asks, so it was not asked and cannot be answered`)
  return {
    asked: candidates?.asked.map((f) => f.id) ?? null, counts: node.countVerdicts(run), deferred: candidates?.deferred.map((f) => f.id) ?? null,
    ledger: node.ledgerLines(run, new Date('2026-11-03T12:00:00Z')), report: node.renderReport(run), second_round_due: node.secondRoundDue(run), summary: node.renderSummary(run, '/r/report.md'),
  }
}

const out = []
const kinds = {}
const seen = {}
for (let i = 0; i < Number(N); ) {
  const big = chance(0.025)
  const stage = big ? pick('spec', 'tickets', 'plan', 'adr-review') : pick(...WEIGHTED)
  const id = `r${int(1, 99)}`
  const critics = Array.from({ length: int(0, 4) }, (_, k) => criticRun(`c${k + 1}`, pick(...FAMILIES), k > 2 ? 'c1' : undefined))
  const tickets = ['tickets/01.md', 'tickets/02.md', 'tickets/03.md']
  // A big run is what makes a summary outgrow its budget.
  const count = big ? int(34, 40) : stage === 'architecture-review' || stage === 'decision-recovery' ? int(0, 20) : int(0, 7)
  const findings = Array.from({ length: count }, (_, n) => finding(n + 1, critics, stage, count))
  const fan = {
    critics, degraded: critics.every((c) => c.status !== 'ok'), findings, findings_path: '/r/findings.json', run_id: id, stage, status: 'reviewed',
    ...(chance(0.25) ? { ledger_warnings: ['gateway busy'] } : {}),
    ...(chance(0.3) ? { skipped_checks: [{ check: 'needs-tree-check', from: 'category', about: text() }, { check: 'other', from: 'role bug-hunter' }].slice(0, int(0, 2)) } : {}),
  }
  // A run with more candidates than it asks, the last of which may be answered although it was never asked.
  const crowded = (stage === 'architecture-review' || stage === 'decision-recovery') && count >= 13 && chance(0.4)
  if (crowded) for (const f of findings) Object.assign(f, { severity: 'major' }), delete f.pre_verdict, delete f.declared_severity
  const verdicts = crowded ? findings.map((f, n) => ({ finding_id: f.id, verdict: 'needs-human', reason: text(), ...(n === count - 1 && chance(0.5) ? { answered: true } : {}) })) : findings.filter(() => (count > 30 ? true : chance(0.93))).map((f) => verdictOf(stage, f, tickets, count > 30))
  const mutate = (t) => (chance(0.04) ? pick(...MUT)(t) : t)
  const nh = verdicts.filter((v) => v.verdict === 'needs-human')
  if ((stage === 'architecture-review' || stage === 'decision-recovery') && nh.length > 0 && chance(0.1)) nh[nh.length - 1].answered = true
  const record = {
    manifest: mutate(JSON.stringify(manifest(id, stage, chance(0.3) ? 2 : 1, tickets))), findings: mutate(JSON.stringify(fan)),
    verdicts: !big && chance(0.02) ? [{ verdict: 'rejected' }] : !big && chance(0.02) ? [...verdicts, { finding_id: 'ghost', verdict: 'hint' }] : !big && chance(0.02) && verdicts.length > 0 ? [...verdicts, verdicts[0]] : verdicts, refute: chance(0.4) ? mutate(refuteText(id, findings, critics)) : null,
    gate: stage === 'feature-review' ? gateText(id, findings) : null,
  }
  try {
    record.expected = { ok: adjudicateRun(record) }
  } catch (error) {
    if (!(error instanceof CritiqueError)) throw error
    record.expected = { error: { code: error.code, message: error.message } }
  }
  const kind = record.expected.error ? 'error' : 'ok'
  // Every kind of error is kept a few times, and the rest of them rarely.
  const shape = record.expected.error?.message.replace(/\S*\d\S*/g, 'N').replace(/f\w*:|\bghost\b/g, 'X').slice(0, 48)
  seen[shape] = (seen[shape] ?? 0) + 1
  if (kind === 'error' && seen[shape] > 2 && chance(0.9)) continue
  kinds[`${kind} ${stage}`] = (kinds[`${kind} ${stage}`] ?? 0) + 1
  out.push(record)
  i += 1
}
writeFileSync(OUT, JSON.stringify(canon(out)))
console.log(`${out.length} cases`, kinds)
