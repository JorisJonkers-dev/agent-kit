#!/usr/bin/env node
// Writes the scenario.json files of prepare (the goldens are expected.json, from regenerate-fixtures.mjs).
// The repositories are real git ones made in the sandbox; the tracker is local markdown unless a scenario says GitHub,
// and then gh is a fake program in the sandbox's bin/. code-nav is on the PATH only where a step puts a fake there, so
// the other steps show the message of a code-nav that is not installed. The replay's clock is fixed, so run ids repeat
// and take a count.
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const SB = '@SANDBOX@'
const GITHUB = 'git@github.com:JorisJonkers-dev/example.git'
const KEY = 'github.com/jorisjonkers-dev/example'
const prepare = (input, extra = {}) => ({ argv: ['prepare', '--json', typeof input === 'string' ? input : JSON.stringify(input)], path: ['bin'], ...extra })
const R = (dir) => `${SB}/repos/${dir}`
const q = (text) => `'${text.split("'").join("'\\''")}'`
const scenarios = {}
const ALLOWLIST = 'home/.config/agent-kit/critique-allowlist.json'
const OVERRIDE = 'home/.config/agent-kit/critique-routing.json'
const allow = (families = ['mistral', 'zhipu', 'deepseek', 'google'], code = true, history = false) => ({
  [ALLOWLIST]: `${JSON.stringify({ repositories: { [KEY]: { code, families, history, updated_at: '2026-11-01T00:00:00.000Z' } }, version: 1 })}\n`,
})
const roots = (...dirs) => dirs.map((dir) => `repos/${dir}/.git/critique`)

/** Repositories committed once, or twice when a repository has a `head`; `remote: null` leaves one with no remote. */
const setup = (repos) => {
  const dirs = Object.keys(repos)
  const files = (which) => Object.fromEntries(dirs.flatMap((dir) => Object.entries(repos[dir][which] ?? {}).filter(([, body]) => body !== null).map(([path, body]) => [`repos/${dir}/${path}`, body])))
  const removed = dirs.flatMap((dir) => Object.entries(repos[dir].head ?? {}).filter(([, body]) => body === null).map(([path]) => `repos/${dir}/${path}`))
  return [
    {
      argv: ['--help'],
      write: files('files'),
      shims: ['env', 'mkdir', 'git'],
      git: dirs.flatMap((dir) => [
        ['init', '-q', '--template=', `repos/${dir}`],
        ...(repos[dir].remote === null ? [] : [['-C', `repos/${dir}`, 'remote', 'add', 'origin', repos[dir].remote ?? GITHUB]]),
        ['-C', `repos/${dir}`, 'add', '-A'],
        ['-C', `repos/${dir}`, 'commit', '-q', '-m', 'base #12'],
      ]),
    },
    {
      argv: ['--help'],
      write: files('head'),
      remove: removed,
      git: dirs.filter((dir) => repos[dir].head !== undefined).flatMap((dir) => [['-C', `repos/${dir}`, 'add', '-A'], ['-C', `repos/${dir}`, 'commit', '-q', '-m', 'head']]),
    },
  ]
}

const fakeGh = (issues) =>
  `#!/bin/sh\ncase "$1 $2 $3" in\n${Object.entries(issues)
    .map(([number, body]) => `"issue view ${number}")\nprintf '%s' ${q(JSON.stringify({ number: Number(number), title: `Issue ${number}`, body, url: `https://github.com/jorisjonkers-dev/example/issues/${number}`, labels: [{ name: 'type: bug' }] }))}\nexit 0\n;;`)
    .join('\n')}\n*)\necho "gh: unexpected: $*" >&2\nexit 9\n;;\nesac\n`
const NAV_MAP = (paths) => ['files:', ...paths.map((path, n) => `${SB}/${path} (${9 - n} refs, 1 symbols)`), 'symbols:', ...paths.map((path, n) => `${SB}/${path}:1 function main (${9 - n} refs)`)].join('\n')
const fakeNav = (dir, paths) =>
  `#!/bin/sh\nprintf '%s\\n' ${q(JSON.stringify({ jsonrpc: '2.0', id: 1, result: { capabilities: {} } }))}\nprintf '%s\\n' ${q(JSON.stringify({ jsonrpc: '2.0', id: 2, result: { content: [{ type: 'text', text: NAV_MAP(paths.map((path) => `repos/${dir}/${path}`)) }] } }))}\n`
const SHIMS = ['env', 'mkdir', 'git']

const LOCAL = { 'docs/agents/issue-tracker.md': '# Issue tracker: Local Markdown\n\nTickets live in .scratch.\n' }
const ADR = (status, extra = '') => `# Decision\n\nStatus: ${status}\n\nWe use \`src/main.rs\` and the \`Widget\` type.${extra}\n`
const SOURCE = Object.fromEntries(
  ['src/main.rs', 'src/lib.rs', 'src/util/mod.rs', 'src/deep/er/leaf.ts', 'web/index.ts', 'web/page.tsx'].map((path, n) => [path, `// file ${n}\n${Array.from({ length: 6 + n * 3 }, (_, k) => `pub fn f${n}_${k}() { f${n}_${k + 1}() }`).join('\n')}\n`]),
)
SOURCE['web/page.tsx'] = `import { f4_1 } from './index'\n${SOURCE['web/page.tsx']}`
SOURCE['web/more.ts'] = `import { f4_2 } from './index'\nexport const more = f4_2\n`
const DOCS = {
  'GLOSSARY.md': '# Glossary\n\n**Widget**: a thing.\n',
  'docs/adr/0001-first.md': ADR('accepted'),
  'docs/adr/0002-second.md': `${ADR('proposed')}\nSee [first](0001-first.md).\n`,
  'docs/adr/0003-third.md': ADR('superseded by 0001'),
  'README.md': '# example\n\nSee `src/lib.rs`.\n',
  'package.json': '{"name":"x"}\n',
  '.dependency-cruiser.js': 'module.exports = {}\n',
}

// Who may be critiqued, and what the input must say.
scenarios['prepare-refusals'] = [
  ...setup({
    p1: { files: { ...LOCAL, 'docs/spec.md': '# Spec\n\n## User Stories\n\n1. one\n' } },
    p2: { files: { 'a.md': 'a\n' }, remote: null },
    p3: { files: { 'a.md': 'a\n' }, remote: 'git@github.com:nedap/secret.git' },
    p4: { files: { ...LOCAL, 'docs/spec.md': '# Spec\n' }, remote: 'https://gitlab.com/someone/project.git' },
  }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('p1'), spec: 'docs/spec.md' }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('p2'), spec: 'docs/spec.md' }, { write: allow() }),
  prepare({ stage: 'plan', orchestrator_family: 'anthropic', repo: R('p1'), plan: 'plan.md' }, { write: allow(['mistral'], false) }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('p1') }, { write: allow() }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('p1'), spec: 'missing.md' }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('p1'), spec: '../outside.md' }),
  prepare({ stage: 'adr-review', orchestrator_family: 'anthropic', repo: R('p1'), flow: 'f' }),
  prepare({ stage: 'nope', orchestrator_family: 'anthropic', repo: R('p1') }),
  prepare({ stage: 'spec', orchestrator_family: 'nobody', repo: R('p1') }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic' }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('p1'), depth: 'x' }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('p1'), comments: 'yes' }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('p1'), staged: true }),
  prepare({ stage: 'ticket-review', orchestrator_family: 'anthropic', repo: R('p1'), staged: true, head: 'HEAD' }),
  prepare({ stage: 'ticket-review', orchestrator_family: 'anthropic', repo: R('p1'), staged: 'x' }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('p4'), spec: 'docs/spec.md' }, { write: { [ALLOWLIST]: `${JSON.stringify({ repositories: { 'gitlab.com/someone/project': { code: false, families: ['mistral'], history: false, updated_at: '2026-11-01T00:00:00.000Z' } }, version: 1 })}\n` } }),
  prepare('[]'),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('p1'), spec: 'docs/spec.md' }, { write: { [OVERRIDE]: '{' } }),
]

// The decisions stage reads no tracker: the feature, then the summary or the decision docs.
scenarios['prepare-decisions'] = [
  ...setup({ d1: { files: { ...DOCS, 'docs/agents/domain.md': '# Domain\n\nDocs: `GLOSSARY.md` and `docs/adr`.\n' } }, d2: { files: { 'a.md': 'a\n' } } }),
  prepare({ stage: 'decisions', orchestrator_family: 'anthropic', repo: R('d1'), feature: 'Add the thing' }, { write: allow(), roots: roots('d1', 'd2') }),
  prepare({ stage: 'decisions', orchestrator_family: 'openai', repo: R('d1'), feature: 'Add the thing', decision_summary: 'We chose X.\n- because Y\n' }, { roots: roots('d1') }),
  prepare({ stage: 'decisions', orchestrator_family: 'anthropic', repo: R('d1'), feature: 'Add the thing', references: ['GLOSSARY.md'], depth: 'deep', flow: 'draft:one' }, { roots: roots('d1') }),
  prepare({ stage: 'decisions', orchestrator_family: 'anthropic', repo: R('d1'), feature: 'a\nb\nc\nd\ne\nf' }),
  prepare({ stage: 'decisions', orchestrator_family: 'anthropic', repo: R('d1') }),
  prepare({ stage: 'decisions', orchestrator_family: 'anthropic', repo: R('d1'), feature: 'f', references: ['../x.md'] }),
  prepare({ stage: 'decisions', orchestrator_family: 'anthropic', repo: R('d2'), feature: 'f' }, { write: allow(), roots: roots('d2') }),
]

// A spec as a document packet: references, depth signals, rounds.
scenarios['prepare-spec'] = [
  ...setup({
    s1: { files: { ...LOCAL, ...DOCS, 'docs/spec.md': '# Spec\n\n## User Stories\n\n1. one\n2. two\n3. three\n\nA migration of the schema.\n', 'docs/other.md': '# Other\n' } },
  }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('s1'), spec: 'docs/spec.md' }, { write: allow(), roots: roots('s1') }),
  prepare({ stage: 'spec', orchestrator_family: 'openai', repo: R('s1'), spec: 'docs/spec.md', depth: 'deep', references: ['GLOSSARY.md', 'docs/adr/0001-first.md'] }, { roots: roots('s1') }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('s1'), spec: 'docs/spec.md', flow: 'my-flow' }, { write: { [OVERRIDE]: '{"stages":{"spec":{"depth_signals":{"risk_terms":["migration"],"thresholds":{"words":3}}}}}\n' }, roots: roots('s1') }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('s1'), spec: 'docs/spec.md', references: ['docs/missing.md'] }),
  prepare({ stage: 'spec', orchestrator_family: 'mistral', repo: R('s1'), spec: 'docs/spec.md' }, { write: allow(['mistral', 'google']), roots: roots('s1') }),
  // A second round of the first run, then a third, and the ways a second round is refused.
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('s1'), spec: 'docs/spec.md', previous_run_dir: `${SB}/repos/s1/.git/critique/20261103T120000Z-spec` }, { write: allow() }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('s1'), spec: 'docs/spec.md', previous_run_dir: `${SB}/repos/s1/.git/critique/20261103T120000Z-spec` }, { write: { 'repos/s1/.git/critique/20261103T120000Z-spec/report.md': '# report\n' } }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('s1'), spec: 'docs/spec.md', previous_run_dir: `${SB}/repos/s1/.git/critique/20261103T120000Z-spec-2` }, { roots: roots('s1') }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('s1'), spec: 'docs/spec.md', previous_run_dir: `${SB}/repos/s1/.git/critique/20261103T120000Z-spec-3` }, {}),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('s1'), spec: 'docs/spec.md', previous_run_dir: `${SB}/repos/s1/.git/critique/nowhere` }),
  prepare({ stage: 'tickets', orchestrator_family: 'anthropic', repo: R('s1'), spec: 'docs/spec.md', previous_run_dir: `${SB}/repos/s1/.git/critique/20261103T120000Z-spec` }),
  // A second round is the last, and it must be of the same stage over the same artifact.
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('s1'), spec: 'docs/spec.md', previous_run_dir: `${SB}/repos/s1/.git/critique/20261103T120000Z-spec-5` }, { write: { 'repos/s1/.git/critique/20261103T120000Z-spec-5/report.md': '# report\n' } }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('s1'), spec: 'docs/other.md', previous_run_dir: `${SB}/repos/s1/.git/critique/20261103T120000Z-spec` }),
]

// The tickets stage: the spec and its tickets, with the checker's report.
scenarios['prepare-tickets'] = [
  ...setup({
    t1: {
      files: {
        ...LOCAL,
        '.scratch/feat/spec.md': '# Feature\n\n## User stories\n\n1. one\n2. two\n',
        '.scratch/feat/issues/01-schema.md': '# 01: Schema\n\nBlocked by: None\n\n## Acceptance criteria\n\n- [ ] a\n- [x] b\n\nUser story 1\n',
        '.scratch/feat/issues/02-api.md': '# 02 - API\n\n**Blocked by:** 01\n\n- [ ] c\n\nuser story 2\n',
        '.scratch/none/spec.md': '# None\n',
      },
    },
  }),
  prepare({ stage: 'tickets', orchestrator_family: 'anthropic', repo: R('t1'), spec: '.scratch/feat/spec.md' }, { write: allow(), roots: roots('t1') }),
  prepare({ stage: 'tickets', orchestrator_family: 'anthropic', repo: R('t1'), spec: '.scratch/none/spec.md' }),
  prepare({ stage: 'tickets', orchestrator_family: 'anthropic', repo: R('t1'), spec: '.scratch/feat/spec.md' }, { write: { [OVERRIDE]: '{"stages":{"tickets":{"depth_signals":{"thresholds":{"tickets":1}},"ticket_size":{"max_criteria":1,"max_modules":2}}}}\n' }, roots: roots('t1') }),
]

// A code stage: the diff, the slices, the tree, and the staged change.
const CODE_BASE = {
  ...LOCAL,
  ...SOURCE,
  '.scratch/feat/spec.md': '# Feature\n\nChange `src/main.rs` and `web/index.ts`.\n',
  '.scratch/feat/issues/01-one.md': '# 01: One\n\n- [ ] a\n',
  'package-lock.json': '{\n"a":1\n}\n',
  'docs/notes.md': '# notes\n',
}
const CODE_HEAD = {
  'src/main.rs': `${SOURCE['src/main.rs']}pub fn added() { f0_1() }\n`,
  'src/lib.rs': SOURCE['src/lib.rs'].replace('f1_2', 'f1_2b'),
  'web/index.ts': `${SOURCE['web/index.ts']}export const extra = f4_1\n`,
  'package-lock.json': '{\n"a":2\n}\n',
  'docs/notes.md': '# notes\n\nmore\n',
  'src/new.rs': 'pub fn fresh() {}\n',
}
scenarios['prepare-code'] = [
  ...setup({ c1: { files: CODE_BASE, head: CODE_HEAD } }),
  prepare({ stage: 'ticket-review', orchestrator_family: 'anthropic', repo: R('c1'), spec: '.scratch/feat/spec.md', ticket: '.scratch/feat/issues/01-one.md', base: 'HEAD~1' }, { write: allow(), roots: roots('c1') }),
  prepare({ stage: 'feature-review', orchestrator_family: 'anthropic', repo: R('c1'), spec: '.scratch/feat/spec.md', base: 'HEAD~1', head: 'HEAD' }, { write: { 'bin/code-nav': fakeNav('c1', ['src/main.rs', 'src/lib.rs', 'web/index.ts', 'gone.rs']) }, exec: ['bin/code-nav'], shims: SHIMS, roots: roots('c1') }),
  prepare({ stage: 'feature-review', orchestrator_family: 'anthropic', repo: R('c1'), spec: '.scratch/feat/spec.md', base: 'HEAD~1' }, { write: { [OVERRIDE]: '{"packet":{"cap_bytes":5700,"window_lines":4,"facts_cap_bytes":1000}}\n' }, roots: roots('c1') }),
  prepare({ stage: 'feature-review', orchestrator_family: 'anthropic', repo: R('c1'), spec: '.scratch/feat/spec.md', base: 'HEAD~1' }, { write: { [OVERRIDE]: '{"packet":{"cap_bytes":3000,"window_lines":4,"facts_cap_bytes":1000}}\n' }, roots: roots('c1') }),
  prepare({ stage: 'ticket-review', orchestrator_family: 'anthropic', repo: R('c1'), spec: '.scratch/feat/spec.md', base: 'HEAD~1' }),
  prepare({ stage: 'feature-review', orchestrator_family: 'anthropic', repo: R('c1'), spec: '.scratch/feat/spec.md' }),
  prepare({ stage: 'feature-review', orchestrator_family: 'anthropic', repo: R('c1'), spec: '.scratch/feat/spec.md', base: 'nowhere' }),
  prepare({ stage: 'ticket-review', orchestrator_family: 'anthropic', repo: R('c1'), spec: '.scratch/feat/spec.md', ticket: '.scratch/feat/issues/01-one.md', base: 'HEAD~1', depth: 'deep' }, { write: { [OVERRIDE]: '{"triage":{"risk_paths":["docs/**"]},"stages":{"ticket-review":{"depth_signals":{"thresholds":{"changed_lines":1,"risk_path_hits":0}}}}}\n' }, roots: roots('c1') }),
  // The staged change: the index becomes a commit under review, and the manifest keeps its hash.
  { argv: ['--help'], write: { 'repos/c1/src/staged.rs': 'pub fn staged() {}\n' }, git: [['-C', 'repos/c1', 'add', 'src/staged.rs']] },
  prepare({ stage: 'ticket-review', orchestrator_family: 'anthropic', repo: R('c1'), spec: '.scratch/feat/spec.md', ticket: '.scratch/feat/issues/01-one.md', base: 'HEAD', staged: true }, { env: { GIT_AUTHOR_DATE: '2026-11-03T12:00:00Z', GIT_COMMITTER_DATE: '2026-11-03T12:00:00Z' }, roots: roots('c1') }),
]

// The review stages and the plan: a sampled tree, the decision records, the facts index.
scenarios['prepare-repository'] = [
  ...setup({
    r1: { files: { ...DOCS, ...SOURCE, 'docs/agents/issue-tracker.md': '# Issue tracker: GitHub\n' }, head: { 'src/main.rs': `${SOURCE['src/main.rs']}// changed\n` } },
    r2: { files: { 'docs/adr/0001-only.md': ADR('accepted') } },
    r3: { files: { 'x.md': 'x\n' } },
    r4: { files: { ...LOCAL, ...DOCS, ...SOURCE, '.scratch/draft/refactoring-plan.md': '# Refactoring plan\n\n## Goal\n\nTidy it.\n\n## Target\n\nsrc/main.rs\n\n## Safety net\n\nTests.\n\n## Steps\n\n### 1. Move\n\nKind: tidy\n\nTouches: src/main.rs\n\n## Not doing\n\nNothing.\n\n## Open decisions\n\nNone.\n' } },
  }),
  prepare({ stage: 'adr-review', orchestrator_family: 'anthropic', repo: R('r1') }, { write: allow(['mistral', 'zhipu', 'deepseek', 'google'], false), roots: roots('r1') }),
  prepare({ stage: 'adr-review', orchestrator_family: 'anthropic', repo: R('r1'), adrs: ['docs/adr/0001-first.md'], references: ['GLOSSARY.md'] }, { write: allow(), roots: roots('r1') }),
  prepare({ stage: 'adr-review', orchestrator_family: 'anthropic', repo: R('r3') }, { write: allow(), roots: roots('r3') }),
  prepare({ stage: 'architecture-review', orchestrator_family: 'openai', repo: R('r1'), head: 'HEAD' }, { write: { 'bin/code-nav': fakeNav('r1', ['src/main.rs', 'web/index.ts', 'src/lib.rs']) }, exec: ['bin/code-nav'], shims: SHIMS, roots: roots('r1') }),
  prepare({ stage: 'architecture-review', orchestrator_family: 'openai', repo: R('r1'), head: 'HEAD' }, { write: { [OVERRIDE]: '{"packet":{"cap_bytes":2700,"window_lines":3}}\n' }, roots: roots('r1') }),
  prepare({ stage: 'architecture-review', orchestrator_family: 'openai', repo: R('r1'), head: 'HEAD' }, { write: { [OVERRIDE]: '{"packet":{"cap_bytes":600}}\n' } }),
  prepare({ stage: 'decision-recovery', orchestrator_family: 'anthropic', repo: R('r1') }, { write: { ...allow(), 'bin/gh': fakeGh({ 12: 'the issue body' }) }, exec: ['bin/gh'], shims: SHIMS, roots: roots('r1') }),
  prepare({ stage: 'decision-recovery', orchestrator_family: 'anthropic', repo: R('r1') }, { write: allow(['mistral', 'zhipu', 'deepseek', 'google'], true, true), roots: roots('r1') }),
  prepare({ stage: 'decision-recovery', orchestrator_family: 'anthropic', repo: R('r1') }, { write: { [OVERRIDE]: '{"packet":{"cap_bytes":300}}\n' } }),
  prepare({ stage: 'plan', orchestrator_family: 'anthropic', repo: R('r4'), plan: '.scratch/draft/refactoring-plan.md' }, { write: allow(), roots: roots('r4') }),
  prepare({ stage: 'plan', orchestrator_family: 'anthropic', repo: R('r4'), flow: 'draft:draft' }, { roots: roots('r4') }),
  prepare({ stage: 'plan', orchestrator_family: 'anthropic', repo: R('r4'), plan: '.scratch/draft/refactoring-plan.md' }, { write: { [OVERRIDE]: '{"packet":{"cap_bytes":300}}\n' } }),
  prepare({ stage: 'plan', orchestrator_family: 'anthropic', repo: R('r4'), flow: 'draft:missing' }),
  prepare({ stage: 'plan', orchestrator_family: 'anthropic', repo: R('r4'), flow: 'draft:a/../b' }),
  prepare({ stage: 'plan', orchestrator_family: 'anthropic', repo: R('r4') }),
  prepare({ stage: 'plan', orchestrator_family: 'anthropic', repo: R('r4'), plan: '/etc/passwd' }),
  prepare({ stage: 'plan', orchestrator_family: 'anthropic', repo: R('r4'), plan: `${SB}/repos/r4/.git/critique/plans/x/refactoring-plan.md` }),
]

// A nedap repository has the work-only roster: the grant, and the work clients.
scenarios['prepare-work'] = [
  ...setup({ w1: { files: { ...LOCAL, 'docs/spec.md': '# Spec\n' }, remote: 'git@github.com:nedap/secret.git' } }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('w1'), spec: 'docs/spec.md' }, { roots: roots('w1') }),
  prepare({ stage: 'spec', orchestrator_family: 'openai', repo: R('w1'), spec: 'docs/spec.md', workflow: 'wf-1' }, { roots: roots('w1') }),
  prepare({ stage: 'spec', orchestrator_family: 'mistral', repo: R('w1'), spec: 'docs/spec.md', flow: 'wf-2' }, { roots: roots('w1') }),
  // A live grant covers the workflow; an unreadable grants file leaves Codex out and says why.
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('w1'), spec: 'docs/spec.md', workflow: 'wf-live' }, { write: { 'home/.config/agent-kit/model-gateway-grants.json': `${JSON.stringify({ version: 1, grants: [{ id: 'g1', workflow: 'wf-live', client: 'codex', cap_usd: 5, question: 'q', answer: 'a', written_at: '2026-11-03T10:00:00.000Z', expires_at: '2026-11-04T10:00:00.000Z' }] })}\n` }, roots: roots('w1') }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('w1'), spec: 'docs/spec.md', workflow: 'wf-live' }, { write: { 'home/.config/agent-kit/model-gateway-grants.json': '{' }, roots: roots('w1') }),
]

// A GitHub tracker gh cannot answer, and one the critique cannot read.
scenarios['prepare-github'] = [
  ...setup({
    h1: { files: { ...SOURCE, 'docs/a.md': 'a\n' }, head: { 'src/main.rs': `${SOURCE['src/main.rs']}// more\n` } },
    h2: { files: { 'docs/agents/issue-tracker.md': '# Issue tracker: GitLab\n', 'docs/a.md': 'a\n' }, remote: 'https://gitlab.com/someone/project.git' },
  }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('h1'), spec: '#9' }, { write: { ...allow(), 'bin/gh': fakeGh({ 5: 'the spec', 6: 'a ticket' }) }, exec: ['bin/gh'] }),
  prepare({ stage: 'tickets', orchestrator_family: 'anthropic', repo: R('h1'), spec: '#9' }),
  prepare({ stage: 'feature-review', orchestrator_family: 'anthropic', repo: R('h1'), spec: '#9', base: 'HEAD~1' }),
  prepare({ stage: 'feature-review', orchestrator_family: 'anthropic', repo: R('h1'), spec: '#5', base: 'HEAD~1' }),
  prepare({ stage: 'ticket-review', orchestrator_family: 'anthropic', repo: R('h1'), spec: '#5', ticket: '#7', base: 'HEAD~1' }),
  prepare({ stage: 'plan', orchestrator_family: 'anthropic', repo: R('h1'), flow: 'draft:one' }),
  prepare({ stage: 'plan', orchestrator_family: 'anthropic', repo: R('h2'), flow: 'draft:one' }, { write: { [ALLOWLIST]: `${JSON.stringify({ repositories: { 'gitlab.com/someone/project': { code: true, families: ['mistral'], history: true, updated_at: '2026-11-01T00:00:00.000Z' } }, version: 1 })}\n` } }),
  prepare({ stage: 'spec', orchestrator_family: 'anthropic', repo: R('h2'), spec: 'docs/a.md' }),
  prepare({ stage: 'decision-recovery', orchestrator_family: 'anthropic', repo: R('h2') }),
]

// What each client says about whether it can run, and the day's Antigravity quota.
const AGY = `#!/bin/sh\nprintf 'Fetching available models...\\ngemini-3.8-flash-high\\tGemini\\n'\n`
const VIBE_HOME = { 'home/.vibe/.env': 'MISTRAL_API_KEY=sk-test\nZAI_API_KEY=sk-z\n', 'home/.vibe/agents/critic-mistral-large.toml': '# agent\n', 'home/.vibe/agents/critic-glm.toml': '# agent\n', 'home/.vibe/agents/critic-mistral-small.toml': '# agent\n' }
const agyLines = (count) => `${Array.from({ length: count }, (_, n) => JSON.stringify({ type: 'invocation', recorded_at: '2026-11-03T08:00:00.000Z', repository: KEY, flow: 'f', workflow: 'f', round: 1, stage: 'spec', kind: 'agy', charged_usd: 0, critic_id: `c${n}` })).join('\n')}\n`
scenarios['prepare-roster'] = [
  ...setup({ k1: { files: { ...LOCAL, 'docs/spec.md': '# Spec\n\n## User Stories\n\n1. one\n' } } }),
  prepare({ stage: 'spec', orchestrator_family: 'openai', repo: R('k1'), spec: 'docs/spec.md', depth: 'deep' }, { write: { ...allow(), 'bin/agy': AGY, 'bin/vibe': '#!/bin/sh\nexit 0\n', ...VIBE_HOME }, exec: ['bin/agy', 'bin/vibe'], roots: roots('k1') }),
  prepare({ stage: 'spec', orchestrator_family: 'openai', repo: R('k1'), spec: 'docs/spec.md' }, { write: { 'home/.local/share/agent-kit/critique/ledger.jsonl': agyLines(12) }, roots: roots('k1') }),
  prepare({ stage: 'spec', orchestrator_family: 'openai', repo: R('k1'), spec: 'docs/spec.md' }, { write: { 'home/.local/share/agent-kit/critique/ledger.jsonl': agyLines(25) }, roots: roots('k1') }),
  prepare({ stage: 'decisions', orchestrator_family: 'openai', repo: R('k1'), feature: 'a feature', decision_summary: 'We chose X.' }, { write: { 'home/.local/share/agent-kit/critique/ledger.jsonl': agyLines(12) }, roots: roots('k1') }),
  prepare({ stage: 'spec', orchestrator_family: 'openai', repo: R('k1'), spec: 'docs/spec.md' }, { write: { 'home/.config/agent-kit/model-gateway.json': '{"clients":{"vibe":{"plan_limits":[{"window":"day","limit_usd":0.00033}]}}}\n', 'state/agent-kit/model-gateway/ledger.jsonl': `${JSON.stringify({ at: '2026-11-03T09:00:00.000Z', client: 'vibe', model: 'm', cost_usd: 0.0003, outcome: 'ok', run_id: 'r1', workflow: 'w' })}\n` }, roots: roots('k1') }),
  prepare({ stage: 'spec', orchestrator_family: 'openai', repo: R('k1'), spec: 'docs/spec.md' }, { write: { 'home/.config/agent-kit/model-gateway.json': '{"clients":{"vibe":{"plan_limits":[{"window":"day","limit_usd":0.0002}]}}}\n', 'state/agent-kit/model-gateway/ledger.jsonl': `${JSON.stringify({ at: '2026-11-03T09:00:00.000Z', client: 'vibe', model: 'm', cost_usd: 0.0003, outcome: 'ok', run_id: 'r1', workflow: 'w' })}\n` }, roots: roots('k1') }),
]

/** A routing override stays where a step wrote it, so the next step without one of its own starts without it. */
const withoutStaleOverrides = (steps) => {
  let written = false
  return steps.map((step) => {
    const own = step.write?.[OVERRIDE] !== undefined
    const stale = written && !own
    written = own
    return stale ? { ...step, remove: [...(step.remove ?? []), OVERRIDE] } : step
  })
}
for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps: withoutStaleOverrides(steps) }, null, 2)}\n`)
}
