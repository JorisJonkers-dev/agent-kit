#!/usr/bin/env node
// Writes the scenario.json files of check-tickets (the goldens are expected.json, from regenerate-fixtures.mjs).
// gh is a fake program in the sandbox's bin/: `gh issue view` prints the spec, and `gh api` the REST lists of
// sub-issues and blockers by the path it is asked for.
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const SB = '@SANDBOX@'
const GITHUB = 'git@github.com:JorisJonkers-dev/example.git'
const URL = 'https://github.com/jorisjonkers-dev/example/issues'
const check = (input) => ({ argv: ['check-tickets', '--json', typeof input === 'string' ? input : JSON.stringify(input)], path: ['bin'] })
const R = (dir) => `${SB}/repos/${dir}`
const q = (text) => `'${text.split("'").join("'\\''")}'`
const scenarios = {}

/** Repositories committed once; `remote: null` leaves one with no remote. */
const setup = (repos) => {
  const dirs = Object.keys(repos)
  const write = Object.fromEntries(dirs.flatMap((dir) => Object.entries(repos[dir].files).map(([path, body]) => [`repos/${dir}/${path}`, body])))
  const git = dirs.flatMap((dir) => [
    ['init', '-q', '--template=', `repos/${dir}`],
    ...(repos[dir].remote === null ? [] : [['-C', `repos/${dir}`, 'remote', 'add', 'origin', repos[dir].remote ?? GITHUB]]),
    ['-C', `repos/${dir}`, 'add', '-A'],
    ['-C', `repos/${dir}`, 'commit', '-q', '-m', 'base'],
  ])
  return { argv: ['--help'], write, git }
}

/** What one fake gh call prints, says on stderr and exits with. */
const answer = ({ out = '', err = '', code = 0 }) => `printf '%s' ${q(out)}\nprintf '%s' ${q(err)} >&2\nexit ${code}`
/** A fake gh: `views` by issue number, `lists` by the end of the REST path (`1/sub_issues`, `2/dependencies/blocked_by`). */
const fakeGh = ({ views = {}, lists = {} }) =>
  `#!/bin/sh\nif [ "$1" = api ]; then\ncase "$6" in\n${Object.entries(lists)
    .map(([path, reply]) => `*/issues/${path}?per_page=100)\n${answer(reply)}\n;;`)
    .join('\n')}\nesac\nelif [ "$1 $2" = "issue view" ]; then\ncase "$3" in\n${Object.entries(views)
    .map(([number, reply]) => `${number})\n${answer(reply)}\n;;`)
    .join('\n')}\nesac\nfi\necho "gh: unexpected: $*" >&2\nexit 9\n`
const tools = (gh, files = {}) => ({ write: { 'bin/gh': fakeGh(gh), ...files }, exec: ['bin/gh'], shims: ['env', 'git'] })

const SPEC = '## User Stories\n\n1. As a user\n2. As an admin\n3) As a guest\n\n## Next\n\n4. not a story\n'
const view = (number, body, extra = {}) => ({ out: JSON.stringify({ number, title: `Spec ${number}`, body, url: `${URL}/${number}`, labels: [], ...extra }) })
const sub = (number, body, extra = {}) => ({ number, title: `Ticket ${number}`, body, html_url: `${URL}/${number}`, labels: [{ name: 'x' }], state: 'open', ...extra })
const pages = (...pages) => ({ out: JSON.stringify(pages) })
const blocker = (number, url = `${URL}/${number}`) => ({ number, html_url: url })

const TICKET_2 = '## Acceptance criteria\n\n- [ ] one\n- [x] two\n\nCovers user story 1.\n'
const TICKET_3 = 'Covers user stories 2 and 4.\n\n- [ ] only\n'
const TICKET_4 = 'Nothing cited.\n'
const GH = {
  views: { 1: view(1, SPEC), 6: view(6, 'no stories here'), 7: view(7, SPEC), 8: view(8, SPEC), 9: { err: 'GraphQL: Could not resolve to an issue\nmore', code: 1 } },
  lists: {
    '1/sub_issues': pages([sub(2, TICKET_2), sub(3, TICKET_3)], [sub(4, TICKET_4, { state: 'closed' }), sub(5, '', { html_url: '', title: 5 })]),
    '2/dependencies/blocked_by': pages([blocker(3)]),
    '3/dependencies/blocked_by': pages([blocker(2), blocker(4)]),
    '4/dependencies/blocked_by': pages([blocker(11, 'https://github.com/other/repo/issues/11'), blocker(12, '')], []),
    '5/dependencies/blocked_by': pages([]),
    '6/sub_issues': pages([]),
    '7/sub_issues': { err: '\n  HTTP 404: Not Found\n', code: 1 },
    '8/sub_issues': pages([sub(10, '')]),
    '10/dependencies/blocked_by': { out: '{"message":"no"}' },
  },
}

scenarios['check-tickets-github'] = [
  setup({ g1: { files: { 'README.md': '# example\n' } } }),
  { ...check({ repo: R('g1'), spec: '#1' }), ...tools(GH) },
  check({ repo: R('g1'), spec: `${URL}/1` }),
  check({ repo: R('g1'), spec: 'JorisJonkers-dev/example#1' }),
  // No stories in the spec, and no tickets at all.
  check({ repo: R('g1'), spec: '6' }),
  // What gh fails on: the spec, the sub-issues, one ticket's blockers.
  check({ repo: R('g1'), spec: '#9' }),
  check({ repo: R('g1'), spec: '#7' }),
  check({ repo: R('g1'), spec: '#8' }),
]

const LOCAL_DOC = { 'docs/agents/issue-tracker.md': '# Issue tracker: Local Markdown\n\nTickets live in .scratch.\n' }
const LOCAL = {
  ...LOCAL_DOC,
  '.scratch/feat/spec.md': '# Feature\n\n## User stories\n\n1. one\n2. two\n3. three\n',
  '.scratch/feat/issues/01-schema.md': '# 01: Schema\n\nStatus: done\n\nBlocked by: None (can start immediately)\n\n## Acceptance criteria\n\n- [ ] a\n- [x] b\n\nUser story 1\n',
  '.scratch/feat/issues/02-api.md': '# 02 - API\n\n**Blocked by:** 01, 03\n\n- [ ] c\n\nuser stories 2–3\n',
  '.scratch/feat/issues/03-ui.md': '# UI\n\n## Blocked by\n\n- #2\n- Schema\n- Missing ticket\n',
  '.scratch/feat/issues/notes.txt': 'not a ticket\n',
  '.scratch/feat/issues/sub/04-x.md': '# nested, not read\n',
  'spec.md': '# Root spec\n\nNo stories.\n',
  'issues/1-a.md': '# A\n\nBlocked by: 1\n',
  'other/spec.md': '# Other\n',
}
scenarios['check-tickets-local'] = [
  setup({ l1: { files: LOCAL }, l2: { files: { ...LOCAL_DOC, 'x.md': '# x\n' }, remote: 'https://gitlab.com/someone/project.git' } }),
  { ...check({ repo: R('l1'), spec: '.scratch/feat/spec.md' }), ...tools({}) },
  check({ repo: R('l1'), spec: `${SB}/repos/l1/.scratch/feat/spec.md` }),
  check({ repo: R('l1'), spec: 'spec.md' }),
  // A spec with no issues folder beside it has no tickets.
  check({ repo: R('l1'), spec: 'other/spec.md' }),
  check({ repo: R('l1'), spec: 'missing.md' }),
  check({ repo: R('l1'), spec: '../outside.md' }),
  // A repository off GitHub with no tracker doc tracks on local markdown too.
  check({ repo: R('l2'), spec: 'x.md' }),
]

scenarios['check-tickets-refusals'] = [
  setup({
    r1: { files: { 'docs/agents/issue-tracker.md': '# Issue tracker: GitLab\n' } },
    r2: { files: { 'a.md': 'a\n' }, remote: null },
    r3: { files: { 'a.md': 'a\n' } },
  }),
  { ...check({ repo: R('r1'), spec: '#1' }), ...tools({}) },
  check({ repo: R('r2'), spec: '#1' }),
  check({ repo: R('r3'), spec: 'not-an-issue' }),
  check({ repo: R('r3'), spec: 'other/repo#3' }),
  check({ spec: '#1' }),
  check({ repo: R('r3'), spec: 3 }),
  check({ repo: 5 }),
  check('[]'),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
