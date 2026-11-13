#!/usr/bin/env node
// Writes the scenario.json files of record (the goldens are expected.json, from regenerate-fixtures.mjs).
// A run directory is written under runs/ by the step that records it, as fan-out leaves one: a manifest and the
// merged findings, with a refute round's file when the run had one. gh is a fake program in the sandbox's bin/.
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const SB = '@SANDBOX@'
const KEY = 'github.com/jorisjonkers-dev/example'
const GITHUB = 'git@github.com:JorisJonkers-dev/example.git'
const URL = 'https://github.com/jorisjonkers-dev/example/issues'
const q = (text) => `'${text.split("'").join("'\\''")}'`
const scenarios = {}
const json = (value) => `${JSON.stringify(value)}\n`

const LIMITS = { antigravity_quota: { daily_cap: 20, reserve_threshold: 10, reserved_stages: ['spec'] }, search: { max_bytes: 8192, max_matches: 50 } }
const HASHES = { contract: 'c0ffee01', preamble: 'c0ffee02', role: 'c0ffee03', schema: 'c0ffee04' }
const dir = (id) => `${SB}/runs/${id}`

/** A run's manifest, as `prepare` writes one. */
const manifest = (id, extra = {}) => ({
  allowed_families: ['mistral'], artifact: { labels: [], reference: 'o/r#1', tracker: 'github' }, created_at: '2026-10-30T12:00:00.000Z', critics: [], depth: 'base',
  depth_choice: { chosen_by: 'argument', measures: {}, signals: [] }, distinct_families: true, dropped: [], flow: 'flow-1', limits: LIMITS, orchestrator_family: 'anthropic',
  packet_dir: `${dir(id)}/packet`, packet_files: [], repo_root: `${SB}/repos/r1`, repository: KEY, run_dir: dir(id), run_id: id, stage: 'spec', version: 1, ...extra,
})
const critic = (id, extra = {}) => ({
  charged_usd: 0.01, client: 'vibe-small', cost_usd: 0.0123, critic_id: id, duration_ms: 1200, exit_code: 0, family: 'mistral', finding_count: 1, lens_hashes: { 'lens-a': 'a1b2c3d4' }, lenses: ['lens-a'],
  model: 'mistral-small', prompt_hashes: HASHES, prompt_path: `/p/${id}.md`, role: 'spec-reviewer', status: 'ok', ...extra,
})
const finding = (id, extra = {}) => ({
  id, critic_finding_id: `c1-${id}`, raised_by: ['c1'], severity: 'major', category: 'omission', location: 'spec#L10', claim: `The spec says nothing about ${id}.`,
  evidence: [{ kind: 'quote', value: 'a quote', file: 'artifact/spec.md' }], ...extra,
})
const fanOut = (id, critics, findings, extra = {}) => ({
  critics, degraded: critics.every((c) => c.status !== 'ok'), findings, findings_path: `${dir(id)}/findings.json`, run_id: id, stage: 'spec', status: 'reviewed', ...extra,
})
/** The files of a run directory. */
const run = (id, { stage = 'spec', critics = [critic('c1')], findings = [], refute = null, manifestExtra = {}, fanExtra = {} } = {}) => ({
  [`runs/${id}/manifest.json`]: json(manifest(id, { stage, ...manifestExtra })),
  [`runs/${id}/findings.json`]: `${JSON.stringify(fanOut(id, critics, findings, { stage, ...fanExtra }), null, 2)}\n`,
  ...(refute === null ? {} : { [`runs/${id}/refute.json`]: json({ counts: {}, refute_path: `${dir(id)}/refute.json`, results: refute, run_id: id, stage, status: 'checked' }) }),
})
const record = (id, verdicts, extra = {}) => ({ argv: ['record', '--json', JSON.stringify({ run_dir: dir(id), verdicts })], roots: ['runs'], ...extra })
const raw = (input, extra = {}) => ({ argv: ['record', '--json', typeof input === 'string' ? input : JSON.stringify(input)], roots: ['runs'], ...extra })
const v = (finding_id, verdict, extra = {}) => ({ finding_id, verdict, ...extra })
const accept = (id, extra = {}) => v(id, 'accepted', { reason_code: 'rule-holds', ...extra })

// A spec run: accepted, rejected with dissent, a hint, a question, a pre-verdict, a severity override, and a second run
// with a refute round, recorded into the same ledger.
const C2 = critic('c2', { family: 'zhipu', client: 'glm-flash', status: 'failed', failure: 'timeout', cost_usd: null, exit_code: 124, finding_count: 0, detail: 'it timed out', tools: ['read_file'], search_failure: 'no index' })
const C3 = critic('c3', { family: 'zhipu', client: 'glm-flash', retry_of: 'c2', no_findings_reason: 'checked the retry policy and found it sound', finding_count: 0, vibe_cost_usd: 0.011, tokens: { cached: 10, input: 20, output: 30 }, stdout_path: '/o', export_path: '/e' })
const SPEC_FINDINGS = [
  finding('f1', { severity: 'critical', raised_by: ['c1', 'c3'], suggestion: 'Say so.' }),
  finding('f2', { severity: 'critical', category: 'contradiction', raised_by: ['c1'], claim: 'x'.repeat(200) }),
  finding('f3', { severity: 'minor', raised_by: ['c1'] }),
  finding('f4', { severity: 'critical', category: 'unasked-question', raised_by: ['c3'], evidence: [] }),
  finding('f5', { raised_by: ['c1'], pre_verdict: { verdict: 'hint', reason_code: 'no-spec-quote', reason: 'no quote from the spec' } }),
  finding('f6', { raised_by: ['c1'], pre_verdict: { verdict: 'rejected', reason_code: 'fabricated-evidence', reason: 'the quote is not in the packet' } }),
  finding('f7', { raised_by: ['c1'], severity: 'major', declared_severity: 'major', check: 'retry-bound' }),
  finding('f8', { severity: 'minor', raised_by: ['c3'] }),
]
const SPEC_VERDICTS = [
  accept('f1', { reason: 'confirmed' }), v('f2', 'rejected', { reason: 'the spec does say it', reason_code: 'out-of-scope' }), v('f3', 'hint'),
  v('f4', 'needs-human', { reason: 'which retry budget?' }), v('f6', 'rejected', { reason: 'fabricated' }), accept('f7', { severity: 'minor', severity_reason: 'low-impact' }), v('f8', 'rejected', { reason: 'nope' }),
]
const refutes = [
  { finding_id: 'f1', outcome: 'refute', answer: 'refute', reply: { category: 'omission', check: 'refute', claim: 'The spec does say it in section 4.', evidence: [{ kind: 'quote', value: 'q' }], location: 'spec#L10', severity: 'critical' }, refuter: critic('r-f1-glm', { critic_id: 'r-f1-glm', family: 'zhipu', client: 'glm-flash', role: 'refuter' }) },
  { finding_id: 'f4', outcome: 'downgrade', answer: 'downgrade', severity: 'major', reply: { category: 'unasked-question', check: 'downgrade', claim: 'Only a major.', evidence: [], location: 'spec#L1', severity: 'major' }, refuter: critic('r-f4-glm', { critic_id: 'r-f4-glm', family: 'zhipu', client: 'glm-flash', role: 'refuter' }) },
  { finding_id: 'f7', outcome: 'stands', answer: 'refute', reason_code: 'no-spec-quote', detail: 'no quote', reply: { category: 'omission', check: 'refute', claim: 'weak', evidence: [], location: 'x', severity: 'major' } },
  { finding_id: 'f2', outcome: 'skipped', reason_code: 'no-eligible-client', detail: 'no routed client can refute it' },
]
scenarios['record-spec'] = [
  { argv: ['--help'], write: { ...run('r1', { critics: [critic('c1', { finding_count: 6 }), C2, C3], findings: SPEC_FINDINGS, fanExtra: { ledger_warnings: ['gateway busy'] } }) } },
  record('r1', SPEC_VERDICTS),
  record('r1', SPEC_VERDICTS),
  // A second run, which records into the same ledger, with the refute round applied.
  { ...record('r2', [accept('f1'), accept('f4', { reason_code: 'rule-holds' }), accept('f7'), v('f2', 'rejected', { reason: 'no' })]), write: run('r2', { critics: [critic('c1'), C3], findings: [SPEC_FINDINGS[0], SPEC_FINDINGS[3], { ...SPEC_FINDINGS[6], declared_severity: undefined }, SPEC_FINDINGS[1]], refute: refutes, manifestExtra: { depth: 'deep', depth_choice: { chosen_by: 'signals', measures: { sections: { threshold: 5, value: 9 } }, signals: [{ kind: 'threshold', measure: 'sections', threshold: 5, value: 9 }, { kind: 'risk-term', term: 'auth' }, { kind: 'label', label: 'risky' }] }, plan_warnings: ['vibe: 90% of the weekly plan used'], round: 2, previous_run_id: 'r1' } }) },
  // An empty refute round, and a run with more than three accepted criticals, which calls for a second round.
  { ...record('r3', [accept('f1'), accept('f2'), accept('f3'), accept('f4')]), write: run('r3', { findings: ['f1', 'f2', 'f3', 'f4'].map((id) => finding(id, { severity: 'critical' })), refute: [] }) },
]

// What record refuses: its input, the files of the run, and every verdict that does not fit.
const ERRORS = run('e1', { critics: [critic('c1')], findings: [finding('f1', { declared_severity: 'major' }), finding('f2', { pre_verdict: { verdict: 'hint', reason_code: 'no-evidence', reason: 'x' } }), finding('f3', { pre_verdict: { verdict: 'rejected', reason_code: 'single-site', reason: 'y' } }), finding('f4', { severity: 'minor', raised_by: ['c1', 'ghost'] })] })
scenarios['record-errors'] = [
  { argv: ['--help'], write: { ...ERRORS, 'runs/bad-manifest/manifest.json': '{not json', 'runs/bad-findings/manifest.json': json(manifest('bad-findings')), 'runs/bad-findings/findings.json': '{"critics": 5}', 'runs/no-findings/manifest.json': json(manifest('no-findings')), ...run('bad-refute', { findings: [finding('f1')], refute: [{ finding_id: 'ghost', outcome: 'stands' }] }), 'runs/bad-refute-shape/manifest.json': json(manifest('bad-refute-shape')), 'runs/bad-refute-shape/findings.json': JSON.stringify(fanOut('bad-refute-shape', [critic('c1')], [finding('f1')])), 'runs/bad-refute-shape/refute.json': '{"results": [{"finding_id": "f1", "outcome": "maybe"}]}' } },
  raw('{}'), raw('[]'), raw({ run_dir: dir('e1') }), raw({ run_dir: dir('e1'), verdicts: 'x' }), raw({ run_dir: dir('e1'), verdicts: [{ finding_id: 'f1', verdict: 'maybe' }] }),
  raw({ run_dir: dir('e1'), verdicts: [{ finding_id: 'f1', verdict: 'rejected' }] }), raw({ run_dir: dir('e1'), verdicts: [accept('f1', { reason_code: 'Not A Slug' })] }),
  raw({ run_dir: dir('missing'), verdicts: [] }), raw({ run_dir: dir('bad-manifest'), verdicts: [] }), raw({ run_dir: dir('no-findings'), verdicts: [] }), raw({ run_dir: dir('bad-findings'), verdicts: [] }),
  record('e1', [accept('f1'), accept('f1')]), record('e1', [accept('nope')]),
  record('e1', [accept('f1', { answered: true })]), record('e1', [v('f1', 'needs-human', { follow_up: '#3' })]), record('e1', [accept('f1', { severity: 'major', severity_reason: 'same' })]),
  record('e1', [accept('f1', { severity: 'minor' })]), record('e1', [accept('f2', { severity: 'minor', severity_reason: 'x' })]), record('e1', [accept('f2', { severity_reason: 'orphan' })]),
  record('e1', [accept('f1'), v('f2', 'needs-human'), v('f3', 'rejected', { reason: 'r' }), v('f4', 'hint')]),
  record('e1', [accept('f1'), v('f3', 'accepted')]),
  // The first that fits is recorded, and a run is recorded once.
  record('e1', [accept('f1'), v('f3', 'rejected', { reason: 'r' }), v('f4', 'hint'), v('f2', 'hint')]),
  record('e1', [accept('f1'), v('f4', 'hint')]),
  record('bad-refute', [accept('f1')]), record('bad-refute-shape', [accept('f1')]),
]

// The stages: decisions (a question answered), tickets (a ticket check), a degraded run, a run whose critics broke the
// rules, a documents-only ADR review, a plan, and a recovery run that asks ten questions and defers the rest.
const D = [critic('c1', { role: 'adr-reviewer' })]
const check = { status: 'failed', ticket_ids: ['#1', '#2', '#3'], cycles: [['#1', '#2']], dangling_blockers: Array.from({ length: 7 }, (_, i) => ({ ticket: `#${i + 1}`, blocker: `#9${i}` })), uncited_stories: [2, 4], coverage_reason: 'x' }
const CANDIDATES = Array.from({ length: 13 }, (_, i) => finding(`f${i + 1}`, { severity: ['critical', 'major', 'minor'][i % 3], category: 'undocumented-decision', claim: `Decision ${i + 1} is implied by the code.` }))
scenarios['record-stages'] = [
  { argv: ['--help'], write: { ...run('d1', { stage: 'decisions', findings: [finding('f1', { severity: 'critical' }), finding('f2', { severity: 'critical' }), finding('f3', { severity: 'minor' })], critics: D }), ...run('t1', { stage: 'tickets', manifestExtra: { ticket_check: check }, findings: [finding('f1', { severity: 'critical' })], critics: [critic('c1'), critic('c2', { family: 'deepseek', status: 'violated-readonly', failure: 'unsafe-tool', exit_code: 1 }), critic('c3', { status: 'budget-skip', detail: 'cap spent', exit_code: null, cost_usd: null })] }), ...run('x1', { stage: 'adr-review', critics: [critic('c1', { status: 'failed', failure: 'exit-137', cost_usd: 0 })], fanExtra: { skipped_checks: [{ check: 'tree-quote', from: 'category', about: 'a quote of the code' }, { check: 'import-edge', from: 'lens layering' }] }, manifestExtra: { dropped: [{ client: 'agy-pro', family: 'google', reason: 'quota', detail: 'daily cap', replaced_by: 'c9' }, { client: 'codex', family: 'openai', reason: 'unavailable', deep: true }, { client: 'x', family: 'zhipu', reason: 'same-family' }] } }), ...run('p1', { stage: 'plan', findings: [finding('f1')] }), ...run('a1', { stage: 'architecture-review', findings: CANDIDATES.concat([finding('g1', { claim: 'A real finding.' })]) }), ...run('k1', { stage: 'decision-recovery', findings: CANDIDATES, critics: [critic('c1', { role: 'decision-recoverer' })] }), ...run('k2', { stage: 'decision-recovery', findings: CANDIDATES.slice(0, 3), critics: [critic('c1')] }), ...run('k3', { stage: 'decision-recovery', findings: CANDIDATES, critics: [critic('c1')] }) } },
  record('d1', [accept('f1', { answered: true }), accept('f2', { reason_code: 'skipped-by-owner' }), v('f3', 'rejected', { reason: 'no' })]),
  record('t1', [accept('f1')]),
  record('x1', []),
  record('p1', [accept('f1')]),
  record('a1', [...CANDIDATES.map((c) => v(c.id, 'needs-human', { reason: `Why ${c.id}?` })), accept('g1')]),
  record('k1', CANDIDATES.map((c) => v(c.id, c.id === 'f2' ? 'rejected' : 'needs-human', { reason: `Why ${c.id}?`, ...(c.id === 'f1' ? { answered: true } : {}) }))),
  // The twelfth candidate is past the ten questions a run asks, so the owner cannot have answered it.
  record('k3', CANDIDATES.map((c) => v(c.id, 'needs-human', { reason: `Why ${c.id}?`, ...(c.id === 'f12' ? { answered: true } : {}) }))),
  record('k2', [v('f1', 'needs-human'), accept('f2'), v('f3', 'hint')]),
  record('k2', [v('f1', 'needs-human'), v('f2', 'rejected', { reason: 'x' }), v('f3', 'hint', { answered: true })]),
  record('k2', [v('f1', 'needs-human'), v('f2', 'needs-human', { answered: true }), v('f3', 'rejected', { reason: 'no', reason_code: 'invented-rationale' })]),
]

// A feature review's gate: each accepted finding names a follow-up that is a ticket of the spec; the gate fails while a
// critical follow-up, this run's or an earlier run's, is open on the tracker.
const q2 = (text) => q(text)
const answer = ({ out = '', err = '', code = 0 }) => `printf '%s' ${q2(out)}\nprintf '%s' ${q2(err)} >&2\nexit ${code}`
const fakeGh = (lists) => `#!/bin/sh\nif [ "$1" = api ]; then\ncase "$6" in\n${Object.entries(lists).map(([path, reply]) => `*/issues/${path}?per_page=100)\n${answer(reply)}\n;;`).join('\n')}\nesac\nfi\necho "gh: unexpected: $*" >&2\nexit 9\n`
const sub = (number, state = 'open') => ({ number, title: `Ticket ${number}`, body: `body ${number}`, html_url: `${URL}/${number}`, labels: [], state })
const pages = (...items) => ({ out: JSON.stringify([items]) })
const withGh = (states, step) => ({ ...step, ...gh(states) })
const gh = (states) => ({ write: { 'bin/gh': fakeGh({ '1/sub_issues': pages(...Object.entries(states).map(([n, s]) => sub(Number(n), s))), ...Object.fromEntries(Object.keys(states).map((n) => [`${n}/dependencies/blocked_by`, pages()])), '9/sub_issues': { err: 'HTTP 404: Not Found', code: 1 } }) }, exec: ['bin/gh'], path: ['bin'], shims: ['env', 'git'] })
const feature = (id, extra = {}) => run(id, { stage: 'feature-review', manifestExtra: { artifact: { labels: [], reference: `${KEY}#1`, tracker: 'github' }, code: { base: 'main', base_commit: 'aaaaaaa', changed_files: ['a.rs'], head: 'HEAD', head_commit: `bbb${id}`, ...extra.code }, packet_files: [{ kind: 'ticket', path: 'tickets/issue-2.md', source: `${KEY}#2`, sha256: 'x' }, { kind: 'ticket', path: 'tickets/issue-3.md', source: `${KEY}#3`, sha256: 'x' }, { kind: 'ticket', path: 'tickets/issue-4.md', source: `${KEY}#4`, sha256: 'x' }], ...extra.manifest }, findings: extra.findings ?? [finding('f1', { severity: 'critical', category: 'bug' }), finding('f2', { severity: 'major', category: 'integration' })], critics: extra.critics })
const trace = (id, follow, extra = {}) => accept(id, { reason_code: 'cross-ticket-trace', trace_tickets: ['tickets/issue-2.md', 'tickets/issue-3.md'], follow_up: follow, ...extra })
const failing = (id, follow, extra = {}) => accept(id, { reason_code: 'test-fails', follow_up: follow, ...extra })
scenarios['record-feature-review'] = [
  { argv: ['--help'], write: { 'repos/r1/README.md': '# example\n', ...feature('g1'), ...feature('g2', { findings: [finding('f1', { severity: 'minor', category: 'drift' })] }), ...feature('g3', { findings: [finding('f1', { category: 'bug' })] }), ...feature('g4'), ...feature('g8', { findings: [finding('f1', { severity: 'critical', category: 'bug' })] }), ...feature('g5', { manifest: { artifact: { labels: [], reference: `${KEY}#9`, tracker: 'github' } } }), ...feature('g6', { manifest: { artifact: { labels: [], reference: `${KEY}#1`, tracker: 'github' }, repo_root: `${SB}/repos/gitlab` } }), ...run('g7', { stage: 'feature-review' }), 'repos/gitlab/docs/agents/issue-tracker.md': '# Issue tracker: GitLab\n', 'repos/gitlab/a.md': 'a\n' } },
  // Critical follow-up #2 is open: the gate fails.
  withGh({ 2: 'open', 3: 'open', 4: 'closed' }, record('g1', [failing('f1', '#2'), trace('f2', `${KEY}#4`)])),
  // A later run of the same spec holds the gate shut by the earlier critical, though it accepts nothing.
  withGh({ 2: 'open', 3: 'open', 4: 'closed' }, record('g2', [v('f1', 'rejected', { reason: 'minor' })])),
  // The same critical ticket named again counts once.
  withGh({ 2: 'open', 3: 'open', 4: 'closed' }, record('g8', [failing('f1', '#2')])),
  // The follow-up is closed now: the gate passes, and the earlier run no longer holds it.
  withGh({ 2: 'closed', 3: 'closed', 4: 'closed' }, record('g3', [failing('f1', '3')])),
  // What the gate refuses: a follow-up that is no ticket of the spec, the tracker, a feature run without its commits.
  withGh({ 2: 'closed', 3: 'closed' }, record('g4', [failing('f1', '#77'), failing('f2', '#3')])),
  withGh({ 2: 'closed', 3: 'closed' }, record('g5', [failing('f1', '#2'), failing('f2', '#3')])),
  withGh({ 2: 'closed', 3: 'closed' }, record('g6', [failing('f1', '#2'), failing('f2', '#3')])),
  record('g7', []),
  record('g4', [accept('f1')]),
  record('g4', [accept('f1', { reason_code: 'rule-holds', follow_up: '#2' })]),
  record('g4', [failing('f1', '#2', { trace_tickets: ['tickets/issue-2.md'] })]),
  record('g4', [trace('f1', '#2', { trace_tickets: ['tickets/issue-2.md'] })]),
  record('g4', [trace('f1', '#2', { trace_tickets: ['tickets/issue-2.md', 'tickets/nope.md'] })]),
  record('g4', [accept('f1', { reason_code: 'test-fails' })]),
  record('g4', [v('f1', 'rejected', { reason: 'x', follow_up: '#2' })]),
  { argv: ['--help'], write: { 'runs/gates.jsonl': 'not json\n' } },
  withGh({ 2: 'closed', 3: 'closed' }, record('g4', [failing('f1', '#2'), failing('f2', '#3')])),
]

// A ticket review: the staged change it vouches for, an unfixed critical, a degraded review and a review of a commit.
const ticketRun = (id, extra = {}) => run(id, { stage: 'ticket-review', manifestExtra: { artifact: { labels: [], reference: `${KEY}#5`, tracker: 'github' }, repo_root: `${SB}/repos/t1`, code: { base: 'HEAD', base_commit: 'aaaaaaa', changed_files: ['a.rs'], head: 'HEAD', head_commit: 'bbbbbbb', ...extra.code }, packet_files: [{ kind: 'spec', path: 'spec/spec.md', source: `${KEY}#1`, sha256: 'x' }] }, findings: extra.findings ?? [finding('f1', { severity: 'critical', category: 'bug' }), finding('f2', { severity: 'major', category: 'bug' })], critics: extra.critics })
const walked = (id, extra = {}) => accept(id, { reason_code: 'trace-walked', ...extra })
const kept = (id, extra = {}) => accept(id, { reason_code: 'test-fails', regression_test: 'tests/a.rs', ...extra })
scenarios['record-ticket-review'] = [
  { argv: ['--help'], write: { 'repos/t1/a.rs': 'fn a() {}\n' }, git: [['init', '-q', '--template=', 'repos/t1'], ['-C', 'repos/t1', 'add', '-A'], ['-C', 'repos/t1', 'commit', '-q', '-m', 'base']] },
  { argv: ['--help'], write: { 'repos/t1/a.rs': 'fn a() { 1; }\n', ...ticketRun('s1', { code: { staged_diff: 'deadbeef' } }), ...ticketRun('s2', { code: { staged_diff: 'deadbeef' } }), ...ticketRun('s3', { critics: [critic('c1', { status: 'failed', failure: 'timeout' }), critic('c2', { status: 'violated-readonly' }), critic('c3', { status: 'budget-skip' })] }), ...ticketRun('s4', { code: { staged_diff: 'deadbeef' }, critics: [critic('c1', { status: 'failed' })], findings: [] }), ...run('s5', { stage: 'ticket-review' }) }, git: [['-C', 'repos/t1', 'add', '-A']] },
  record('s1', [kept('f1'), walked('f2', { fixed: true })]),
  record('s2', [kept('f1', { fixed: true }), walked('f2')]),
  record('s3', [walked('f1', { fixed: true }), walked('f2')]),
  record('s4', []),
  record('s5', []),
  record('s1', [accept('f1')]), record('s1', [kept('f1', { regression_test: undefined })]), record('s1', [walked('f1', { regression_test: 'tests/a.rs' })]), record('s1', [v('f1', 'rejected', { reason: 'x', fixed: true })]),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
