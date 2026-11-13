#!/usr/bin/env node
// Writes the scenario.json files of reviews (the goldens are expected.json, from regenerate-fixtures.mjs).
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const reviews = (input, extra = {}) => ({ argv: ['reviews', '--json', typeof input === 'string' ? input : JSON.stringify(input)], ...extra })
const SB = '@SANDBOX@'
const GITHUB = 'git@github.com:JorisJonkers-dev/example.git'
const KEY = 'github.com/jorisjonkers-dev/example'
const init = (dir, remote = GITHUB) => [['init', '-q', '--template=', `repos/${dir}`], ['-C', `repos/${dir}`, 'remote', 'add', 'origin', remote]]
const runDir = (repo, id) => `repos/${repo}/.git/critique/${id}`
const scenarios = {}

const LIMITS = { antigravity_quota: { daily_cap: 20, reserve_threshold: 10, reserved_stages: ['spec'] }, search: { max_bytes: 8192, max_matches: 50 } }
/** A run's manifest, as `prepare` writes one. */
const manifest = (repo, id, extra = {}) =>
  `${JSON.stringify({
    allowed_families: ['mistral'],
    artifact: { labels: [], reference: 'o/r#1', tracker: 'github' },
    created_at: '2026-10-30T12:00:00.000Z',
    critics: [],
    depth: 'base',
    depth_choice: { chosen_by: 'argument', measures: {}, signals: [] },
    distinct_families: true,
    dropped: [],
    flow: id,
    limits: LIMITS,
    orchestrator_family: 'anthropic',
    packet_dir: `${SB}/${runDir(repo, id)}/packet`,
    packet_files: [],
    repo_root: `${SB}/repos/${repo}`,
    repository: KEY,
    run_dir: `${SB}/${runDir(repo, id)}`,
    run_id: id,
    stage: 'adr-review',
    version: 1,
    ...extra,
  })}\n`
const runs = (repo, specs) => Object.fromEntries(Object.entries(specs).flatMap(([id, extra]) => [[`${runDir(repo, id)}/manifest.json`, manifest(repo, id, extra)], ...(extra?.report ? [[`${runDir(repo, id)}/report.md`, '# report\n']] : [])]))

const OWN = 'home/.local/share/agent-kit/critique/ledger.jsonl'
const GATEWAY = 'state/agent-kit/model-gateway/ledger.jsonl'
const lines = (...objects) => `${objects.map((o) => (typeof o === 'string' ? o : JSON.stringify(o))).join('\n')}\n`
const context = (run_id, stage, extra = {}) => ({ recorded_at: '2026-11-02T10:00:00.000Z', repository: KEY, run_id, stage, ...extra })
const runLine = (run_id, stage, critic_id, extra = {}) => ({ type: 'run', ...context(run_id, stage), client: 'vibe', cost_usd: 0.1, critic_id, failure: null, finding_count: 1, lens_hashes: {}, lenses: [], model: 'm', prompt_hashes: { contract: 'c', preamble: 'p', role: 'r', schema: 's' }, role: 'r', status: 'ok', ...extra })
const finding = (run_id, stage, verdict, extra = {}) => ({ type: 'finding', ...context(run_id, stage), finding_id: `${run_id}-${verdict}`, pre_verdict: null, raised_by: ['c1'], severity: 'major', verdict, ...extra })
const invocation = (run_id, kind, charged_usd, extra = {}) => ({ type: 'invocation', recorded_at: '2026-11-02T10:00:00.000Z', repository: KEY, flow: run_id ?? 'x', workflow: run_id ?? 'x', round: 1, stage: 'adr-review', kind, charged_usd, critic_id: 'c1', ...(run_id === null ? {} : { run_id }), ...extra })

// What the input may be, and the repositories a review is refused for.
scenarios['reviews-input'] = [
  reviews({}), reviews([]), reviews('null'), reviews('nope'), reviews({ repo: 5 }), reviews({ repo: '' }),
  reviews({ repo: `${SB}/repos/none`, limit: 0 }), reviews({ repo: `${SB}/repos/none`, limit: 1.5 }), reviews({ repo: `${SB}/repos/none`, limit: '3' }),
  reviews({ repo: `${SB}/repos/none`, limit: null }), reviews({ limit: 2 }),
  reviews({ repo: `${SB}/repos/none` }, { git: init('none').slice(0, 1) }),
  reviews({ repo: `${SB}/repos/nedap` }, { git: init('nedap', 'git@github.com:nedap/secret.git') }),
  reviews({ repo: `${SB}/repos/gone` }),
  reviews({ repo: `${SB}/repos/ok` }, { git: init('ok') }),
  reviews({ repo: `${SB}/repos/ok`, limit: 3 }),
  { argv: ['reviews', '--json', JSON.stringify({ repo: `${SB}/repos/none` }), '--strict'] },
]

// The list: directories and the ledger together, newest first, numeric run ids, the limit, the spend and the verdicts.
const ids = { 'r2': { stage: 'adr-review', round: 1 }, 'r10': { stage: 'adr-review', round: 2, previous_run_id: 'r2', report: true }, 'r9': { stage: 'decision-recovery' }, 'r1': { stage: 'architecture-review', report: true }, 'spec-run': { stage: 'spec' }, 'r09': { stage: 'adr-review' }, 'R3': { stage: 'adr-review' }, 'r-3': { stage: 'adr-review' }, 'r_3': { stage: 'decision-recovery' } }
scenarios['reviews-list'] = [
  reviews({ repo: `${SB}/repos/rv` }, {
    git: init('rv'),
    write: {
      ...runs('rv', ids),
      [OWN]: lines(
        runLine('r10', 'adr-review', 'c1'), runLine('r10', 'adr-review', 'c2'), runLine('r7', 'adr-review', 'c1', { round: 2 }), runLine('r1', 'decision-recovery', 'c1'), runLine('r5', 'plan', 'c1'),
        runLine('r8', 'architecture-review', 'c1', { repository: 'github.com/other/repo' }), '', '   ',
        finding('r10', 'adr-review', 'accepted'), finding('r10', 'adr-review', 'accepted', { finding_id: 'r10-a2' }), finding('r10', 'adr-review', 'rejected'), finding('r10', 'adr-review', 'hint'), finding('r10', 'adr-review', 'needs-human'),
        finding('r7', 'adr-review', 'accepted'), finding('r9', 'decision-recovery', 'hint'), finding('r2', 'adr-review', 'rejected', { repository: 'github.com/other/repo' }),
      ),
      [GATEWAY]: lines(
        invocation('r10', 'vibe', 0.1), invocation('r10', 'vibe', 0.2), invocation('r10', 'agy', 5), invocation('r10', 'vibe', 0.3, { repository: 'github.com/other/repo' }),
        invocation('r2', 'vibe', 0.7), invocation(null, 'vibe', 9), invocation('r9', 'codex', 1), invocation('r7', 'vibe', 1e-7), invocation('r7', 'vibe', 0.1),
      ),
    },
  }),
  reviews({ repo: `${SB}/repos/rv`, limit: 3 }),
  reviews({ repo: `${SB}/repos/rv`, limit: 1 }),
  reviews({ repo: `${SB}/repos/rv`, limit: 1000000 }),
  reviews({ repo: `${SB}/repos/rv/.git` }),
  reviews({ repo: `${SB}/repos/rv` }, { mkdir: [runDir('rv', 'nomanifest'), 'repos/rv/.git/critique/stray'], write: { 'repos/rv/.git/critique/file.txt': 'a file\n' } }),
]

// A repository with no runs, and one whose ledger or manifests cannot be read.
scenarios['reviews-empty'] = [
  reviews({ repo: `${SB}/repos/e` }, { git: init('e') }),
  reviews({ repo: `${SB}/repos/e` }, { write: { [OWN]: '' , [GATEWAY]: '\n\n' } }),
  reviews({ repo: `${SB}/repos/e` }, { write: { [OWN]: lines(runLine('r1', 'adr-review', 'c1')) } }),
  reviews({ repo: `${SB}/repos/e` }, { write: { [GATEWAY]: lines(invocation('r1', 'vibe', 0.5)) } }),
]
scenarios['reviews-errors'] = [
  reviews({ repo: `${SB}/repos/x` }, { git: init('x'), write: { [OWN]: 'not json\n' } }),
  reviews({ repo: `${SB}/repos/x` }, { remove: [OWN], write: { [GATEWAY]: 'not json\n[1]\n{"type":"invocation"}\n' , [`${runDir('x', 'bad')}/manifest.json`]: '{' } }),
  reviews({ repo: `${SB}/repos/x` }, { remove: [`${runDir('x', 'bad')}`], write: { [`${runDir('x', 'odd')}/manifest.json`]: '[1]\n', [`${runDir('x', 'good')}/manifest.json`]: manifest('x', 'good') } }),
  reviews({ repo: `${SB}/repos/x` }, { remove: [`${runDir('x', 'odd')}`], write: { [`${runDir('x', 'unreadable')}/manifest.json/keep`]: 'a directory where the manifest belongs\n' } }),
  reviews({ repo: `${SB}/repos/x` }, { remove: [`${runDir('x', 'unreadable')}`], mkdir: [OWN] }),
  reviews({ repo: `${SB}/repos/x` }, { remove: [OWN], env: { HOME: null } }),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
