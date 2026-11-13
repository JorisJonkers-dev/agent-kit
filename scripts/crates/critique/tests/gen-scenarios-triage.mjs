#!/usr/bin/env node
// Writes the scenario.json files of triage (the goldens are expected.json, from regenerate-fixtures.mjs).
// A scenario's `assets` flag makes the Rust replay read the node runner's real shipped folder, since the
// command reads prompts/triage.md from it. gh and vibe are fake programs in the sandbox's bin/.
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const SB = '@SANDBOX@'
const GITHUB = 'git@github.com:JorisJonkers-dev/example.git'
const KEY = 'github.com/jorisjonkers-dev/example'
const triage = (input, extra = {}) => ({ argv: ['triage', '--json', typeof input === 'string' ? input : JSON.stringify(input)], path: ['bin'], ...extra })
const R = (dir) => `${SB}/repos/${dir}`
const q = (text) => `'${text.split("'").join("'\\''")}'`
const scenarios = {}

const OVERRIDE = 'home/.config/agent-kit/critique-routing.json'
const ALLOWLIST = 'home/.config/agent-kit/critique-allowlist.json'
const allow = (families = ['mistral'], code = true) => ({ [ALLOWLIST]: `${JSON.stringify({ repositories: { [KEY]: { code, families, history: false, updated_at: '2026-11-01T00:00:00.000Z' } }, version: 1 })}\n` })
const HOME = { 'home/.vibe/.env': 'MISTRAL_API_KEY=sk-test\n', 'home/.vibe/agents/critic-mistral-small.toml': '# agent\n' }

/** Repositories made in two steps: every `base` is committed, then every `head` on top of it (a file with a null body is removed). */
const setup = (repos, remote = GITHUB) => {
  const dirs = Object.keys(repos)
  const files = (which) => Object.fromEntries(dirs.flatMap((dir) => Object.entries(repos[dir][which] ?? {}).filter(([, body]) => body !== null).map(([path, body]) => [`repos/${dir}/${path}`, body])))
  const removed = dirs.flatMap((dir) => Object.entries(repos[dir].head ?? {}).filter(([, body]) => body === null).map(([path]) => `repos/${dir}/${path}`))
  return [
    { argv: ['--help'], write: files('base'), git: dirs.flatMap((dir) => [['init', '-q', '--template=', `repos/${dir}`], ...(remote === null ? [] : [['-C', `repos/${dir}`, 'remote', 'add', 'origin', remote]]), ['-C', `repos/${dir}`, 'add', '-A'], ['-C', `repos/${dir}`, 'commit', '-q', '-m', 'base']]) },
    { argv: ['--help'], write: files('head'), remove: removed, git: dirs.flatMap((dir) => [['-C', `repos/${dir}`, 'add', '-A'], ['-C', `repos/${dir}`, 'commit', '-q', '-m', 'head']]) },
  ]
}
const roots = (...dirs) => dirs.map((dir) => `repos/${dir}/.git/critique`)

/** A fake `gh issue view`: the issue numbers it knows, each with what it prints, says on stderr and exits with. */
const issue = (number, labels = [], extra = {}) => ({ out: JSON.stringify({ number, title: `T${number}`, body: `body ${number}`, url: `https://github.com/jorisjonkers-dev/example/issues/${number}`, labels: labels.map((name) => ({ name })), ...extra }) })
const fakeGh = (cases) => `#!/bin/sh\ncase "$1 $2 $3" in\n${Object.entries(cases).map(([number, { out = '', err = '', code = 0 }]) => `"issue view ${number}")\nprintf '%s' ${q(out)}\nprintf '%s' ${q(err)} >&2\nexit ${code}\n;;`).join('\n')}\n*)\necho "gh: unexpected: $*" >&2\nexit 9\n;;\nesac\n`
const GH = { 7: issue(7), 8: issue(8, ['Risk:High', 'bug']), 9: issue(9, ['bug']), 10: issue(10, ['feature']) }
/** The fake programs a step puts in bin/ (git is the host's, linked in), and the files they need. */
const tools = ({ gh = null, vibe = null, files = {} } = {}) => ({
  write: { ...(gh === null ? {} : { 'bin/gh': fakeGh(gh) }), ...(vibe === null ? {} : { 'bin/vibe': fakeVibe(vibe), ...HOME }), ...files },
  exec: [...(gh === null ? [] : ['bin/gh']), ...(vibe === null ? [] : ['bin/vibe'])],
  shims: ['env', 'mkdir', 'git'],
})

const GOOD_EXPORT = { config: { enable_mcp: false, enabled_tools: ['read_file'], include_project_context: false, outside_workdir: 'deny' }, cost_usd: 0.002, usage: { input_tokens: 1200, cached_input_tokens: 200, output_tokens: 300 } }
const journal = (text) => JSON.stringify([{ type: 'message', role: 'user', content: [{ text: 'q' }] }, { type: 'message', role: 'assistant', content: [{ text }] }])
const outcome = ({ exported = GOOD_EXPORT, stdout = '', stderr = '', code = 0 }) =>
  `${exported === null ? '' : `printf '%s\\n' ${q(JSON.stringify(exported))} > "$out/export.json"\n`}printf '%s' ${q(stdout)}\nprintf '%s' ${q(stderr)} >&2\nexit ${code}`
const fakeVibe = (cases) =>
  `#!/bin/sh\nout=''\nwhile [ $# -gt 0 ]; do\n  if [ "$1" = --output-dir ]; then out="$2"; fi\n  shift\ndone\nmkdir -p "$out"\ncase "$out" in\n${Object.entries(cases)
    .map(([where, body]) => `*${where}/export)\n${body}\n;;`)
    .join('\n')}\n*)\nexit 9\n;;\nesac\n`
const says = (text, extra = {}) => outcome({ stdout: journal(text), ...extra })

// What the risk rules decide, from git and the ticket's labels alone: no model, no allowlist.
const BASE = { 'src/a.ts': 'export const a = 1\n', 'docs/readme.md': '# readme\n', 'old/x.ts': '// do not edit\nexport const x = 1\n', 'keep.txt': 'k\n' }
const MID = { 'docs/readme.md': '# readme\n\nmore\nlines\nhere\n' }
scenarios['triage-rules'] = [
  ...setup({
    t1: { base: BASE, head: MID },
    t2: { base: BASE, head: { ...MID, 'src/auth/login.ts': 'export const login = 1\n', 'package-lock.json': '{\n"a":1\n}\n', 'gen/out.ts': '// @generated\nexport const g = 1\n', 'old/x.ts': null, 'sp ace/f.sql': 'select 1;\n', 'src/a.ts': 'export const a = 2\n' } },
  }),
  triage({ repo: R('t2'), ticket: '#7', base: 'HEAD~1' }, { ...tools({ gh: GH }), roots: roots('t1', 't2') }),
  // The same moment again: the run id takes a count.
  triage({ repo: R('t2'), ticket: '#7', base: 'HEAD~1' }, { roots: roots('t2') }),
  triage({ repo: R('t2'), ticket: '#7', base: 'HEAD~1' }, { roots: roots('t2') }),
  // A label alone (any case), the flow from the spec, then from the input.
  triage({ repo: R('t1'), ticket: '#8', base: 'HEAD~1', spec: '#10' }, { roots: roots('t1') }),
  triage({ repo: R('t1'), ticket: '#8', base: 'HEAD~1', spec: '#10', flow: 'my-flow' }, { roots: roots('t1') }),
  // The size rule, and every rule at once, the deciding one first.
  triage({ repo: R('t1'), ticket: '#9', base: 'HEAD~1' }, { write: { [OVERRIDE]: '{"triage":{"changed_lines_threshold":2}}\n' }, roots: roots('t1') }),
  triage({ repo: R('t2'), ticket: '#8', base: 'HEAD~1', head: 'HEAD' }, { write: { [OVERRIDE]: '{"triage":{"changed_lines_threshold":2,"labels":["risk:high","bug"]}}\n' }, roots: roots('t2') }),
  // A threshold the change only reaches: not above it.
  triage({ repo: R('t1'), ticket: '#9', base: 'HEAD~1', head: 'HEAD' }, { write: { [OVERRIDE]: '{"triage":{"changed_lines_threshold":4,"risk_paths":[]}}\n' }, roots: roots('t1') }),
]

// What the triage model is asked, and what it may answer; one repository per case so each run is the first.
const CASES = {
  m1: ['yes', says('Yes.')],
  m2: ['no', says('no')],
  m3: ['markup', says('**No**\n')],
  m4: ['unclear', says('Maybe, I would look at it.')],
  m5: ['exit', outcome({ code: 3, stderr: 'boom\n', stdout: journal('yes') })],
  m6: ['empty', outcome({ stdout: '  \n' })],
  m7: ['unsafe', outcome({ exported: { ...GOOD_EXPORT, config: { ...GOOD_EXPORT.config, enable_mcp: true } }, stdout: journal('no') })],
  m8: ['noexport', outcome({ exported: null, stdout: journal('no') })],
  m9: ['quoted', says("  'YES!'  ", { exported: { ...GOOD_EXPORT, usage: undefined } })],
  m10: ['nocost', says('no', { exported: { config: GOOD_EXPORT.config, usage: { input_tokens: 0, output_tokens: 0 } } })],
}
const caseRepos = Object.fromEntries(Object.keys(CASES).map((dir) => [dir, { base: { 'src/a.ts': 'a\n', 'notes.md': 'n\n' }, head: { 'notes.md': 'n\nm\n' } }]))
scenarios['triage-model'] = [
  ...setup(caseRepos),
  ...Object.keys(CASES).map((dir, index) =>
    triage({ repo: R(dir), ticket: '#7', base: 'HEAD~1' }, {
      ...(index === 0 ? tools({ gh: GH, vibe: Object.fromEntries(Object.entries(CASES).map(([name, [, body]]) => [`/${name}/.git/critique/20261103T120000Z-triage`, body])), files: allow() }) : {}),
      roots: roots(dir),
    }),
  ),
]

// Why the model may not be asked: the allowlist, a plan limit the first run reaches, a client not ready.
const GATES = Object.fromEntries(['g1', 'g2', 'g3', 'g4', 'g5', 'g6', 'g7'].map((dir) => [dir, { base: { 'a.txt': 'a\n' }, head: { 'a.txt': 'a\nb\n' } }]))
const PLAN = '{"clients":{"vibe":{"plan_limits":[{"window":"day","limit_usd":0.0002}]}}}\n'
scenarios['triage-gates'] = [
  ...setup(GATES),
  triage({ repo: R('g1'), ticket: '#7', base: 'HEAD~1' }, { ...tools({ gh: GH }), roots: roots('g1') }),
  triage({ repo: R('g2'), ticket: '#7', base: 'HEAD~1' }, { write: allow(['mistral'], false), roots: roots('g2') }),
  triage({ repo: R('g3'), ticket: '#7', base: 'HEAD~1' }, { write: allow(['deepseek', 'zhipu']), roots: roots('g3') }),
  triage({ repo: R('g4'), ticket: '#7', base: 'HEAD~1' }, { write: allow(), roots: roots('g4') }),
  triage({ repo: R('g4'), ticket: '#7', base: 'HEAD~1' }, { ...tools({ vibe: {}, files: { 'home/.vibe/.env': 'MISTRAL_API_KEY=\n' } }), roots: roots('g4') }),
  triage({ repo: R('g5'), ticket: '#7', base: 'HEAD~1' }, { write: { 'home/.vibe/.env': 'MISTRAL_API_KEY=sk-test\n' }, remove: ['home/.vibe/agents/critic-mistral-small.toml'], roots: roots('g5') }),
  triage({ repo: R('g6'), ticket: '#7', base: 'HEAD~1', spec: '#10' }, { ...tools({ vibe: { '/g6/.git/critique/20261103T120000Z-triage': says('no') } }), roots: roots('g6') }),
  triage({ repo: R('g7'), ticket: '#7', base: 'HEAD~1', spec: '#10' }, { write: { 'home/.config/agent-kit/model-gateway.json': PLAN }, roots: roots('g7') }),
  triage({ repo: R('g7'), ticket: '#7', base: 'HEAD~1' }, { write: { 'home/.config/agent-kit/model-gateway.json': '{' }, roots: roots('g7') }),
]

// Input, routing, tracker and git errors, in the order the command meets them.
const ERR_REPO = { base: { 'a.txt': 'a\n', 'docs/spec.md': '# s\n' }, head: { 'a.txt': 'a\nb\n' } }
const ok = { repo: R('e1'), ticket: '#7', base: 'HEAD~1' }
const TRACKER_DOC = 'docs/agents/issue-tracker.md'
scenarios['triage-errors'] = [
  ...setup({ e1: ERR_REPO, e2: ERR_REPO, e3: ERR_REPO, e4: ERR_REPO, e5: ERR_REPO }),
  triage({}, tools({ gh: GH })),
  triage([]), triage('null'), triage({ repo: 'x' }), triage({ base: 'b' }), triage({ base: 'b', repo: 'x' }),
  triage({ base: 'b', repo: 'x', ticket: 't', flow: 5 }), triage({ base: 'b', repo: 'x', ticket: 't', head: 5 }), triage({ base: 'b', repo: 'x', ticket: 't', spec: '' }),
  triage({ base: 'b', repo: 'x', ticket: 't', staged: 'yes' }), triage({ base: 'b', repo: 'x', ticket: 't', staged: true, head: 'HEAD' }), triage({ base: 'b', repo: 'x', staged: true }),
  triage({ base: 'b', repo: 'x', ticket: 't', staged: false, head: 'HEAD' }),
  triage({ ...ok, repo: R('none') }, { git: [['init', '-q', '--template=', 'repos/none']] }),
  triage({ ...ok, repo: R('nedap') }, { git: [['init', '-q', '--template=', 'repos/nedap'], ['-C', 'repos/nedap', 'remote', 'add', 'origin', 'git@github.com:nedap/secret.git']] }),
  triage({ ...ok, repo: '' }), triage({ ...ok, repo: R('missing') }),
  triage(ok, { write: { [OVERRIDE]: '{"triage":null}\n' } }),
  triage(ok, { write: { [OVERRIDE]: '{"triage":{"model":{"client":"nope"}}}\n' } }),
  triage(ok, { write: { [OVERRIDE]: '{' } }),
  // The tracker doc.
  triage({ ...ok, repo: R('e2') }, { write: { ['repos/e2/' + TRACKER_DOC]: '# Issue tracker: GitLab\n' } }),
  triage({ ...ok, repo: R('e2') }, { write: { ['repos/e2/' + TRACKER_DOC]: 'no heading\n' } }),
  triage({ ...ok, repo: R('e2') }, { write: { ['repos/e2/' + TRACKER_DOC]: '# issue TRACKER:\n\n  GitHub \n' } }),
  triage({ ...ok, repo: R('e3'), ticket: 'docs/spec.md' }, { write: { ['repos/e3/' + TRACKER_DOC]: '# Issue tracker: Local markdown\n' }, roots: roots('e3') }),
  triage({ ...ok, repo: R('e3'), ticket: 'docs/spec.md', spec: 'docs/spec.md', flow: 'f' }, { roots: roots('e3') }),
  triage({ ...ok, repo: R('e3'), ticket: 'docs/missing.md' }),
  triage({ ...ok, repo: R('e3'), ticket: '../x.md' }),
  triage({ ...ok, repo: R('e3'), ticket: 'docs' }),
  triage({ ...ok, repo: R('e3'), ticket: 'docs/spec.md', spec: 'nope.md' }),
  // gh: references, failures, and what it may print.
  triage({ ...ok, ticket: 'banana' }), triage({ ...ok, ticket: 'other/repo#5' }), triage({ ...ok, ticket: 'github.com/JorisJonkers-dev/Example#7' }), triage({ ...ok, ticket: 'https://github.com/jorisjonkers-dev/example/issues/7' }),
  triage({ ...ok, ticket: '#20' }, tools({ gh: { ...GH, 20: { err: '\n  \nnot found: 20\nsecond line\n', code: 1 }, 21: { out: 'not json' }, 22: { out: '{}' }, 23: { out: '{"number":23,"title":"t","body":"b","url":"u","labels":"x"}' }, 24: { err: '', out: '\nstdout words\n', code: 2 }, 25: { code: 4 }, 26: issue(26, ['a'], { labels: [{ name: 'x' }, { nope: 1 }, 5, null] }) } })),
  triage({ ...ok, ticket: '#21' }), triage({ ...ok, ticket: '#22' }), triage({ ...ok, ticket: '#23' }), triage({ ...ok, ticket: '#24' }), triage({ ...ok, ticket: '#25' }),
  triage({ ...ok, ticket: '#7', spec: '#20' }), triage({ ...ok, ticket: '#26' }, { roots: roots('e1') }),
  // git: revisions that name no commit.
  triage({ ...ok, base: 'nope' }), triage({ ...ok, head: 'nope' }), triage({ ...ok, base: '--oops' }), triage({ ...ok, base: 'HEAD', head: 'HEAD' }, { roots: roots('e1') }),
  // A glob JavaScript cannot build a pattern from.
  triage({ ...ok, repo: R('e4') }, { write: { [OVERRIDE]: '{"triage":{"risk_paths":["{x"]}}\n' } }),
  triage({ ...ok, repo: R('e4') }, { write: { [OVERRIDE]: '{"packet":{"exclude":{"lock":["a{b"]}}}\n' } }),
  triage({ ...ok, repo: R('e5') }, { write: { [OVERRIDE]: '{"triage":{"risk_paths":["a.txt","{x"]}}\n' }, roots: roots('e5') }),
  // gh missing from PATH.
  triage({ ...ok, ticket: '#7' }, { remove: ['bin/gh'] }),
  step_strict(),
]
function step_strict() {
  return { argv: ['triage', '--json', JSON.stringify({ ...ok, repo: R('none') }), '--strict'], path: ['bin'] }
}

// The staged change: the index made into a commit on HEAD, and the triage index the commit gate reads.
const STAGED_REPO = { base: { 'a.txt': 'a\n', 'src/auth/x.ts': 'x\n' }, head: { 'a.txt': 'a\nb\n' } }
const DATES = { GIT_AUTHOR_DATE: '2026-11-03T12:00:00Z', GIT_COMMITTER_DATE: '2026-11-03T12:00:00Z' }
scenarios['triage-staged'] = [
  ...setup({ s1: STAGED_REPO, s2: STAGED_REPO, s3: STAGED_REPO }),
  { argv: ['--help'], write: { 'repos/s1/a.txt': 'a\nb\nc\n', 'repos/s2/src/auth/x.ts': 'x\ny\n', 'repos/s3/a.txt': 'z\n', 'repos/s3/untracked.txt': 'u\n' }, git: [['-C', 'repos/s1', 'add', 'a.txt'], ['-C', 'repos/s2', 'add', '-A'], ['-C', 'repos/s3', 'add', 'a.txt']] },
  triage({ repo: R('s1'), ticket: '#8', base: 'HEAD', staged: true }, { ...tools({ gh: GH }), env: DATES, roots: roots('s1') }),
  triage({ repo: R('s1'), ticket: '#8', base: 'HEAD', staged: true }, { env: DATES, roots: roots('s1') }),
  triage({ repo: R('s2'), ticket: '#7', base: 'HEAD', staged: true, spec: '#10' }, { env: DATES, roots: roots('s2') }),
  triage({ repo: R('s3'), ticket: '#7', base: 'HEAD', staged: true }, { ...tools({ vibe: { '/s3/.git/critique/20261103T120000Z-triage': says('no') }, files: allow() }), env: DATES, roots: roots('s3') }),
  // A repository with no commit: there is nothing to diff the index against.
  { argv: ['--help'], write: { 'repos/s4/a.txt': 'a\n' }, git: [['init', '-q', '--template=', 'repos/s4'], ['-C', 'repos/s4', 'remote', 'add', 'origin', GITHUB], ['-C', 'repos/s4', 'add', '-A']] },
  triage({ repo: R('s4'), ticket: '#7', base: 'HEAD', staged: true }, { env: DATES }),
  triage({ repo: R('s4'), ticket: '#7', base: 'HEAD' }),
]

// The paths a diff can name: a rename and a partial rename (read as a removal and an addition), a binary file,
// non-ASCII and C-quoted names (a tab, a quote, a backslash), and a removed lock file.
const PATHS_BASE = {
  'src/auth/old.ts': 'export const a = 1\nexport const b = 2\n', 'src/keep.ts': 'one\ntwo\nthree\nfour\n',
  'package-lock.json': '{"lockfileVersion": 3}\n', 'img.bin': '\u0000\u0001\u0002binary\n', 'ünï/é.sql': 'select 1;\n',
}
const PATHS_HEAD = {
  'src/auth/old.ts': null, 'src/auth/new.ts': 'export const a = 1\nexport const b = 2\n',
  'src/keep.ts': null, 'src/kept.ts': 'one\ntwo\nthree\nfive\n',
  'package-lock.json': null, 'img.bin': '\u0000\u0003\u0004binary2\n', 'ünï/é.sql': 'select 2;\n',
  'a\tb.sql': 'drop table x;\n', 'say "hi".md': 'hi\n', 'back\\slash.txt': 'x\n',
}
scenarios['triage-paths'] = [
  ...setup({ p1: { base: PATHS_BASE, head: PATHS_HEAD } }),
  triage({ repo: R('p1'), ticket: '#7', base: 'HEAD~1' }, { ...tools({ gh: GH, vibe: { '/p1/.git/critique/20261103T120000Z-triage': says('no') }, files: allow() }), env: DATES, roots: roots('p1') }),
  triage({ repo: R('p1'), ticket: '#8', base: 'HEAD~1' }, { env: DATES, roots: roots('p1') }),
]

// A step's routing override lasts only for the step: the sandbox keeps files, so every other triage step removes it first.
const fresh = (step) => (step.argv[0] === 'triage' && step.write?.[OVERRIDE] === undefined ? { ...step, remove: [...(step.remove ?? []), OVERRIDE] } : step)
for (const [name, steps] of Object.entries(scenarios)) {
  scenarios[name] = steps.map(fresh)
}
for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ assets: true, steps }, null, 2)}\n`)
}
