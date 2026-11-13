#!/usr/bin/env node
// Writes the scenario.json files of `facts` (the goldens are expected.json, from regenerate-fixtures.mjs).
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
// git is pinned: the steps run with a PATH that holds only a link to the host's git.
const PINNED = { path: ['bin'] }
const step = (argv, extra = {}) => ({ argv, ...PINNED, ...extra })
const facts = (input, extra) => step(['facts', '--json', typeof input === 'string' ? input : JSON.stringify(input)], extra)
const GITHUB = 'git@github.com:JorisJonkers-dev/example.git'
const R = (dir) => `@SANDBOX@/repos/${dir}`
const init = (dir, remote = GITHUB) => [['init', '-q', '--template=', `repos/${dir}`], ['-C', `repos/${dir}`, 'remote', 'add', 'origin', remote]]
const commit = (dir, date, message = 'x') => [['-C', `repos/${dir}`, 'add', '-A'], [`GIT_COMMITTER_DATE=${date}`, '-C', `repos/${dir}`, 'commit', '-q', '-m', message]]
const put = (dir, text) => Object.fromEntries(Object.entries(text).map(([path, body]) => [`repos/${dir}/${path}`, body]))
const scenarios = {}

const tree = {
  'src/a.ts': "import { b } from './b.js'\nimport x from 'pkg'\nexport * from \"./c\"\nconst d = require('./d')\nimport('./c')\n// import y from './nope'\n/* import z from './c'\n*/\nexport { q } from '../outside'\n",
  'src/b.ts': "import { a } from './a'\nimport './e'\n",
  'src/c.ts': "import d from './d.js'\n",
  'src/d.ts': "import c from './c.js'\nimport './a'\n",
  'src/e.ts': "export const e = 1\n",
  'src/dir/index.ts': "export {}\n",
  'src/user.ts': "import dir from './dir'\nimport {} from './missing'\n",
  'pkg/__init__.py': '',
  'pkg/m.py': 'from . import n\nfrom .n import f, g as h\nfrom ..top import z\nimport pkg.n, os\nimport pkg\n# import nope\n',
  'pkg/n.py': 'import pkg.m\n',
  'top.py': 'x = 1\n',
  'src/main/kotlin/a/B.kt': 'import a.C\nimport a.D\nimport kotlin.io.println\n',
  'src/main/kotlin/a/C.kt': 'import a.B\n',
  'src/main/kotlin/a/D.kt': '',
  'src/other/kotlin/a/D.kt': '',
  'cmd/main.go': 'package main\n\nimport "fmt"\nimport (\n\t"os"\n\tio "io"\n)\n\nvar s = "not/an/import"\n',
  'src/lib.rs': 'mod a;\npub mod b;\nuse crate::a;\nmod missing;\n',
  'src/a.rs': 'mod c;\nuse std::io;\n',
  'src/a/c.rs': 'use super::*;\n',
  'src/b/mod.rs': 'mod lib;\n',
  'README.md': '# readme\n',
  'weird name/é.ts': "import x from '../src/e.js'\n",
  'src/f*.ts': "import './e'\n",
}
const base = { repo: R('fx'), modules: Object.keys(tree), files: ['src/a.ts', 'README.md', 'missing.txt', 'src', 'src/*.ts'] }
scenarios['facts-history'] = [
  facts({ repo: R('fx'), files: ['src/a.ts', 'src/b.ts'], modules: [] }, { shims: ['git'], write: put('fx', tree), git: [...init('fx'), ...commit('fx', '2026-01-01T10:00:00Z', 'one')] }),
  facts({ repo: R('fx'), files: ['src/a.ts', 'src/b.ts'], modules: ['src/a.ts'] }, { write: put('fx', { 'src/a.ts': tree['src/a.ts'] + 'export const second = 2\n' }), git: commit('fx', '2026-02-03T04:05:06+0200', 'two') }),
  facts({ repo: R('fx'), files: ['src/b.ts', 'src/a.ts', 'src/a.ts', 'nothing.ts', 'src', 'src/*.ts', 'src/f*.ts'], modules: [] }, { write: put('fx', { 'src/b.ts': tree['src/b.ts'] + '\n', 'src/a.ts': tree['src/a.ts'] + 'export const third = 3\n' }), git: commit('fx', '2026-03-04T05:06:07-0530', 'three') }),
  facts({ repo: R('fx'), files: ['src/a.ts'], modules: [] }, { write: put('fx', { 'src/b.ts': 'changed\n' }), git: [['-C', 'repos/fx', 'rm', '-q', '--cached', 'src/e.ts']] }),
  facts({ repo: R('fx'), files: ['../outside', '/etc/hostname'] }),
  facts({ repo: R('fx'), files: ['src/a.ts'] }),
  facts({ repo: R('fx') }),
  facts({ repo: R('fx'), files: [], modules: [] }),
  facts({ repo: R('fx'), files: ['é.ts', 'weird name/é.ts', '😀', 'z', 'a', '￿', '\u{10000}'] }),
]
scenarios['facts-modules'] = [
  facts(base, { shims: ['git'], write: put('fx', tree), git: [...init('fx'), ...commit('fx', '2026-01-01T10:00:00Z', 'one')] }),
  facts({ repo: R('fx'), modules: ['src/a.ts', 'src/b.ts', 'src/c.ts', 'src/d.ts'] }),
  facts({ repo: R('fx'), modules: ['src/a.ts'] }),
  facts({ repo: R('fx'), modules: ['src/e.ts', 'src/user.ts', 'src/dir/index.ts'] }),
  facts({ repo: R('fx'), modules: ['pkg/m.py', 'pkg/n.py', 'pkg/__init__.py', 'top.py'] }),
  facts({ repo: R('fx'), modules: ['src/main/kotlin/a/B.kt', 'src/main/kotlin/a/C.kt', 'src/main/kotlin/a/D.kt', 'src/other/kotlin/a/D.kt'] }),
  facts({ repo: R('fx'), modules: ['cmd/main.go', 'src/lib.rs', 'src/a.rs', 'src/a/c.rs', 'src/b/mod.rs'] }),
  facts({ repo: R('fx'), modules: ['README.md', 'nothing.ts', 'nothing', 'src/nothing.py', 'weird name/é.ts', 'src/f*.ts', 'src/e.ts'] }),
  facts({ repo: R('fx'), modules: ['src/b.ts', 'src/a.ts', 'src/b.ts', 'src/a.ts'], files: ['src/b.ts', 'src/a.ts'] }),
  facts({ repo: R('fx'), modules: ['src/\u{10000}.ts', 'src/￿.ts', 'src/a.ts'] }),
]
scenarios['facts-refusals'] = [
  facts({ repo: R('fresh') }, { shims: ['git'], git: init('fresh') }),
  facts({ repo: R('fresh'), files: ['a'], modules: ['b'] }),
  facts({ repo: R('none') }, { git: init('none').slice(0, 1) }),
  facts({ repo: R('nedap') }, { git: init('nedap', 'git@github.com:nedap/secret.git') }),
  facts({ repo: R('gone') }),
  step(['facts', '--json', JSON.stringify({ repo: R('none') }), '--strict']),
  step(['facts', '--json', JSON.stringify({ repo: R('fresh') }), '--strict']),
  step(['facts', '--json', JSON.stringify({ repo: R('fx') })], { path: [], remove: [] }),
]
scenarios['facts-input'] = [
  facts({}, { shims: ['git'] }), facts([]), facts('null'), facts('5'), facts('{"repo":'),
  facts({ repo: '' }), facts({ repo: '  ' }), facts({ repo: 5 }), facts({ repo: null }),
  facts({ repo: R('fx'), files: 'x' }), facts({ repo: R('fx'), files: [5] }), facts({ repo: R('fx'), files: null }),
  facts({ repo: R('fx'), modules: 'x' }), facts({ repo: R('fx'), modules: [null] }), facts({ repo: R('fx'), modules: {} }),
  facts({ repo: R('fx'), files: [''] , modules: [] }),
  facts({ repo: R('fx'), files: [], modules: [''] }),
  facts({ repo: R('fx'), files: [], modules: [], extra: 1 }),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
