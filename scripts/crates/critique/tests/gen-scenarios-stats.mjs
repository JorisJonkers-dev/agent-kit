#!/usr/bin/env node
// Writes the scenario.json files of stats (the goldens are expected.json, from regenerate-fixtures.mjs).
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const stats = (input, extra = {}) => ({ argv: ['stats', '--json', typeof input === 'string' ? input : JSON.stringify(input)], ...extra })
const SB = '@SANDBOX@'
const GITHUB = 'git@github.com:JorisJonkers-dev/example.git'
const KEY = 'github.com/jorisjonkers-dev/example'
const init = (dir, remote = GITHUB) => [['init', '-q', '--template=', `repos/${dir}`], ['-C', `repos/${dir}`, 'remote', 'add', 'origin', remote]]

const OWN = 'home/.local/share/agent-kit/critique/ledger.jsonl'
const GATEWAY = 'state/agent-kit/model-gateway/ledger.jsonl'
const OVERRIDE = 'home/.config/agent-kit/critique-routing.json'
const lines = (...objects) => `${objects.map((o) => (typeof o === 'string' ? o : JSON.stringify(o))).join('\n')}\n`
const context = (run_id, stage, extra = {}) => ({ recorded_at: '2026-11-02T10:00:00.000Z', repository: KEY, run_id, stage, ...extra })
const runLine = (run_id, stage, critic_id, client, extra = {}) => ({ type: 'run', ...context(run_id, stage), client, cost_usd: 0.1, critic_id, failure: null, finding_count: 1, lens_hashes: {}, lenses: [], model: 'm', prompt_hashes: { contract: 'c', preamble: 'p', role: 'r', schema: 's' }, role: 'r', status: 'ok', ...extra })
const finding = (run_id, stage, verdict, raised_by, extra = {}) => ({ type: 'finding', ...context(run_id, stage), finding_id: `${run_id}-${verdict}-${raised_by.join('-')}`, pre_verdict: null, raised_by, severity: 'major', verdict, ...extra })
const scenarios = {}

// What the input may be: dates, the two ways to name a repository, and refused repositories.
scenarios['stats-input'] = [
  stats('nope'), stats([]), stats('null'),
  stats({ repo: 'a', repository: 'b' }),
  stats({ since: '2026-11-1' }), stats({ since: '2026-02-30' }), stats({ until: '2026-13-01' }), stats({ since: 5 }),
  stats({ since: '2026-11-05', until: '2026-11-01' }),
  stats({ since: '2024-02-29', until: '2024-02-29' }),
  stats({ repository: 7 }), stats({ repo: '' }),
  stats({ repo: `${SB}/repos/gone` }),
  stats({ repo: `${SB}/repos/nedap` }, { git: init('nedap', 'git@github.com:nedap/secret.git') }),
  { argv: ['stats', '--json', '{}', '--strict'] },
]

// No ledger at all, and empty or blank ledgers: an empty report.
scenarios['stats-empty'] = [
  stats({}),
  stats({}, { write: { [OWN]: '', [GATEWAY]: '\n\n   \n' } }),
]

// The figures: runs and findings over clients and stages, both ledgers, the repository and date filters, and lines
// that are not run or finding records.
const runs = []
const findings = []
for (const [i, client] of ['vibe', 'agy', 'codex', 'claude'].entries()) {
  for (let n = 0; n < 6 + i * 4; n += 1) {
    const id = `r${client}${n}`
    const stage = n % 3 === 0 ? 'adr-review' : n % 3 === 1 ? 'spec' : 'tickets'
    runs.push(runLine(id, stage, `${client}-c`, client, n % 5 === 4 ? { status: 'failed', failure: 'timeout', finding_count: 0 } : {}))
    findings.push(finding(id, stage, n % 4 === 0 ? 'rejected' : n % 4 === 1 ? 'hint' : 'accepted', [`${client}-c`], { severity: n % 3 === 0 ? 'critical' : n % 3 === 1 ? 'minor' : 'major' }))
  }
}
const ledgerWrite = {
  [OWN]: lines(...runs.slice(0, 10), ...findings.slice(0, 10), '{"type":"other"}', runLine('o1', 'spec', 'x', 'vibe', { repository: 'github.com/other/repo' })),
  [GATEWAY]: lines(...runs.slice(10), ...findings.slice(10), finding('o1', 'spec', 'accepted', ['x'], { repository: 'github.com/other/repo' })),
}
scenarios['stats-figures'] = [
  stats({}, { write: ledgerWrite }),
  stats({ repository: KEY }),
  stats({ repository: 'github.com/other/repo' }),
  stats({ since: '2026-11-03' }),
  stats({ since: '2026-11-02', until: '2026-11-02' }),
  stats({ repo: `${SB}/repos/ex` }, { git: init('ex') }),
  stats({}, { write: { [OWN]: 'not json\n' } }),
  stats({}, { write: { [OWN]: '', [GATEWAY]: '[1]\n' } }),
]

// Routing: an override without stats thresholds is refused; one that changes the thresholds shows in the report.
scenarios['stats-routing'] = [
  stats({}, { write: { ...ledgerWrite, [OVERRIDE]: JSON.stringify({ stats: null }) } }),
  stats({}, { write: { [OVERRIDE]: JSON.stringify({ stats: { min_adjudicated: 2 } }) } }),
  stats({}, { write: { [OVERRIDE]: '{ not json' } }),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
