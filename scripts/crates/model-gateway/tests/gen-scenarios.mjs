#!/usr/bin/env node
// Writes the scenario.json files (the inputs; the goldens are expected.json, from regenerate-fixtures.mjs).
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { addClientScenarios } from './gen-scenarios-clients.mjs'
import { addReserveScenarios } from './gen-scenarios-reserve.mjs'
import { addWorktreeScenarios } from './gen-scenarios-worktrees.mjs'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const T = '2026-11-03T12:00:00Z'
const step = (argv, extra = {}) => ({ argv, ...extra })
const wf = (verb, input, extra) => step(['workflow', verb, '--json', typeof input === 'string' ? input : JSON.stringify(input)], extra)
const scenarios = {}

scenarios['cli-usage'] = [
  step(['--help']), step(['-h']),
  step([]), step(['bogus']), step(['workflow']), step(['workflow', 'bogus']), step(['grant']), step(['grant', 'bogus', '--json', '{}']),
  step(['worktrees']), step(['--json', '{}']),
  step(['workflow', 'list']), step(['workflow', 'list', '--json']), step(['workflow', 'list', '--json', '{}', 'extra']),
  step(['workflow', 'list', '--bogus', '{}']), step(['workflow', 'list', '--json', '{}', '--input', 'x']),
]

scenarios['cli-input'] = [
  wf('list', ''), wf('list', '{'), wf('list', '{"a":'), wf('list', '{"a" 1}'), wf('list', '[1]'), wf('list', 'null'), wf('list', '"text"'),
  wf('list', '5'), wf('list', 'nope'), wf('list', '{"a":1} x'), wf('list', "{'a':1}"), wf('list', '{"a":01}'), wf('list', '{"a":-}'),
  wf('list', '{"a":1.}'), wf('list', '{"a":1e}'), wf('list', '{"a":"\\x"}'), wf('list', '{"a":"line\nbreak"}'),
  wf('list', '{"a":[1,2,]}'), wf('list', '{"a":tru}'), wf('list', '{"an unexpectedly long key name here":1,}'),
  wf('list', '{"a":1}\n'), wf('list', '  {}  '),
  step(['workflow', 'list', '--input', 'missing.json']),
  step(['workflow', 'list', '--input', '@SANDBOX@/work'], { mkdir: ['work'] }),
  step(['workflow', 'list', '--input', 'in.json'], { write: { 'work/in.json': '{}' } }),
  step(['workflow', 'start', '--input', 'in.json'], { write: { 'work/in.json': '{"name":"from-file"}' } }),
  step(['workflow', 'start', '--input', '@SANDBOX@/work/in2.json'], { write: { 'work/in2.json': '{"name":"from-abs"}' } }),
  step(['workflow', 'start', '--input', 'bad.json'], { write: { 'work/bad.json': 'not json' } }),
]

scenarios['workflow-lifecycle'] = [
  wf('list', {}),
  wf('start', { name: 'spec-293' }),
  wf('start', { name: 'spec-293' }, { now: '2026-11-03T13:00:00Z' }),
  wf('start', { name: 'other:ticket/1.2_x' }, { now: '2026-11-03T13:30:00.250Z' }),
  wf('list', {}, { now: '2026-11-03T14:00:00Z' }),
  wf('end', { name: 'spec-293' }, { now: '2026-11-03T15:00:00Z' }),
  wf('end', { name: 'spec-293' }, { now: '2026-11-03T16:00:00Z' }),
  wf('list', { ignored: true }, { now: '2026-11-03T16:00:00Z' }),
  wf('start', { name: 'spec-293' }, { now: '2026-11-03T17:00:00Z' }),
  wf('end', { name: 'never-started' }),
  wf('end', { name: 'other:ticket/1.2_x' }, { now: '2026-11-03T18:00:00Z' }),
  wf('list', {}, { now: '2026-11-03T18:00:00Z' }),
]

scenarios['workflow-expiry'] = [
  wf('start', { name: 'old' }),
  wf('list', {}, { now: '2026-11-04T11:59:59.999Z' }),
  wf('list', {}, { now: '2026-11-04T12:00:00Z' }),
  wf('start', { name: 'old' }, { now: '2026-11-04T12:00:00Z' }),
  wf('end', { name: 'old' }, { now: '2026-11-04T12:00:01Z' }),
  wf('start', { name: 'fresh' }, { now: '2026-11-04T12:00:01Z' }),
  wf('end', { name: 'fresh' }, { now: '2026-11-05T12:00:01Z' }),
  wf('list', {}, { now: '2026-11-06T12:00:00Z' }),
]

const longName = 'a'.repeat(80)
scenarios['workflow-names'] = [
  wf('start', {}), wf('start', { name: '' }), wf('start', { name: 5 }), wf('start', { name: null }), wf('start', { name: ['a'] }),
  wf('start', { name: '-lead' }), wf('start', { name: '.lead' }), wf('start', { name: 'has space' }), wf('start', { name: 'semi;colon' }),
  wf('start', { name: 'newline\nname' }), wf('start', { name: 'quote"d' }), wf('start', { name: 'tab\tname' }), wf('start', { name: 'ünï' }),
  wf('start', { name: 'a'.repeat(81) }), wf('start', { name: longName }), wf('start', { name: 'A' }), wf('start', { name: '9' }),
  wf('end', {}), wf('end', { name: 'bad name' }), wf('end', { name: 'ünï' }), wf('end', { name: 'quote"d' }),
  wf('list', {}),
]

const file = (text) => ({ 'state/agent-kit/model-gateway/workflows.json': text })
const bad = (text, extra = {}) => wf('list', {}, { write: file(text), ...extra })
scenarios['workflow-state-files'] = [
  bad('not json'), bad(''), bad('[]'), bad('null'), bad('{}'), bad('{"version":2,"workflows":[]}'), bad('{"version":1}'),
  bad('{"version":1,"workflows":{}}'), bad('{"version":1,"workflows":[5]}'), bad('{"version":1,"workflows":[[]]}'),
  bad('{"version":1,"workflows":[{}]}'), bad('{"version":1,"workflows":[{"name":""}]}'),
  bad('{"version":1,"workflows":[{"name":"a"}]}'), bad('{"version":1,"workflows":[{"name":"a","started_at":"yesterday"}]}'),
  bad('{"version":1,"workflows":[{"name":"a","started_at":5}]}'),
  bad('{"version":1,"workflows":[{"name":"a","started_at":"2026-11-03T12:00:00Z","ended_at":"later"}]}'),
  bad('{"version":1,"workflows":[{"name":"a","started_at":"2026-11-03T12:00:00Z","ended_at":null}]}'),
  bad('{"version":1,"workflows":[{"name":"a","started_at":"2026-11-03T12:00:00Z"},{"name":"b","started_at":"2026-11-03T12:00:00Z","ended_at":"2026-11-03T13:00:00Z"}]}'),
  bad('{"version":1,"workflows":[{"name":"a","started_at":"2026-11-03T12:00:00+02:00"},{"name":"b","started_at":"2026-11-03"},{"name":"c","started_at":"2026-11-03T12:00"},{"name":"d","started_at":"2026-11-03T12:00:00.5Z"},{"name":"e","started_at":"+002026-11-03T12:00:00Z"}]}'),
  bad('{"version":1,"workflows":[{"name":"a","started_at":"2026"},{"name":"b","started_at":"2026-11"},{"name":"c","started_at":"2026-11-31"},{"name":"d","started_at":"2026-11-03T24:00"}]}', { now: '2026-12-01T12:00:00Z' }),
  bad('{"version":1,"workflows":[{"name":"a","started_at":"2026-11-32"}]}'), bad('{"version":1,"workflows":[{"name":"a","started_at":"2026-11-03T24:00:01"}]}'),
  wf('start', { name: 'new' }, { write: file('{"version":1,"workflows":[{"ended_at":"2026-11-03T13:00:00Z","name":"b","started_at":"2026-11-03T12:00:00Z"},{"name":"a","started_at":"2026-11-03T12:00:00Z"}],"extra":true}') }),
  wf('end', { name: 'a' }, { now: '2026-11-03T14:00:00Z' }),
  wf('end', { name: 'b' }),
  wf('start', { name: 'x' }, { write: file('{"version":1,"workflows":[{"name":"a","started_at":"2026-11-03T12:00:00Z"}]}'), mkdir: ['state/agent-kit/model-gateway/workflows.json.d'] }),
  wf('start', { name: 'x' }, { remove: ['state/agent-kit/model-gateway/workflows.json'], mkdir: ['state/agent-kit/model-gateway/workflows.json'] }),
  wf('list', {}),
]

scenarios['workflow-environment'] = [
  wf('start', { name: 'home-only' }, { env: { XDG_STATE_HOME: null } }),
  wf('list', {}, { env: { XDG_STATE_HOME: null } }),
  wf('start', { name: 'relative' }, { env: { XDG_STATE_HOME: 'relative/state' } }),
  wf('start', { name: 'empty' }, { env: { XDG_STATE_HOME: '' } }),
  wf('start', { name: 'no-home' }, { env: { XDG_STATE_HOME: null, HOME: null } }),
  wf('list', {}, { env: { HOME: null } }),
  wf('start', { name: 'elsewhere' }, { env: { XDG_STATE_HOME: '@SANDBOX@/elsewhere' } }),
  wf('list', {}, { env: { XDG_STATE_HOME: '@SANDBOX@/elsewhere' } }),
  wf('start', { name: 'blocked' }, { write: { 'blocker': 'x' }, env: { XDG_STATE_HOME: '@SANDBOX@/blocker' } }),
]

addClientScenarios(scenarios, step)
addReserveScenarios(scenarios, step)
addWorktreeScenarios(scenarios, step)

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
