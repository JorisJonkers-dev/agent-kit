#!/usr/bin/env node
// Writes the scenario.json files of session-start and the critique-session-start hook (the goldens are expected.json,
// from regenerate-fixtures.mjs). The hook reads git and local files only, so no program is faked.
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const SB = '@SANDBOX@'
const KEY = 'github.com/jorisjonkers-dev/example'
const R = (dir) => `${SB}/repos/${dir}`
const start = (payload, extra = {}) => ({ argv: ['session-start', '--json', typeof payload === 'string' ? payload : JSON.stringify(payload)], shims: ['git'], ...extra })
const hook = (stdin, extra = {}) => ({ argv: ['hook', 'critique-session-start'], stdin: typeof stdin === 'string' ? stdin : JSON.stringify(stdin), shims: ['git'], ...extra })
const CONFIG = 'home/.config/agent-kit/critique.json'
const ALLOWLIST = 'home/.config/agent-kit/critique-allowlist.json'
const RESULTS = 'home/.local/share/agent-kit/critique/machine-setup.json'
const OVERRIDE = 'home/.config/agent-kit/critique-routing.json'
const LOG = ['home/.local/share/agent-kit/critique']
const allow = `${JSON.stringify({ repositories: { [KEY]: { families: ['mistral'], updated_at: '2026-11-01T00:00:00.000Z' } }, version: 1 })}\n`
const result = (outcome, inputs = {}) => ({ checked_at: '2026-11-01T00:00:00.000Z', detail: 'd', inputs, result: outcome })
const results = (checks) => `${JSON.stringify({ checks, version: 1 })}\n`
const ALL = ['vibe-signed-in', 'critic:vibe-mistral-large', 'critic:vibe-glm', 'critic:vibe-deepseek-flash', 'critic:vibe-mistral-small', 'agy-signed-in', 'critic:agy-gemini-flash-high', 'agy-quota', 'hooks-installed', 'kill-switch']
const every = (outcome) => Object.fromEntries(ALL.map((id) => [id, result(outcome)]))

const setup = {
  argv: ['--help'],
  write: { 'repos/s1/a.md': 'a\n', 'repos/s2/a.md': 'a\n', 'repos/s3/a.md': 'a\n' },
  git: [
    ['init', '-q', '--template=', 'repos/s1'], ['-C', 'repos/s1', 'remote', 'add', 'origin', 'git@github.com:JorisJonkers-dev/example.git'],
    ['init', '-q', '--template=', 'repos/s2'],
    ['init', '-q', '--template=', 'repos/s3'], ['-C', 'repos/s3', 'remote', 'add', 'origin', 'git@github.com:nedap/thing.git'],
  ],
}

const scenarios = {}
scenarios['session-start-decide'] = [
  setup,
  // Not opted in, no remote, a nedap remote.
  start({ cwd: R('s1') }),
  start({ cwd: R('s2') }),
  start({ cwd: R('s3') }),
  // Opted in: nothing checked yet, then some checks, a failure, several failures, everything current.
  start({ cwd: R('s1') }, { write: { [ALLOWLIST]: allow } }),
  start({ cwd: R('s1') }, { write: { [RESULTS]: results({ 'vibe-signed-in': result('pass'), 'read-only:claude': result('unproven'), 'other': result('fail') }) } }),
  start({ cwd: R('s1') }, { write: { [RESULTS]: results({ ...every('pass'), 'agy-quota': result('fail') }) } }),
  start({ cwd: R('s1') }, { write: { [RESULTS]: results({ ...every('fail'), 'vibe-signed-in': result('unproven') }) } }),
  start({ cwd: R('s1') }, { write: { [RESULTS]: results(every('unproven')) } }),
  // Routing with no Antigravity client has none of its checks.
  start({ cwd: R('s1') }, { write: { [OVERRIDE]: '{"clients":{"agy-gemini-flash-high":null},"stages":{}}\n' } }),
  start({ cwd: R('s1') }, { remove: [OVERRIDE] }),
  // The kill switch: the environment, then the configuration file.
  start({ cwd: R('s1') }, { env: { CRITIQUE_OFF: '1' } }),
  start({ cwd: R('s1') }, { env: { CRITIQUE_OFF: '0' }, write: { [CONFIG]: '{"disabled":true}\n' } }),
  start({ cwd: R('s1') }, { write: { [CONFIG]: '{"disabled":false}\n' } }),
]

scenarios['session-start-failed-open'] = [
  setup,
  start('not json', { roots: LOG }),
  start({ cwd: '' }, { roots: LOG }),
  start({ cwd: 5 }, { roots: LOG }),
  start({ cwd: R('s1') }, { write: { [CONFIG]: '{"disabled":"yes"}\n' }, roots: LOG }),
  start({ cwd: R('s1') }, { write: { [CONFIG]: '{nope' }, roots: LOG }),
  start({ cwd: R('s1') }, { remove: [CONFIG], write: { [ALLOWLIST]: '{"version":1}\n' }, roots: LOG }),
  start({ cwd: R('s1') }, { write: { [ALLOWLIST]: allow, [RESULTS]: '{"version":2,"checks":{}}\n' }, roots: LOG }),
  start({ cwd: R('s1') }, { write: { [RESULTS]: results({ x: { ...result('pass'), result: 'maybe' } }) }, roots: LOG }),
  start({ cwd: R('s1') }, { write: { [RESULTS]: results({ x: result('pass', { cli_version: null, agent_hash: 'h', routing_hash: 'r', files_hash: 5 }) }) }, roots: LOG }),
  start({ cwd: R('s1') }, { write: { [RESULTS]: results({ x: result('pass', { cli_version: null, agent_hash: null, routing_hash: 'r', files_hash: 'f' }) }) }, roots: LOG }),
  start({ cwd: `${SB}/nowhere` }, { roots: LOG }),
]

scenarios['session-start-hook'] = [
  setup,
  hook({ cwd: R('s1') }),
  hook({ cwd: R('s1') }, { write: { [ALLOWLIST]: allow } }),
  hook({ cwd: R('s1') }, { write: { [RESULTS]: results(every('pass')) } }),
  hook('garbage', { roots: LOG }),
  hook({ cwd: R('s1') }, { write: { [RESULTS]: results({ 'kill-switch': result('fail') }) } }),
  { argv: ['hook', 'critique-session-start', 'extra'], stdin: '{}' },
  { argv: ['hook', 'critique-nope'], stdin: '{}' },
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
