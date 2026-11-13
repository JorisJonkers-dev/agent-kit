#!/usr/bin/env node
// Writes the scenario.json files of state and budget (the goldens are expected.json, from regenerate-fixtures.mjs).
// gh is a fake program in the sandbox's bin/: `gh issue view` prints the spec and `gh api` its sub-issues and
// blockers. Runs are run directories with a manifest, or lines of critique's ledger.
import { createHash } from 'node:crypto'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const SB = '@SANDBOX@'
const KEY = 'github.com/jorisjonkers-dev/example'
const URL = 'https://github.com/jorisjonkers-dev/example/issues'
const R = (dir) => `${SB}/repos/${dir}`
const q = (text) => `'${text.split("'").join("'\\''")}'`
const OWN = 'home/.local/share/agent-kit/critique/ledger.jsonl'
const OVERRIDE = 'home/.config/agent-kit/critique-routing.json'
const GATEWAY_CONFIG = 'home/.config/agent-kit/model-gateway.json'
const state = (input, extra = {}) => ({ argv: ['state', '--json', typeof input === 'string' ? input : JSON.stringify(input)], path: ['bin'], ...extra })
const budget = (input, extra = {}) => ({ argv: ['budget', '--json', typeof input === 'string' ? input : JSON.stringify(input)], path: ['bin'], ...extra })
const init = (dir, remote = 'git@github.com:JorisJonkers-dev/example.git') => [
  ['init', '-q', '--template=', `repos/${dir}`],
  ...(remote === null ? [] : [['-C', `repos/${dir}`, 'remote', 'add', 'origin', remote]]),
  ['-C', `repos/${dir}`, 'add', '-A'],
  ['-C', `repos/${dir}`, 'commit', '-q', '-m', 'base'],
]

const answer = ({ out = '', err = '', code = 0 }) => `printf '%s' ${q(out)}\nprintf '%s' ${q(err)} >&2\nexit ${code}`
const fakeGh = ({ views = {}, lists = {} }) =>
  `#!/bin/sh\nif [ "$1" = api ]; then\ncase "$6" in\n${Object.entries(lists)
    .map(([path, reply]) => `*/issues/${path}?per_page=100)\n${answer(reply)}\n;;`)
    .join('\n')}\nesac\nelif [ "$1 $2" = "issue view" ]; then\ncase "$3" in\n${Object.entries(views)
    .map(([number, reply]) => `${number})\n${answer(reply)}\n;;`)
    .join('\n')}\nesac\nfi\necho "gh: unexpected: $*" >&2\nexit 9\n`
const view = (number) => ({ out: JSON.stringify({ number, title: `Spec ${number}`, body: '## User Stories\n\n1. one\n', url: `${URL}/${number}`, labels: [] }) })
const sub = (number, state = 'open') => ({ number, title: `Ticket ${number}`, body: 'b', html_url: `${URL}/${number}`, labels: [], state })
const pages = (...list) => ({ out: JSON.stringify(list) })
const GH = {
  views: { 1: view(1), 5: view(5), 9: { err: 'not found', code: 1 } },
  lists: {
    '1/sub_issues': pages([sub(2), sub(3), sub(4, 'closed')]),
    '2/dependencies/blocked_by': pages([{ number: 3, html_url: `${URL}/3` }]),
    '3/dependencies/blocked_by': pages([]),
    '4/dependencies/blocked_by': pages([]),
    '5/sub_issues': { err: 'HTTP 500', code: 1 },
  },
}
const tools = { write: { 'bin/gh': fakeGh(GH) }, exec: ['bin/gh'], shims: ['git'] }

const lines = (...objects) => `${objects.map((o) => (typeof o === 'string' ? o : JSON.stringify(o))).join('\n')}\n`
const context = (run_id, stage, extra = {}) => ({ recorded_at: '2026-11-02T10:00:00.000Z', repository: KEY, run_id, stage, ...extra })
const runLine = (run_id, stage, extra = {}) => ({ type: 'run', ...context(run_id, stage), client: 'vibe', cost_usd: 0.1, critic_id: 'c1', failure: null, finding_count: 1, lens_hashes: {}, lenses: [], model: 'm', prompt_hashes: { contract: 'c', preamble: 'p', role: 'r', schema: 's' }, role: 'r', status: 'ok', round: 1, ...extra })
const finding = (run_id, stage, finding_id, verdict, severity = 'major', extra = {}) => ({ type: 'finding', ...context(run_id, stage), finding_id, pre_verdict: null, raised_by: ['c1'], severity, verdict, ...extra })
const flowLine = (flow, event) => ({ type: 'flow', flow, recorded_at: '2026-11-02T09:00:00.000Z', repository: KEY, ...event })
const invocation = (flow, kind, charged_usd, extra = {}) => ({ type: 'invocation', recorded_at: '2026-11-03T08:00:00.000Z', repository: KEY, flow, workflow: flow, round: 1, stage: 'spec', kind, charged_usd, critic_id: 'c1', run_id: 'r-x', ...extra })
const SPEC = `${KEY}#1`
const LIMITS = { antigravity_quota: { daily_cap: 20, reserve_threshold: 10, reserved_stages: ['spec'] }, search: { max_bytes: 8192, max_matches: 50 } }
const manifest = (repo, id, extra = {}) =>
  `${JSON.stringify({
    allowed_families: ['mistral'],
    artifact: { labels: [], reference: SPEC, tracker: 'github' },
    created_at: '2026-10-30T12:00:00.000Z',
    critics: [],
    depth: 'base',
    depth_choice: { chosen_by: 'argument', measures: {}, signals: [] },
    distinct_families: true,
    dropped: [],
    flow: SPEC,
    limits: LIMITS,
    orchestrator_family: 'anthropic',
    packet_dir: `${SB}/repos/${repo}/.git/critique/${id}/packet`,
    packet_files: [],
    repo_root: `${SB}/repos/${repo}`,
    repository: KEY,
    run_dir: `${SB}/repos/${repo}/.git/critique/${id}`,
    run_id: id,
    stage: 'spec',
    version: 1,
    ...extra,
  })}\n`
const flows = (repo) => `repos/${repo}/.git/critique/flows`
/** The state file a flow is cached in, as flowStateFileName names it. */
const stateFile = (flow) => `${flow.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 60)}-${createHash('sha256').update(flow).digest('hex').slice(0, 12)}.json`
const scenarios = {}

scenarios['state-flow'] = [
  { argv: ['--help'], write: { 'repos/s1/a.md': 'a\n', 'repos/s2/a.md': 'a\n', 'repos/s3/docs/agents/issue-tracker.md': '# Issue tracker: GitLab\n' }, git: [...init('s1'), ...init('s2', null), ...init('s3')] },
  // A draft flow: nothing yet, then grilled, then cached, then a record that refreshes it.
  { ...state({ repo: R('s1'), draft: 'my-idea' }), ...tools, roots: [flows('s1')] },
  state({ repo: R('s1'), draft: 'my-idea', record: { grilled: true } }, { roots: [flows('s1'), 'home/.local/share/agent-kit/critique'] }),
  state({ repo: R('s1'), draft: 'my-idea', cached: true }),
  state({ repo: R('s1'), draft: 'my-idea', cached: true, record: { no_critique: true } }),
  state({ repo: R('s1'), draft: 'my-idea', record: { no_critique: false } }),
  // A spec flow linked to its draft, with run directories, ledger runs, findings and a gate.
  state(
    { repo: R('s1'), spec: '#1', draft: 'my-idea' },
    {
      write: {
        [`repos/s1/.git/critique/20261101T100000Z-spec/manifest.json`]: manifest('s1', '20261101T100000Z-spec'),
        [`repos/s1/.git/critique/20261101T100000Z-spec/report.md`]: '# report\n',
        [`repos/s1/.git/critique/20261102T100000Z-tickets/manifest.json`]: manifest('s1', '20261102T100000Z-tickets', { stage: 'tickets', ticket_check: { cycles: [], dangling_blockers: [], status: 'passed', ticket_ids: ['#2', '#3', '#4'], uncited_stories: null, coverage_reason: 'no ticket cites a user story' } }),
        [`repos/s1/.git/critique/notes/x.md`]: 'not a run\n',
        [OWN]: lines(
          flowLine('draft:my-idea', { event: 'grilled' }),
          runLine('20261102T100000Z-tickets', 'tickets', { flow: SPEC }),
          runLine('20261103T100000Z-ticket-review', 'ticket-review', { spec: SPEC, flow: `${KEY}#2` }),
          runLine('20261099T100000Z-other', 'spec', { flow: 'github.com/o/r#1' }),
          finding('20261101T100000Z-spec', 'spec', 'F1', 'needs-human', 'critical'),
          finding('20261101T100000Z-spec', 'spec', 'F2', 'needs-human', 'major'),
          finding('20261102T100000Z-tickets', 'tickets', 'F3', 'accepted', 'critical'),
          { ...context('20261103T100000Z-triage', 'ticket-review'), type: 'run', status: 'skipped-low-risk', ticket: `${KEY}#3`, flow: SPEC },
        ),
        'repos/s1/.git/critique/ticket-reviews.jsonl': lines({ base_commit: 'b', critic_failures: [], degraded: false, head_commit: 'h', recorded_at: '2026-11-03T10:00:00.000Z', repository: KEY, review: 'fail', reviewed_staged_diff: null, run_id: '20261103T100000Z-ticket-review', spec: SPEC, stage: 'ticket-review', staged_diff: null, ticket: `${KEY}#2`, unfixed_critical: ['F9'], version: 1 }),
        'repos/s1/.git/critique/gates.jsonl': lines({ base_commit: 'b', critical_open: 0, follow_ups: [], gate: 'pass', head_commit: 'old', line: 'GATE: PASS', open_critical: [], recorded_at: '2026-11-03T10:00:00.000Z', repository: KEY, run_id: 'g1', spec: SPEC, stage: 'feature-review', version: 1 }),
      },
      roots: [flows('s1'), 'home/.local/share/agent-kit/critique'],
    },
  ),
  state({ repo: R('s1'), spec: '#1', record: { answered: [{ finding_id: 'F1', run_id: '20261101T100000Z-spec' }], landed: ['#2', `${KEY}#3`] } }, { roots: [flows('s1'), 'home/.local/share/agent-kit/critique'] }),
  state({ repo: R('s1'), spec: '#1', record: { answered: [{ finding_id: 'F7', run_id: 'r' }] } }),
  state({ repo: R('s1'), spec: '#1', record: { landed: ['#99'] } }),
  state({ repo: R('s1'), spec: '#1', cached: true }),
  state({ repo: R('s1'), spec: '#1', cached: true }, { write: { [`${flows('s1')}/${stateFile(SPEC)}`]: '{}' } }),
  state({ repo: R('s1'), draft: 'a/./b' }),
  state(
    { repo: R('s1'), spec: '#1' },
    {
      write: {
        'repos/s1/.git/critique/elsewhere/manifest.json': manifest('s1', 'elsewhere', { repository: 'github.com/o/r', flow: 'github.com/o/r#1' }),
        'repos/s1/.git/critique/zz-copy/manifest.json': manifest('s1', '20261101T100000Z-spec', { round: 2, previous_run_id: 'x' }),
      },
    },
  ),
  state({ repo: R('s3'), spec: '#1' }),
  // What is refused, and what is wrong with the input.
  state({ repo: R('s1'), spec: '#9' }),
  state({ repo: R('s1'), spec: '#5' }),
  state({ repo: R('s2'), draft: 'x' }),
  state({ repo: R('s1') }),
  state({ repo: R('s1'), draft: '-bad' }),
  state({ repo: R('s1'), draft: 'x', cached: 'yes' }),
  state({ repo: R('s1'), draft: 'x', record: { answered: [{ run_id: 'r' }] } }),
  state('[]'),
  state({ repo: R('s1'), spec: '#1' }, { write: { 'repos/s1/.git/critique/gates.jsonl': '{"version":1}\n' } }),
  state({ repo: R('s1'), spec: '#1' }, { write: { 'repos/s1/.git/critique/gates.jsonl': '', 'repos/s1/.git/critique/bad/manifest.json': '{"version":1}\n' } }),
]

scenarios['state-local'] = [
  {
    argv: ['--help'],
    write: {
      'repos/l1/docs/agents/issue-tracker.md': '# Issue tracker: Local Markdown\n',
      'repos/l1/.scratch/feat/spec.md': '# Spec\n',
      'repos/l1/.scratch/feat/issues/01-a.md': '# A\n\nStatus: done\n',
      'repos/l1/.scratch/feat/issues/02-b.md': '# B\n\nBlocked by: 01\n',
      'repos/l1/.scratch/refactor-x/plan.md': '# Plan\n',
    },
    git: init('l1'),
  },
  { ...state({ repo: R('l1'), draft: 'refactor-x' }), shims: ['git'] },
  state({ repo: R('l1'), draft: 'refactor-x', record: { grilled: true } }),
  state({ repo: R('l1'), draft: 'nothing-here' }),
  state({ repo: R('l1'), spec: '.scratch/feat/spec.md' }),
  state({ repo: R('l1'), spec: '.scratch/feat/spec.md', draft: 'refactor-x', record: { landed: ['02'] } }),
]

scenarios['budget-read'] = [
  { argv: ['--help'], write: { 'repos/b1/a.md': 'a\n', 'repos/b2/a.md': 'a\n' }, git: [...init('b1'), ...init('b2', null)] },
  { ...budget({ repo: R('b1'), draft: 'my-idea' }), ...tools },
  budget({ repo: R('b1'), spec: '#1' }, { write: { [OWN]: lines(invocation(SPEC, 'vibe', 0.25), invocation('draft:my-idea', 'vibe', 0.5), invocation(SPEC, 'agy', 0), flowLine(SPEC, { draft: 'my-idea', event: 'draft' })) } }),
  budget({ repo: R('b1'), spec: '#1' }, { write: { [OWN]: lines(...Array.from({ length: 12 }, () => invocation(SPEC, 'agy', 0))) } }),
  budget({ repo: R('b1'), spec: '#1' }, { write: { [OWN]: lines(...Array.from({ length: 21 }, () => invocation(SPEC, 'agy', 0))) } }),
  budget({ repo: R('b1'), draft: 'my-idea' }, { write: { [GATEWAY_CONFIG]: '{"clients":{"vibe":{"plan_limits":[{"window":"day","limit_usd":0.0002}]}}}\n', [OWN]: lines(invocation('draft:my-idea', 'vibe', 0.5)) } }),
  budget({ repo: R('b1'), draft: 'my-idea' }, { write: { [GATEWAY_CONFIG]: '{' } }),
  budget({ repo: R('b1'), draft: 'my-idea' }, { remove: [GATEWAY_CONFIG], write: { [OVERRIDE]: '{"fallbacks":{"agy-gemini-flash-high":["agy-gemini-flash-high","vibe-glm"]}}\n', [OWN]: lines(...Array.from({ length: 21 }, () => invocation(SPEC, 'agy', 0))) } }),
  budget({ repo: R('b1'), spec: '#1' }, { write: { [OVERRIDE]: `${JSON.stringify({ stages: { spec: { deep: [{ client: 'agy-gemini-flash-high', role: 'spec-redteam', lenses: ['integration'], max_price_usd: 0.5, time_limit_s: 600 }] } } })}\n` } }),
  { ...budget({ repo: R('b2'), draft: 'x' }), remove: [OVERRIDE] },
  budget({ repo: R('b1') }),
  budget('[]'),
  budget({ repo: R('b1'), spec: '#9' }),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
