#!/usr/bin/env node
// Writes the scenario.json files (the inputs; the goldens are expected.json, from regenerate-fixtures.mjs).
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const step = (argv, extra = {}) => ({ argv, ...extra })
const cmd = (name, input, extra) => step([name, '--json', typeof input === 'string' ? input : JSON.stringify(input)], extra)
const scenarios = {}

/** A repository at work/<dir> whose remotes are the given [name, url] pairs. */
const repo = (dir, remotes = []) => [['init', '-q', `work/${dir}`], ...remotes.map(([name, url]) => ['-C', `work/${dir}`, 'remote', 'add', name, url])]
const at = (dir) => `@SANDBOX@/work/${dir}`

scenarios['cli-usage'] = [
  step(['--help']), step(['-h']), step([]), step(['bogus']), step(['allow', 'bogus']), step(['allow', '--json']), step(['allow', '--input']),
  step(['allow', '--json', '{}', 'extra']), step(['allow', '--bogus', '{}']), step(['--json', '{}']), step(['Allow', '--json', '{}']),
  step(['constructor', '--json', '{}']), step(['toString']), step(['__proto__']),
  step(['hook']), step(['hook', 'bogus']), step(['hook', 'critique-commit-gate', 'extra']), step(['hook', 'constructor']),
  step(['--strict']), step(['bogus', '--strict']), step(['allow', '--json', '{}', '--strict', 'x']),
]

scenarios['cli-input'] = [
  cmd('allow', ''), cmd('allow', '{'), cmd('allow', '{"a":'), cmd('allow', '{"a" 1}'), cmd('allow', 'nope'), cmd('allow', '{"a":1} x'),
  cmd('allow', "{'a':1}"), cmd('allow', '{"a":01}'), cmd('allow', '{"a":-}'), cmd('allow', '{"a":1.}'), cmd('allow', '{"a":1e}'),
  cmd('allow', '{"a":"\\x"}'), cmd('allow', '{"a":"line\nbreak"}'), cmd('allow', '{"a":[1,2,]}'), cmd('allow', '{"a":tru}'),
  cmd('allow', '[1]'), cmd('allow', 'null'), cmd('allow', '"text"'), cmd('allow', '5'), cmd('allow', 'true'), cmd('allow', '[]'),
  step(['allow']), step(['allow'], { stdin: '{"families":5}' }), step(['allow'], { stdin: '[1]' }), step(['allow'], { stdin: 'not json' }),
  step(['allow', '--input', 'missing.json']),
  step(['allow', '--input', '@SANDBOX@/work'], { mkdir: ['work'] }),
  step(['allow', '--input', 'in.json'], { write: { 'work/in.json': 'not json' } }),
  step(['allow', '--input', 'in.json'], { write: { 'work/in.json': '{}' } }),
  step(['allow', '--input', '@SANDBOX@/work/in2.json'], { write: { 'work/in2.json': '[1]' } }),
  step(['allow', '--json', '{}', '--strict']), step(['allow', '--strict', '--json', '{}']),
]

const GITHUB = 'git@github.com:JorisJonkers-dev/example.git'
const git = (dir, remotes) => ({ git: repo(dir, remotes), mkdir: ['work'] })
const allow = (input, extra) => cmd('allow', input, extra)

scenarios['allow-basic'] = [
  allow({ repo: at('r1'), families: ['deepseek'] }, git('r1', [['origin', GITHUB]])),
  allow({ repo: at('r1'), families: ['deepseek', 'zhipu'], code: true }),
  allow({ repo: at('r1'), families: ['mistral'] }, { now: '2026-11-03T13:00:00.250Z' }),
  allow({ repo: at('r1'), families: ['mistral'], history: true }),
  allow({ repo: at('r1'), families: [], code: false, history: false }),
  allow({ repo: at('r1'), families: ['google', 'anthropic', 'openai', 'google'] }),
  allow({ repo: at('r2'), families: ['zhipu'], code: true, history: true }, git('r2', [['origin', 'https://github.com/JorisJonkers-dev/second.git']])),
  allow({ repo: at('r2'), families: ['zhipu'], extra: 'ignored' }),
]

scenarios['allow-remotes'] = [
  allow({ repo: at('none'), families: ['zhipu'] }, git('none', [])),
  allow({ repo: at('nedap'), families: ['zhipu'] }, git('nedap', [['origin', 'git@github.com:nedap/secret.git']])),
  allow({ repo: at('mixed'), families: ['zhipu'] }, git('mixed', [['origin', GITHUB], ['work', 'https://github.com/nedap/other.git']])),
  allow({ repo: at('alias'), families: ['zhipu'] }, git('alias', [['origin', 'git@github-nedap:nedap/secret.git']])),
  allow({ repo: at('upstream'), families: ['zhipu'] }, git('upstream', [['upstream', 'https://gitlab.com/group/proj.git'], ['fork', GITHUB]])),
  allow({ repo: at('ssh'), families: ['zhipu'] }, git('ssh', [['origin', 'ssh://git@example.com:2222/team/proj.git']])),
  allow({ repo: at('plain'), families: ['zhipu'] }, git('plain', [['origin', 'not a url']])),
  allow({ repo: at('missing'), families: ['zhipu'] }),
  step(['allow', '--json', JSON.stringify({ repo: at('none'), families: ['zhipu'] }), '--strict']),
  step(['allow', '--json', JSON.stringify({ repo: at('nedap'), families: ['zhipu'] }), '--strict']),
  step(['allow', '--json', JSON.stringify({ repo: at('upstream'), families: ['zhipu'] }), '--strict']),
  step(['allow', '--strict', '--json', JSON.stringify({ repo: at('none'), families: ['zhipu'] })]),
  step(['allow', '--json', JSON.stringify({ repo: at('none'), families: ['zhipu'] }), '--strict', '--strict']),
  allow({ repo: at('r1/sub'), families: ['zhipu'] }, { ...git('r1', [['origin', GITHUB]]), mkdir: ['work/r1/sub'] }),
]

scenarios['allow-input'] = [
  allow({}), allow({ repo: at('r1') }), allow({ families: ['zhipu'] }), allow({ repo: '', families: [] }), allow({ repo: '  ', families: [] }),
  allow({ repo: 5, families: [] }), allow({ repo: at('r1'), families: 'zhipu' }), allow({ repo: at('r1'), families: null }),
  allow({ repo: at('r1'), families: ['bogus'] }), allow({ repo: at('r1'), families: ['zhipu', 5] }), allow({ repo: at('r1'), families: [['zhipu']] }),
  allow({ repo: at('r1'), families: [], code: 'yes' }), allow({ repo: at('r1'), families: [], code: null }), allow({ repo: at('r1'), families: [], history: 1 }),
  allow({ repo: at('r1'), families: [], code: false, history: 'no' }),
  allow({ repo: at('r1'), families: [], code: undefined }),
  allow('{"repo":"x","repo":"y","families":[]}'),
  allow('{"__proto__":{"a":1},"repo":"x","families":[]}'),
  allow({ repo: at('r1'), families: ['zhipu'] }, git('r1', [['origin', GITHUB]])),
]

const FILE = 'home/.config/agent-kit/critique-allowlist.json'
const withFile = (text, extra = {}) => ({ write: { [FILE]: text }, ...extra })
const entry = (extra = '') => `{"families":["zhipu"],"code":true,"history":false,"updated_at":"2026-11-01T00:00:00.000Z"${extra}}`
scenarios['allow-files'] = [
  allow({ repo: at('r1'), families: ['deepseek'] }, { ...git('r1', [['origin', GITHUB]]), ...withFile('not json') }),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('[]')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('null')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":2,"repositories":{}}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1,"repositories":[]}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1,"repositories":{"a":5}}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1,"repositories":{"a":{}}}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1,"repositories":{"a":{"families":["bogus"],"updated_at":"x"}}}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1,"repositories":{"a":{"families":[],"updated_at":""}}}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1,"repositories":{"a":{"families":[],"updated_at":"x","code":"y"}}}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1,"repositories":{"a":{"families":[],"updated_at":"x","history":2}}}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1,"repositories":{"a":{"families":["zhipu"],"updated_at":"x"}}}')),
  // The entry already there keeps its place and the permission the call leaves out; the others keep theirs.
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile(`{"version":1,"repositories":{"github.com/jorisjonkers-dev/before":${entry()},"github.com/jorisjonkers-dev/example":${entry()},"github.com/jorisjonkers-dev/after":${entry()}},"extra":true}`)),
  allow({ repo: at('r1'), families: ['deepseek'], code: false }, withFile(`{"version":1,"repositories":{"github.com/jorisjonkers-dev/example":${entry(',"unknown":1')}}}`)),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1,"repositories":{"github.com/jorisjonkers-dev/example":{"families":["zhipu"],"updated_at":"x"}}}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1,"repositories":{"7":{"families":[],"updated_at":"x"},"b":{"families":[],"updated_at":"x"},"3":{"families":[],"updated_at":"y"}}}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, withFile('{"version":1.0,"repositories":{}}')),
  allow({ repo: at('r1'), families: ['deepseek'] }, { mkdir: [FILE], remove: [FILE] }),
  allow({ repo: at('r1'), families: ['deepseek'] }, { remove: [FILE], mkdir: [FILE] }),
]

scenarios['allow-environment'] = [
  allow({ repo: at('r1'), families: ['zhipu'] }, { ...git('r1', [['origin', GITHUB]]), env: { XDG_CONFIG_HOME: '@SANDBOX@/xdg' } }),
  allow({ repo: at('r1'), families: ['zhipu'] }, { env: { XDG_CONFIG_HOME: 'relative/dir' } }),
  allow({ repo: at('r1'), families: ['zhipu'] }, { env: { XDG_CONFIG_HOME: '' } }),
  allow({ repo: at('r1'), families: ['zhipu'] }, { env: { XDG_CONFIG_HOME: null } }),
  allow({ repo: at('r1'), families: ['zhipu'] }, { env: { HOME: null } }),
  allow({ repo: at('r1'), families: ['zhipu'] }, { env: { HOME: null, XDG_CONFIG_HOME: '@SANDBOX@/xdg2' } }),
  allow({ repo: at('r1'), families: ['zhipu'] }, { env: { XDG_CONFIG_HOME: '@SANDBOX@/blocker' }, write: { blocker: 'x' } }),
  allow({ repo: at('r1'), families: ['zhipu'] }, { path: ['nowhere'] }),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
