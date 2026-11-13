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
const repo = (dir, remotes = []) => [['init', '-q', '--template=', `work/${dir}`], ...remotes.map(([name, url]) => ['-C', `work/${dir}`, 'remote', 'add', name, url])]
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
  step(['allow', '--input', '']), step(['allow', '--input', '', '--strict']),
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

// check-adrs. The repositories live under repos/, which the goldens do not snapshot (a commit makes git write binary objects).
const R = (dir) => `@SANDBOX@/repos/${dir}`
const adrs = (input, extra) => cmd('check-adrs', input, extra)
const init = (dir, remote = GITHUB) => [['init', '-q', '--template=', `repos/${dir}`], ['-C', `repos/${dir}`, 'remote', 'add', 'origin', remote]]
const commit = (dir) => [['-C', `repos/${dir}`, 'add', '-A'], ['-C', `repos/${dir}`, 'commit', '-q', '-m', 'x']]
/** Files under repos/<dir>/, then the repository made (when `fresh`) and everything committed. */
const files = (dir, text, { fresh = false, commits = true, git = [], ...extra } = {}) => ({
  write: Object.fromEntries(Object.entries(text).map(([path, body]) => [`repos/${dir}/${path}`, body])),
  git: [...(fresh ? init(dir) : []), ...(commits ? commit(dir) : []), ...git],
  ...extra,
})

scenarios['adrs-refusals'] = [
  adrs({}), adrs({ repo: '' }), adrs({ repo: 5 }), adrs([]), adrs('null'), adrs({ repo: R('none') }, { git: init('none').slice(0, 1) }),
  adrs({ repo: R('nedap') }, { git: init('nedap', 'git@github.com:nedap/secret.git') }),
  adrs({ repo: R('gone') }),
  adrs({ repo: R('bare') }, { git: init('bare') }),
  adrs({ repo: R('bare') }, { write: { 'repos/bare/docs/adr/notes.txt': 'x', 'repos/bare/docs/adr/md': 'x' } }),
  adrs({ repo: R('bare') }, { mkdir: ['repos/bare/docs/adr/dir.md'] }),
  adrs({ repo: R('bare') }, { remove: ['repos/bare/docs/adr'], write: { 'repos/bare/docs/adr': 'a file, not a directory' } }),
  adrs({ repo: R('bare') }, { remove: ['repos/bare/docs'] }),
  step(['check-adrs', '--json', JSON.stringify({ repo: R('none') }), '--strict']),
  step(['check-adrs', '--json', JSON.stringify({ repo: R('bare') }), '--strict']),
]

const adr = (n, body) => `# ${n}. Decision ${n}\n\nStatus: Accepted\n\n${body}\n`
scenarios['adrs-passed'] = [
  adrs({ repo: R('ok') }, files('ok', {
    'docs/adr/0001-first.md': adr(1, 'Uses `src/app.ts` and [the second](0002-second.md).'),
    'docs/adr/0002-second.md': adr(2, 'Follows ADR-0001. See [readme](../../README.md#intro).'),
    'docs/adr/0003-third.md': '---\nstatus: accepted\ndate: 2026-12-01\n---\n# Third\n\nNames `src/app.ts#fooBar`.\n',
    'src/app.ts': 'export function fooBar() {}\n',
    'README.md': '# Intro\n',
  }, { fresh: true })),
  adrs({ repo: R('ok/docs') }),
  adrs({ repo: R('ok') }, { write: { 'repos/ok/docs/adr/zzz.txt': 'not a record' } }),
]

scenarios['adrs-structure'] = [
  adrs({ repo: R('s') }, files('s', {
    'docs/adr/0001-a.md': adr(1, 'Cites [two](0002-b.md), [missing](0009-gone.md), [site](https://example.com/x.md), [anchor](#top), [mail](mailto:a@b.c), [abs](/docs/adr/0001-a.md), [out](../../../escape.md), `x`.'),
    'docs/adr/0002-b.md': '# 2. B\n\n## Status\n\n> Superseded by [0004](0004-d.md)\n\nBody.\n',
    'docs/adr/0002-b-again.md': adr(2, 'Duplicate number.'),
    'docs/adr/0004-d.md': adr(4, 'Supersedes ADR-0002; see also ADR 7 and adr9 and [ref][r].\n\n[r]: 0001-a.md "Title"\n[^1]: footnote.md\n[<angle>]: <0003-c.md>'),
    'docs/adr/0006-e.md': adr(6, 'Replaces [x](0001-a.md). Cites [two](0002-b.md); amended by ADR-0001. Unrelated [y](0004-d.md).'),
    'docs/adr/README.md': '# Index\n\nStatus: Weird\n',
    'docs/adr/2024-01-15-dated.md': '# Dated\n\nStatus: draft\n',
    'docs/adr/2024-1-5-short.md': '# Short dated\n',
    'docs/adr/0020-big.md': '# Big\n\nStatus: **Proposed**\n',
    'docs/adr/123456-too-big.md': '# Too big\n',
    'docs/adr/0010 spaced.md': '# Spaced\n\n- Status: deprecated\n',
    'docs/adr/0011_under.md': '#NoSpace\n',
  }, { fresh: true })),
]

const day = (date) => `\n\nDate: ${date}\n`
scenarios['adrs-drift'] = [
  adrs({ repo: R('d') }, files('d', {
    'docs/adr/0001-a.md': `# A${day('2026-10-01')}\nNames \`src/app.ts\`, \`src/app.ts#fooBar\`, \`src/gone.ts\`, \`./lib/\`, \`lib/x.json\`, \`missingSym()\`, \`hiddenSym()\`, \`fooBar()\`, \`src/app.ts#nope()\`, \`yes/no\`, \`max_tokens\`, \`a/../b.ts\`, \`@scope/pkg.json\`, \`Docs/Guide.MD\`, \`docs/adr/0002-b.md\`, \`#frag\`, \`two words.ts\`, \`only#hash\`, \`src/app.ts#\`.\n`,
    'docs/adr/0002-b.md': `# B\n\n**Date**: 2026-12-31\n\nNames \`src/app.ts\` and \`other.ts\`.\n`,
    'docs/adr/0003-c.md': `# C\n\n- date: 2026-11-03\n\nNames \`src/app.ts\`.\n`,
    'docs/adr/0004-d.md': `---\ndate: '2026-09-01'\n---\n# D\n\nNames \`src/app.ts\`.\n`,
    'docs/adr/0005-e.md': `# E\n\n**Date:** 2026-08-01\n\nNames \`src/app.ts\` and \`lib/\`.\n`,
    'docs/adr/0006-f.md': `# F\n\n__Date__ 2026-07-01\n\nNames \`src/app.ts\`.\n`,
    'docs/adr/0007-g.md': `# G\n\nDate: 2026-6-1\n\nNames \`src/app.ts\`.\n`,
    'docs/adr/0008-h.md': `# H\n\nStatus: Proposed${day('2020-01-01')}\nNames \`src/app.ts\` and \`src/gone.ts\`.\n`,
    'src/app.ts': 'export function fooBar() { return 9hiddenSym }\n',
    'lib/x.json': '{}\n',
    'other.ts': 'export const other = 1\n',
    'node_modules/pkg/index.js': 'const missingSym = 1\n',
    'docs/adr/notes.txt': 'missingSym\n',
  }, { fresh: true })),
  // A symbol in an untracked file counts, one only in the log or in node_modules does not; a deleted tracked file holds none.
  adrs({ repo: R('d') }, { write: { 'repos/d/untracked/new.js': 'const missingSym = 2\n' }, remove: ['repos/d/lib/x.json'] }),
  adrs({ repo: R('d') }, { write: { 'repos/d/.gitignore': 'untracked/\n' } }),
  // A tracked file that is now a directory cannot be read: the symbol search is incomplete, so every symbol counts as found.
  adrs({ repo: R('d') }, { remove: ['repos/d/src/app.ts'], mkdir: ['repos/d/src/app.ts'] }),
  // A path the working tree holds but no commit touched is not stale.
  adrs({ repo: R('d') }, { write: { 'repos/d/docs/adr/0009-new.md': '# N\n\nDate: 2026-01-01\n\nNames `fresh.ts`.\n', 'repos/d/fresh.ts': 'x\n' } }),
]

scenarios['adrs-nocommit'] = [
  adrs({ repo: R('n') }, files('n', {
    'docs/adr/0001-a.md': adr(1, 'Names `src/gone.ts` and `missingSym()`.'),
  }, { fresh: true, commits: false })),
  adrs({ repo: R('n') }, files('n', {})),
  adrs({ repo: R('n') }, { remove: ['repos/n/docs/adr/0001-a.md'], write: { 'repos/n/docs/adr/0001-a.md': adr(1, 'Names `src/gone.ts`, `missingSym()`, `docs/adr/0001-a.md`.') } }),
]

scenarios['adrs-edges'] = [
  adrs({ repo: R('e') }, files('e', {
    'docs/adr/0001-a.md': '# A\n\nSee [a](b.md c) [b](<b.md) [c](../..) [d](../../) [e](../../sub/) [f](.) [g](/) supersededby [x](0002-b.md).\n[abc\ndef]:\n[x]: \n[y]:\n\n  [z]:<\n',
    'docs/adr/0002-b.md': '---\nstatus: rejected\n# Unclosed frontmatter\n\nDate 2026-05-05\n',
    'docs/adr/0003-c.md': '#  \n\nbody\n',
    'docs/adr/0004-d.md': '# D\n\n`` `src/a.ts` `` and `open and `\nx` `\n',
    'docs/adr/0005-e.md': '#\t\n#\n#x\n',
    'docs/adr/0006-f.md': '# F\n\nDate 2026-05-05\n',
  }, { fresh: true })),
]

scenarios['adrs-paths'] = [
  adrs({ repo: R('p') }, files('p', {
    'docs/adr/0001-a.md': '# A\n',
    'docs/adr/0002-b.md': `Supersedes [first](//docs/adr/0001-a.md). Glossary: [terms](/CONTEXT.md/), [ok](/CONTEXT.md), [dir](/docs/) [dir2](/docs//), [long](${'a'.repeat(300)}.md). Names \`${'b'.repeat(300)}.md\` and \`CONTEXT.md/\`.\n`,
    'docs/adr/0003-c.md': '# C\n\nSee [root](/) and [up](/../x.md) and [double](//) [d](//docs/) [e](//../x.md).\n',
    'CONTEXT.md': '# Context\n',
  }, { fresh: true })),
]

scenarios['adrs-unnumbered'] = [
  adrs({ repo: R('u') }, files('u', { 'docs/adr/README.md': '# Index\n' }, { fresh: true })),
]

scenarios['adrs-markdown'] = [
  adrs({ repo: R('m') }, files('m', {
    'docs/adr/0001-frontmatter.md': '---\nstatus: "Rejected"\ndate: 2026-01-01\n---\n# Front\n\nNames `src/a.ts`.\n',
    'docs/adr/0002-plain.md': '# Plain\n\nstatus : proposed\n\nNames `src/a.ts`.\n',
    'docs/adr/0003-list.md': '# List\n\n* **Status**: Superseded\n\nSee `src/a.ts`.\n',
    'docs/adr/0004-under.md': '# Under\n\n__Status__: Deprecated\n',
    'docs/adr/0005-heading.md': '# Heading\n\n### Status\n\n\n  > *Accepted* by the owner\n',
    'docs/adr/0006-fenced.md': '# Fenced\n\n```\nStatus: rejected\nDate: 2020-01-01\n```\n\n~~~md\nStatus: proposed\n~~~\n\nNames `src/a.ts` but not `src/b.ts` inside:\n\n```\n`src/inside.ts` and [l](0099-x.md)\n```\n',
    'docs/adr/0007-unclosed.md': '# Unclosed\n\n```\nStatus: rejected\n\nNames `src/a.ts`.\n',
    'docs/adr/0008-crlf.md': '# CRLF title\r\n\r\nStatus: accepted\r\nDate: 2020-02-02\r\n\r\nNames `src/a.ts`.\r\n```\r\nfenced\r\n```\r\n',
    'docs/adr/0009-separators.md': '# Sep title\n\nStatus: rejected \nStatus: accepted\n\nSee [a](0001-frontmatter.md) and [b](0002-plain.md).\n',
    'docs/adr/0010-odd-fences.md': '# Odd\n\n````\nStatus: rejected\n````\n\n~~~\nStatus: deprecated\n```\n~~~\n\n   ```\nStatus: proposed\n```\n',
    'docs/adr/0011-clauses.md': '# Clauses\n\nSupersedes [a](0003-list.md); superseded by [b](0012-more.md). Replaced by ADR-0013. See [c](0004-under.md).\n',
    'docs/adr/0012-more.md': '# More\n\nAmends ADR 1. Replaces [x](0002-plain.md) and\n[y](0005-heading.md). Cites ADR-0099 and https://example.com/adr/ADR-0004.md and ADR-123456.\n',
    'docs/adr/0013-last.md': '# Last\n\nStatus: Accepted\n\nSee [one](0001-frontmatter.md) [two](./0002-plain.md "title") [three](<0003-list.md>) [four](0004-under.md#x).\n[five]: 0005-heading.md\n   [six]: <0006-fenced.md>\n[^7]: 0007-unclosed.md\nADR--5 ADR5 xADR-0001 ADR-0001x\n',
    'src/a.ts': 'const a = 1\n',
  }, { fresh: true })),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
