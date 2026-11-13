#!/usr/bin/env node
// Writes the scenario.json files of gate and the critique-commit-gate and critique-pr-gate hooks (the goldens are
// expected.json, from regenerate-fixtures.mjs). The records a gate reads name commits and staged changes by id, so
// the generator replays each repository's git steps in a scratch repository, with the identity and instants the
// fixture runner uses, to learn those ids before it writes the records.
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const SB = '@SANDBOX@'
const KEY = 'github.com/jorisjonkers-dev/example'
const REMOTE = 'git@github.com:JorisJonkers-dev/example.git'
const NOW = '2026-11-03T12:00:00Z'
const IDENTITY = ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', '-c', 'commit.gpgsign=false', '-c', 'init.defaultBranch=main']
const ALLOWLIST = 'home/.config/agent-kit/critique-allowlist.json'
const CONFIG = 'home/.config/agent-kit/critique.json'
const LOG = 'home/.local/share/agent-kit/critique'
const allow = (code = true) => ({ [ALLOWLIST]: `${JSON.stringify({ repositories: { [KEY]: { code, families: ['mistral'], updated_at: '2026-11-01T00:00:00.000Z' } }, version: 1 })}\n` })

/** One repository's history as steps: `write` files (null removes one) or run `git` in it. */
class Repo {
  constructor(name, { remote = REMOTE } = {}) {
    this.name = name
    this.ops = [{ git: ['init', '-q', '--template=', '.'] }, ...(remote === null ? [] : [{ git: ['remote', 'add', 'origin', remote] }])]
    this.scratch = mkdtempSync(join(tmpdir(), 'critique-gate-gen-'))
  }

  /** Applies ops to the scratch repository now, and returns them as fixture step fields. */
  apply(...ops) {
    const write = {}
    const remove = []
    const git = []
    for (const op of ops) {
      if (op.write !== undefined) {
        for (const [path, body] of Object.entries(op.write)) {
          if (body === null) {
            rmSync(join(this.scratch, path), { force: true })
            remove.push(`repos/${this.name}/${path}`)
          } else {
            mkdirSync(dirname(join(this.scratch, path)), { recursive: true })
            writeFileSync(join(this.scratch, path), body)
            write[`repos/${this.name}/${path}`] = body
          }
        }
      } else {
        this.git(...op.git)
        git.push(['-C', `repos/${this.name}`, ...op.git])
      }
    }
    return { write, remove, git }
  }

  git(...args) {
    const made = spawnSync('git', [...IDENTITY, ...args], { cwd: this.scratch, encoding: 'utf8', env: { PATH: process.env.PATH, HOME: this.scratch, GIT_CONFIG_NOSYSTEM: '1', GIT_AUTHOR_DATE: NOW, GIT_COMMITTER_DATE: NOW } })
    if (made.status !== 0 && !args.includes('--no-commit')) throw new Error(`git ${args.join(' ')}: ${made.stderr}`)
    return made.stdout.trim()
  }

  rev(revision) {
    return this.git('rev-parse', revision)
  }

  /** The id the critique gives the change staged now: sha256 of HEAD's tree and the staged entries. */
  stagedHash() {
    const tree = this.git('rev-parse', 'HEAD^{tree}')
    const entries = spawnSync('git', ['ls-files', '--stage', '-z'], { cwd: this.scratch, encoding: 'utf8' }).stdout
    return createHash('sha256').update(`staged-diff v1\0${tree}\0${entries}`).digest('hex')
  }
}

/** A step that sets up repositories: their first ops, then the flow state that makes a flow active. */
const setup = (repos, flows = { phase: 'implementation' }) => {
  const parts = repos.map((repo) => repo.apply(...repo.ops))
  const write = Object.assign({}, ...parts.map((part) => part.write))
  for (const repo of repos) if (flows !== null) write[`repos/${repo.name}/.git/critique/flows/feat.json`] = `${JSON.stringify({ repository: KEY, ...flows })}\n`
  return { argv: ['--help'], write, git: parts.flatMap((part) => part.git), shims: ['git'] }
}
/** Steps that apply ops in order: the fixture runner writes a step's files before its git runs, so a write after a git command starts a new step. */
const ops = (repo, ...list) => {
  const groups = []
  for (const op of list) {
    const last = groups.at(-1)
    if (last === undefined || (op.write !== undefined && last.some((other) => other.git !== undefined))) groups.push([op])
    else last.push(op)
  }
  return groups.map((group) => {
    const { write, remove, git } = repo.apply(...group)
    return { argv: ['--help'], write, remove, git }
  })
}
/** One step from `ops`, which must make only one. */
const op1 = (repo, ...list) => {
  const steps = ops(repo, ...list)
  if (steps.length !== 1) throw new Error('these ops make more than one step')
  return steps[0]
}
const payload = (repo, command, extra = {}) => ({ cwd: `${SB}/repos/${repo.name}`, tool_name: 'Bash', tool_input: { command }, ...extra })
const gate = (input, extra = {}) => ({ argv: ['gate', '--json', typeof input === 'string' ? input : JSON.stringify(input)], shims: ['git'], ...extra })
const hook = (name, input, extra = {}) => ({ argv: ['hook', name], stdin: typeof input === 'string' ? input : JSON.stringify(input), shims: ['git'], ...extra })

const review = (repo, fields) =>
  JSON.stringify({ base_commit: 'b'.repeat(40), critic_failures: [], degraded: false, head_commit: 'h'.repeat(40), recorded_at: NOW, repository: KEY, review: 'pass', reviewed_staged_diff: null, run_id: 'r1', spec: `${KEY}#1`, stage: 'ticket-review', staged_diff: null, ticket: `${KEY}#2`, unfixed_critical: [], version: 1, ...fields })
const triage = (fields) => JSON.stringify({ labels: [], recorded_at: NOW, repository: KEY, run_id: 't1', staged_diff: 's', staged_on: 'h', status: 'risky', ticket: `${KEY}#2`, version: 1, ...fields })
const gateRecord = (fields) =>
  JSON.stringify({ base_commit: 'b'.repeat(40), critical_open: 0, follow_ups: [], gate: 'pass', head_commit: 'h', line: 'GATE: PASS', open_critical: [], recorded_at: NOW, repository: KEY, run_id: 'g1', spec: `${KEY}#1`, stage: 'feature-review', version: 1, ...fields })
const index = (repo, name, lines) => ({ [`repos/${repo.name}/.git/critique/${name}`]: lines.length === 0 ? '' : `${lines.join('\n')}\n` })
const REVIEWS = 'ticket-reviews.jsonl'
const TRIAGES = 'triages.jsonl'
const GATES = 'gates.jsonl'

const scenarios = {}

// What the gate does before it looks at a repository.
{
  const s1 = new Repo('s1')
  const s2 = new Repo('s2', { remote: null })
  const s3 = new Repo('s3', { remote: 'git@github.com:nedap/thing.git' })
  for (const repo of [s1, s2, s3]) repo.ops.push({ write: { 'a.md': 'a\n' } }, { git: ['add', '-A'] }, { git: ['commit', '-q', '-m', 'base'] })
  scenarios['gate-skip'] = [
    setup([s1, s2, s3]),
    gate(payload(s1, 'git commit -m x'), { env: { CRITIQUE_OFF: '1' } }),
    gate(payload(s1, 'git commit -m x'), { write: { [CONFIG]: '{"disabled":true}\n' } }),
    gate(payload(s1, 'git commit -m x'), { write: { [CONFIG]: '{"disabled":"no"}\n' }, roots: [LOG] }),
    gate(payload(s1, 'git commit -m x'), { remove: [CONFIG], write: { [CONFIG]: '{"disabled":false}\n' } }),
    gate(payload(s1, 'git commit -m x'), { write: { [CONFIG]: '{}\n' } }),
    gate(payload(s1, 'git commit -m x', { tool_name: 'Read' })),
    gate(payload(s1, 'npm test && echo "git commit"')),
    gate('not json', { roots: [LOG] }),
    gate({ tool_input: {} }, { roots: [LOG] }),
    gate({ cwd: '/x', tool_input: { command: 5 } }, { roots: [LOG] }),
    gate({ cwd: '/x', tool_input: { command: 'git commit' }, gate: 'deploy' }, { roots: [LOG] }),
    gate(payload(s1, 'git commit -m x && git push', { gate: 'pr' })),
    gate(payload(s1, 'cd $DIR && git commit -m x'), { roots: [LOG] }),
    gate(payload(s2, 'git commit -m x')),
    gate(payload(s3, 'git push')),
    gate(payload(s1, 'git commit -m x')),
    gate(payload(s1, 'git commit -m x'), { write: allow(false) }),
    gate(payload(s1, 'git commit -m x'), { write: allow(true), remove: ['repos/s1/.git/critique/flows/feat.json'] }),
    gate(payload(s1, 'git commit -m x'), { write: { 'repos/s1/.git/critique/flows/feat.json': `${JSON.stringify({ phase: 'planning', repository: KEY })}\n`, 'repos/s1/.git/critique/flows/other.json': `${JSON.stringify({ phase: 'implementation', repository: 'github.com/o/r' })}\n`, 'repos/s1/.git/critique/flows/notes.txt': 'x' } }),
    gate(payload(s1, 'git commit -m x'), { write: { 'repos/s1/.git/critique/flows/bad.json': '{"phase":"done"}\n' }, roots: [LOG] }),
    gate(payload(s1, 'git commit -m x'), { write: { 'repos/s1/.git/critique/flows/feat.json': `${JSON.stringify({ phase: 'finishing', repository: KEY })}\n` } }),
    gate(payload(s1, 'git commit -m x'), { write: { [ALLOWLIST]: '{"version":1,"repositories":5}\n' }, roots: [LOG] }),
  ]
}

// The commit gate: risk, the ticket reviews of the staged change, and commits that reach beyond the index.
{
  const c = new Repo('c1')
  c.ops.push({ write: { 'src/a.ts': 'export const a = 1\n', 'README.md': '# readme\n' } }, { git: ['add', '-A'] }, { git: ['commit', '-q', '-m', 'base'] })
  const base = setup([c])
  const lowRisk = op1(c, { write: { 'README.md': '# readme\n\nmore\n' } }, { git: ['add', 'README.md'] })
  const lowHash = c.stagedHash()
  const head = c.rev('HEAD')
  const risky = op1(c, { write: { 'src/auth/login.ts': 'export const login = 1\n' } }, { git: ['add', 'src/auth/login.ts'] })
  const riskyHash = c.stagedHash()
  const unstaged = op1(c, { write: { 'src/a.ts': 'export const a = 2\n' } })
  const commit = (command = 'git commit -m x') => gate(payload(c, command))
  const reviews = (...lines) => ({ write: { ...allow(), ...index(c, REVIEWS, lines) } })
  scenarios['gate-commit'] = [
    base,
    { ...lowRisk, write: { ...lowRisk.write, ...allow() } },
    commit(),
    { ...gate(payload(c, 'git commit -m x')), write: index(c, TRIAGES, [triage({ labels: ['Risk:High'], staged_on: head }), triage({ labels: [], staged_on: 'other' })]) },
    { ...gate(payload(c, 'git commit -m x')), write: { ...index(c, TRIAGES, []), ...index(c, REVIEWS, [review(c, { staged_diff: lowHash })]) } },
    risky,
    commit(),
    commit('git commit --dry-run -m x'),
    { ...commit(), ...reviews(review(c, { staged_diff: riskyHash, review: 'fail', unfixed_critical: ['F1'] })) },
    { ...commit(), ...reviews(review(c, { staged_diff: riskyHash, review: 'fail', unfixed_critical: ['F1', 'F3'] })) },
    { ...commit(), ...reviews(review(c, { staged_diff: riskyHash, review: 'degraded', critic_failures: [{ critic_id: 'c1', reason: 'timeout' }, { critic_id: 'c2', reason: 'exit 1' }] })) },
    { ...commit(), ...reviews(review(c, { staged_diff: riskyHash, review: 'pass', degraded: true })) },
    { ...commit(), ...reviews(review(c, { staged_diff: riskyHash, review: 'degraded' })) },
    { ...commit(), ...reviews(review(c, { staged_diff: riskyHash, review: 'fail', unfixed_critical: ['F1'] }), review(c, { staged_diff: riskyHash }), review(c, { staged_diff: 'x', review: 'fail', repository: 'github.com/o/r' })) },
    { ...commit(), ...reviews(review(c, { staged_diff: riskyHash, repository: 'github.com/o/r' })) },
    { ...commit(), write: { ...index(c, REVIEWS, [review(c, { staged_diff: riskyHash }), '{"version":1']) }, roots: [LOG] },
    { ...commit(), write: { ...index(c, REVIEWS, [review(c, { staged_diff: riskyHash, review: 'maybe' })]) }, roots: [LOG] },
    { ...commit(), ...reviews(review(c, { staged_diff: riskyHash })) },
    { ...commit(), ...reviews(JSON.stringify({ ...JSON.parse(review(c, { staged_diff: riskyHash })), critic_failures: undefined })) },
    { ...commit(), write: { 'home/.config/agent-kit/critique-routing.json': '{"triage":null}\n' }, roots: [LOG] },
    { ...commit(), remove: ['home/.config/agent-kit/critique-routing.json'], write: { 'home/.config/agent-kit/critique-routing.json': '{"triage":{"changed_lines_threshold":0}}\n', ...index(c, REVIEWS, []), ...index(c, TRIAGES, [triage({ labels: ['risk:high'], staged_on: head })]) } },
    { ...commit(), remove: ['home/.config/agent-kit/critique-routing.json'], write: { 'home/.config/agent-kit/critique-routing.json': '{"triage":{"changed_lines_threshold":0,"risk_paths":[]}}\n' } },
    { ...commit(), remove: ['home/.config/agent-kit/critique-routing.json'], write: { ...index(c, TRIAGES, []), ...index(c, REVIEWS, [review(c, { staged_diff: riskyHash })]) } },
    { ...unstaged, write: { ...unstaged.write } },
    commit('git commit -am x'),
    commit('git commit -m x src/a.ts'),
    commit('git add -A && git commit -m x'),
    { ...commit('git stash && git commit -m x'), write: { 'repos/c1/new.sql': 'select 1;\n' } },
    hook('critique-commit-gate', payload(c, 'git commit -am x')),
    hook('critique-commit-gate', payload(c, 'git commit -m x')),
    hook('critique-commit-gate', payload(c, 'git commit -am x', { gate: 'pr' })),
    hook('critique-commit-gate', 'garbage', { roots: [LOG] }),
    hook('critique-pr-gate', payload(c, 'git commit -am x')),
  ]
}

// The PR gate: pushes and gh pr create, feature branches, and the feature review of the commit sent.
{
  const p = new Repo('p1')
  p.ops.push({ write: { 'a.md': 'a\n' } }, { git: ['add', '-A'] }, { git: ['commit', '-q', '-m', 'base'] }, { git: ['branch', 'feat/x'] })
  const base = setup([p])
  const head = p.rev('HEAD')
  const pr = (command, extra = {}) => gate(payload(p, command), extra)
  scenarios['gate-pr'] = [
    { ...base, write: { ...base.write, ...allow() } },
    pr('git push origin feat/x'),
    pr('git push origin HEAD:feat/y fix/z'),
    pr('git push origin main'),
    pr('git push'),
    pr('git push --all'),
    pr('git push origin --tags'),
    pr('git push --delete origin old'),
    pr('gh pr create --fill'),
    pr('git checkout -b feat/q && git push origin main'),
    pr('git commit -m x && gh pr create'),
    pr('git push origin main', { write: index(p, GATES, [gateRecord({ head_commit: head })]) }),
    pr('gh pr create --head main'),
    pr('git push origin main', { write: index(p, GATES, [gateRecord({ head_commit: head }), gateRecord({ head_commit: head, gate: 'fail', line: 'GATE: FAIL (2 critical open)' })]) }),
    pr('git push origin main', { write: index(p, GATES, [gateRecord({ head_commit: head, repository: 'github.com/o/r' })]) }),
    pr('git push origin feat/x:main', { write: index(p, GATES, [gateRecord({ head_commit: head })]) }),
    pr('git push origin nope:main'),
    pr('git push origin main', { write: index(p, GATES, [gateRecord({ head_commit: head, gate: 'maybe' })]), roots: [LOG] }),
    pr('git push origin main', { write: { ...index(p, GATES, []), 'home/.config/agent-kit/critique-routing.json': '{"gate":{"feature_branches":["main"]}}\n' } }),
    { ...pr('git push origin main'), remove: ['home/.config/agent-kit/critique-routing.json'] },
    op1(p, { git: ['checkout', '-q', '--detach'] }),
    pr('git push origin HEAD'),
    pr('git push'),
    hook('critique-pr-gate', payload(p, 'git push origin main')),
    hook('critique-pr-gate', payload(p, 'git push origin feat/x')),
  ]
}

// Merges: what a merge brings in, vouched or not, and a merge in progress that a commit concludes.
{
  const m = new Repo('m1')
  m.ops.push({ write: { 'src/a.ts': 'export const a = 1\n', 'README.md': '# readme\n', 'manifest.yaml': `files:\n  - sha256: ${'0'.repeat(64)}\n` } }, { git: ['add', '-A'] }, { git: ['commit', '-q', '-m', 'base'] })
  const base = setup([m])
  const branches = ops(
    m,
    { git: ['checkout', '-q', '-b', 'docs'] },
    { write: { 'docs/x.md': 'x\n' } },
    { git: ['add', '-A'] },
    { git: ['commit', '-q', '-m', 'docs'] },
    { git: ['checkout', '-q', '-b', 'risky', 'main'] },
    { write: { 'src/auth/login.ts': 'export const login = 1\n' } },
    { git: ['add', '-A'] },
    { git: ['commit', '-q', '-m', 'risky'] },
    { write: { 'src/auth/two.ts': 'export const two = 2\n' } },
    { git: ['add', '-A'] },
    { git: ['commit', '-q', '-m', 'risky two'] },
    { git: ['checkout', '-q', '-b', 'clash', 'main'] },
    { write: { 'README.md': '# clash\n', 'src/auth/clash.ts': 'export const c = 1\n' } },
    { git: ['add', '-A'] },
    { git: ['commit', '-q', '-m', 'clash'] },
    { git: ['checkout', '-q', '-b', 'revert', 'main'] },
    { write: { 'src/auth/tmp.ts': 'export const t = 1\n' } },
    { git: ['add', '-A'] },
    { git: ['commit', '-q', '-m', 'tmp'] },
    { write: { 'src/auth/tmp.ts': null } },
    { git: ['add', '-A'] },
    { git: ['commit', '-q', '-m', 'drop tmp'] },
    { git: ['checkout', '-q', '-b', 'merged', 'main'] },
    { write: { 'notes.md': 'n\n' } },
    { git: ['add', '-A'] },
    { git: ['commit', '-q', '-m', 'notes'] },
    { git: ['merge', '-q', '--no-ff', '-m', 'merge docs', 'docs'] },
    { git: ['checkout', '-q', '--orphan', 'lonely'] },
    { git: ['rm', '-q', '-rf', '--cached', '.'] },
    { write: { 'lonely.md': 'l\n' } },
    { git: ['add', 'lonely.md'] },
    { git: ['commit', '-q', '-m', 'lonely'] },
    { git: ['clean', '-q', '-fd'] },
    { git: ['checkout', '-q', '-f', 'main'] },
    { write: { 'README.md': '# main moved\n' } },
    { git: ['add', '-A'] },
    { git: ['commit', '-q', '-m', 'main moved'] },
  )
  const risky = m.rev('risky')
  const riskyFirst = m.rev('risky~1')
  const mainHead = m.rev('main')
  const merge = (command, extra = {}) => gate(payload(m, command), extra)
  scenarios['gate-merge'] = [
    { ...base, write: { ...base.write, ...allow() } },
    ...branches,
    merge('git merge docs'),
    merge('git merge --no-ff docs'),
    merge('git merge risky'),
    merge('git merge clash'),
    merge('git merge --squash risky'),
    merge('git merge revert', { write: index(m, TRIAGES, [triage({ labels: [], staged_on: 'elsewhere' }), triage({ labels: ['docs'], staged_on: mainHead }), triage({ labels: [], staged_on: 'elsewhere', repository: 'github.com/o/r' })]) }),
    merge('git merge lonely', { roots: [LOG] }),
    merge('git merge revert', { write: { ...index(m, TRIAGES, ['{"labels":5}']) }, roots: [LOG] }),
    { ...merge('git merge revert'), write: index(m, TRIAGES, []) },
    merge('git merge merged'),
    merge('git merge main'),
    merge('git pull'),
    merge('git pull origin main', { roots: [LOG] }),
    merge('git checkout docs && git merge risky'),
    merge('git merge risky', { write: index(m, REVIEWS, [review(m, { head_commit: risky })]) }),
    merge('git merge risky', { write: index(m, REVIEWS, [review(m, { head_commit: riskyFirst }), review(m, { head_commit: risky })]) }),
    merge('git merge risky', { write: index(m, REVIEWS, [review(m, { head_commit: risky, review: 'fail' })]) }),
    merge('git merge risky', { write: { ...index(m, REVIEWS, []), 'home/.config/agent-kit/critique-routing.json': '{"gate":{"merge_commits":0}}\n' } }),
    merge('git merge risky', { write: { 'home/.config/agent-kit/critique-routing.json': '{"gate":{"merge_commits":1}}\n' } }),
    { ...merge('git merge risky docs'), remove: ['home/.config/agent-kit/critique-routing.json'] },
    op1(m, { git: ['merge', '-q', '--no-commit', '--no-ff', 'risky'] }),
    merge('git commit -m merged'),
    merge('git merge --continue'),
    merge('git commit -m merged', { write: index(m, REVIEWS, [review(m, { head_commit: riskyFirst }), review(m, { head_commit: risky })]) }),
    ...ops(m, { write: { 'manifest.yaml': `files:\n  - sha256: ${'1'.repeat(64)}\n`, 'dist/app.js': 'x\n', 'gen.ts': '// @generated\nexport const g = 1\n' } }, { git: ['add', '-A'] }),
    merge('git commit -m merged', { write: index(m, REVIEWS, [review(m, { head_commit: riskyFirst }), review(m, { head_commit: risky })]) }),
    ...ops(m, { write: { 'src/new.sql': 'select 1;\n', 'manifest.yaml': 'files: []\n' } }, { git: ['add', '-A'] }),
    merge('git commit -m merged'),
    hook('critique-commit-gate', payload(m, 'git commit -m merged'), { write: index(m, REVIEWS, []) }),
    // A merge of an unrelated history brings in a root commit, which nothing vouches for.
    ...ops(m, { git: ['reset', '-q', '--hard'] }, { git: ['merge', '-q', '--no-commit', '--allow-unrelated-histories', 'lonely'] }),
    merge('git commit -m lonely'),
  ]
}

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
