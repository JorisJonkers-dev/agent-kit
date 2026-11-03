import { execFileSync } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  fenced,
  finding,
  harness,
  isolatedExport,
  vibeStdout,
  type Harness,
} from '../../test/critique/harness.js'
import { CRITIQUE_USAGE, defaultCritiqueRuntime, runCritiqueCli, type CritiqueCliRuntime } from './index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

function cli(h: Harness, stdin = ''): CritiqueCliRuntime {
  return { commands: h.commands, readFile: (path) => readFile(path, 'utf8'), readStdin: () => Promise.resolve(stdin) }
}

async function run(runtime: CritiqueCliRuntime, ...argv: string[]): Promise<Record<string, unknown>> {
  const result = await runCritiqueCli(argv, runtime)
  expect(result.exitCode).toBe(0)
  return JSON.parse(result.stdout) as Record<string, unknown>
}

describe('critique runner command line', () => {
  // The one end-to-end test: a whole critique through the command line. Every
  // other test covers one command.
  it('runs a whole critique, from allow to record, with JSON in and JSON out', async () => {
    const h = await harness({
      scripts: [{ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [finding()] })) }],
    })
    cleanups.push(h.cleanup)
    const runtime = cli(h)

    expect(await run(runtime, 'allow', '--json', JSON.stringify({ families: ['mistral'], repo: h.repo }))).toMatchObject({
      status: 'allowed',
    })
    const inputFile = join(h.home, 'prepare.json')
    await h.writeText(inputFile, JSON.stringify({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' }))
    const prepared = await run(runtime, 'prepare', '--input', inputFile)
    expect(prepared).toMatchObject({ status: 'ready' })
    const fan = await run(cli(h, JSON.stringify({ run_dir: prepared.run_dir })), 'fan-out')
    expect(fan).toMatchObject({ findings: [{ id: 'F1' }] })
    const recorded = await run(
      runtime,
      'record',
      '--json',
      JSON.stringify({ run_dir: prepared.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] }),
    )
    expect(recorded).toMatchObject({ counts: { accepted: 1 }, status: 'recorded' })
  })

  it('exits 3 when check-tickets finds a dependency cycle, so a caller sees the failure without reading the JSON', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)
    await h.writeText(join(h.repo, '.scratch', 'widgets', 'issues', '01-schema.md'), '# 01: Schema\n\n**Blocked by:** 01\n')

    const result = await runCritiqueCli(['check-tickets', '--json', JSON.stringify({ repo: h.repo, spec: '.scratch/widgets/spec.md' })], cli(h))

    expect(result.exitCode).toBe(3)
    expect(JSON.parse(result.stdout)).toMatchObject({ cycles: [['01']], status: 'failed' })
  })

  it('exits 0 when check-tickets finds no cycle', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    expect(await run(cli(h), 'check-tickets', '--json', JSON.stringify({ repo: h.repo, spec: '.scratch/widgets/spec.md' }))).toMatchObject({
      status: 'passed',
    })
  })

  it('exits 3 when check-adrs finds a structural error in the decision log', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)
    await h.writeText(join(h.repo, 'docs', 'adr', '0001-a.md'), '# A\n\nWe decided.\n')
    await h.writeText(join(h.repo, 'docs', 'adr', '0003-c.md'), '# C\n\nWe decided again.\n')

    const result = await runCritiqueCli(['check-adrs', '--json', JSON.stringify({ repo: h.repo })], cli(h))

    expect(result.exitCode).toBe(3)
    expect(JSON.parse(result.stdout)).toMatchObject({ numbering: { gaps: [2] }, status: 'failed' })
  })

  it('reports critique stats from the ledger', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    expect(await run(cli(h), 'stats', '--json', '{}')).toMatchObject({ cells: [], status: 'reported' })
  })

  it("prints a flow's state and its next step", async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    expect(await run(cli(h), 'state', '--json', JSON.stringify({ draft: 'widgets', repo: h.repo }))).toMatchObject({
      next: { command: '/grill-with-docs', kind: 'command' },
      status: 'read',
    })
  })

  it("lists this machine's setup checklist", async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    expect(await run(cli(h), 'setup', '--json', JSON.stringify({ action: 'list', repo: h.repo }))).toMatchObject({
      repository: { status: 'not-opted-in' },
      status: 'listed',
    })
  })

  it("prints a flow's Vibe spend and today's Antigravity launches against their caps", async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    expect(await run(cli(h), 'budget', '--json', JSON.stringify({ draft: 'widgets', repo: h.repo }))).toMatchObject({
      antigravity: { daily_cap: 20, invocations: 0 },
      status: 'read',
      vibe: { cap_usd: 5, spent_usd: 0 },
    })
  })

  it('exits 0 with the decision when triage calls a change risky', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)
    await h.writeText(join(h.repo, 'src', 'auth', 'session.ts'), 'export const ttl = 60\n')
    await h.git.commit()

    expect(
      await run(cli(h), 'triage', '--json', JSON.stringify({ base: 'HEAD~1', repo: h.repo, ticket: '.scratch/widgets/spec.md' })),
    ).toMatchObject({ signal: { kind: 'risk-path', path: 'src/auth/session.ts' }, status: 'risky' })
  })

  it('exits 0 with the deny a hook prints when the gate blocks', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)
    await h.commands.allow.run({ code: true, families: ['mistral'], repo: h.repo })
    await h.writeText(join(h.repo, 'src', 'auth', 'session.ts'), 'export const ttl = 60\n')
    await h.git.stage()
    const hook = { cwd: h.repo, hook_event_name: 'PreToolUse', tool_input: { command: 'git commit -m x' }, tool_name: 'Bash' }

    expect(await run(cli(h, JSON.stringify(hook)), 'gate')).toMatchObject({
      hook_output: { hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny' } },
      status: 'blocked',
    })
  })

  it('allows, and exits 0, when the gate is given input that is not JSON', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    expect(await run(cli(h, '{not json'), 'gate')).toMatchObject({ failure: 'error', reason: 'failed-open', status: 'allowed' })
  })

  it('as a hook, prints only the deny the agent reads when the commit gate blocks, and exits 0', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)
    await h.commands.allow.run({ code: true, families: ['mistral'], repo: h.repo })
    await h.writeText(join(h.repo, 'src', 'auth', 'session.ts'), 'export const ttl = 60\n')
    await h.git.stage()
    const payload = (command: string) => JSON.stringify({ cwd: h.repo, hook_event_name: 'PreToolUse', tool_input: { command }, tool_name: 'Bash' })

    const commit = await runCritiqueCli(['hook', 'critique-commit-gate'], cli(h, payload('git commit -m x')))
    const prOnly = await runCritiqueCli(['hook', 'critique-commit-gate'], cli(h, payload('gh pr create')))

    expect([commit.exitCode, commit.stderr, JSON.parse(commit.stdout)]).toEqual([
      0,
      '',
      {
        hookSpecificOutput: {
          hookEventName: 'PreToolUse',
          permissionDecision: 'deny',
          permissionDecisionReason:
            'Commit blocked: the staged change is risky (src/auth/session.ts is a risk path) and no passing /review-ticket run matches it. Run /review-ticket, then commit.',
        },
      },
    ])
    expect(prOnly).toEqual({ exitCode: 0, stderr: '', stdout: '' })
  })

  it('as the PR gate hook, blocks a PR with no feature review and leaves commits to the commit gate', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)
    await h.commands.allow.run({ code: true, families: ['mistral'], repo: h.repo })
    await h.writeText(join(h.repo, 'src', 'auth', 'session.ts'), 'export const ttl = 60\n')
    await h.git.stage()
    const payload = (command: string) => JSON.stringify({ cwd: h.repo, tool_input: { command }, tool_name: 'Bash' })

    const pr = await runCritiqueCli(['hook', 'critique-pr-gate'], cli(h, payload('gh pr create')))
    const commit = await runCritiqueCli(['hook', 'critique-pr-gate'], cli(h, payload('git commit -m x')))

    expect([pr.exitCode, (JSON.parse(pr.stdout) as { hookSpecificOutput: { permissionDecisionReason: string } }).hookSpecificOutput.permissionDecisionReason]).toEqual([
      0,
      'PR blocked: no /review-feature run has recorded a gate for c1. Run /review-feature, then open the PR.',
    ])
    expect(commit).toEqual({ exitCode: 0, stderr: '', stdout: '' })
  })

  it('as the session-start hook, prints its one line as plain text, or nothing', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)
    const payload = JSON.stringify({ cwd: h.repo, hook_event_name: 'SessionStart', source: 'startup' })
    const before = await runCritiqueCli(['hook', 'critique-session-start'], cli(h, payload))
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })

    const after = await runCritiqueCli(['hook', 'critique-session-start'], cli(h, payload))

    expect([before, after]).toEqual([
      { exitCode: 0, stderr: '', stdout: '' },
      { exitCode: 0, stderr: '', stdout: 'Critique: /flow setup has not checked this machine yet (10 of 10 checks never ran). Run /flow setup.\n' },
    ])
    expect(await run(cli(h, payload), 'session-start')).toMatchObject({ reason: 'setup-missing', status: 'suggest' })
  })

  it('as a hook, prints nothing and exits 0 on a payload that is not JSON', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    expect(await runCritiqueCli(['hook', 'critique-commit-gate'], cli(h, '{not json'))).toEqual({ exitCode: 0, stderr: '', stdout: '' })
    expect(await runCritiqueCli(['hook', 'critique-session-start'], cli(h, '{not json'))).toEqual({ exitCode: 0, stderr: '', stdout: '' })
  })

  it('as a hook, prints nothing and exits 0 when the payload cannot be read', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)
    const broken: CritiqueCliRuntime = { ...cli(h), readStdin: () => Promise.reject(new Error('stdin closed')) }

    expect(await runCritiqueCli(['hook', 'critique-pr-gate'], broken)).toEqual({ exitCode: 0, stderr: '', stdout: '' })
  })

  it('refuses a hook ADR-0002 does not name with exit 1, which both agents treat as a hook error that does not block', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    const result = await runCritiqueCli(['hook', 'after-skill'], cli(h))
    const extra = await runCritiqueCli(['hook', 'critique-pr-gate', 'x'], cli(h))

    expect([result.exitCode, result.stdout, result.stderr]).toEqual([1, '', CRITIQUE_USAGE])
    expect(extra.exitCode).toBe(1)
  })

  it('prints a refusal as a normal result', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    expect(
      await run(cli(h), 'prepare', '--json', JSON.stringify({ orchestrator_family: 'anthropic', repo: h.repo, spec: 'x', stage: 'spec' })),
    ).toMatchObject({ reason: 'not-opted-in', status: 'refused' })
  })

  it('reports input that is not JSON as an invalid-input error with exit code 1', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    const result = await runCritiqueCli(['allow', '--json', '{not json'], cli(h))

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'invalid-input', status: 'error' })
    expect(result.stderr).toMatch(/^critique allow: /)
  })

  it('reports the code of the error a command raises', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })

    const result = await runCritiqueCli(
      ['prepare', '--json', JSON.stringify({ orchestrator_family: 'anthropic', repo: h.repo, spec: '../x.md', stage: 'spec' })],
      cli(h),
    )

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({ code: 'outside-repository', message: '../x.md is outside the repository', status: 'error' })
  })

  it('reports an error the runner did not expect with the code unexpected', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    const result = await runCritiqueCli(['allow', '--input', join(h.home, 'missing.json')], cli(h))

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'unexpected', status: 'error' })
  })

  it.each([[[]], [['explode']], [['prepare', '--input']], [['prepare', '--yaml', 'x']], [['prepare', '--json', '{}', 'extra']]])(
    'prints usage for %j',
    async (argv) => {
      const h = await harness()
      cleanups.push(h.cleanup)

      expect(await runCritiqueCli(argv, cli(h))).toEqual({ exitCode: 2, stderr: CRITIQUE_USAGE, stdout: '' })
    },
  )

  it('prints usage on --help', async () => {
    const h = await harness()
    cleanups.push(h.cleanup)

    expect(await runCritiqueCli(['--help'], cli(h))).toEqual({ exitCode: 0, stderr: '', stdout: CRITIQUE_USAGE })
  })
})

interface RealRepository {
  readonly base: string
  readonly repo: string
  readonly allowlistPath: string
  readonly runtime: CritiqueCliRuntime
}

/** A real git repository with an origin remote, and the runner wired to the real adapters. */
async function realRepository(): Promise<RealRepository> {
  const base = await mkdtemp(join(tmpdir(), 'critique-cli-'))
  cleanups.push(() => rm(base, { force: true, recursive: true }))
  const repo = join(base, 'repo')
  const home = join(base, 'home')
  await mkdir(join(repo, '.scratch', 'widgets'), { recursive: true })
  await writeFile(join(repo, '.scratch', 'widgets', 'spec.md'), '# Widgets\n')
  await writeFile(join(repo, 'CONTEXT.md'), '# Words\n')
  await mkdir(join(repo, 'docs', 'agents'), { recursive: true })
  await writeFile(join(repo, 'docs', 'agents', 'issue-tracker.md'), '# Issue tracker: Local Markdown\n')
  execFileSync('git', ['init', '-q', repo])
  execFileSync('git', ['-C', repo, 'remote', 'add', 'origin', 'https://github.com/example/widgets.git'])
  return {
    allowlistPath: join(home, '.config', 'agent-kit', 'critique-allowlist.json'),
    base,
    repo,
    runtime: defaultCritiqueRuntime(ASSETS_ROOT, () => Promise.resolve(''), { HOME: home, PATH: process.env.PATH }),
  }
}

describe('the real adapters', () => {
  it('keep the allowlist in HOME, keyed by the real repository origin', async () => {
    const real = await realRepository()
    await writeFile(join(real.base, 'allow.json'), JSON.stringify({ families: ['mistral'], repo: real.repo }))

    await run(real.runtime, 'allow', '--input', join(real.base, 'allow.json'))

    expect(JSON.parse(await readFile(real.allowlistPath, 'utf8'))).toMatchObject({
      repositories: { 'github.com/example/widgets': { families: ['mistral'] } },
    })
  })

  it('freeze the packet in the git common directory of a real repository', async () => {
    const real = await realRepository()
    await mkdir(join(real.allowlistPath, '..'), { recursive: true })
    await writeFile(
      real.allowlistPath,
      JSON.stringify({ repositories: { 'github.com/example/widgets': { families: ['mistral'], updated_at: 'x' } }, version: 1 }),
    )

    const prepared = await run(
      real.runtime,
      'prepare',
      '--json',
      JSON.stringify({ orchestrator_family: 'anthropic', repo: real.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' }),
    )

    expect(String(prepared.packet_dir)).toContain(join('.git', 'critique'))
    expect(await readFile(join(String(prepared.packet_dir), 'artifact', 'spec.md'), 'utf8')).toBe('# Widgets\n')
  })

  it('gate a commit of a real repository from its staged change, with no model and no network', async () => {
    const real = await realRepository()
    const git = (...args: string[]) => execFileSync('git', ['-C', real.repo, ...args], { env: { ...process.env, GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@example.com', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@example.com' } })
    git('add', '-A')
    git('commit', '-q', '-m', 'base')
    await mkdir(join(real.allowlistPath, '..'), { recursive: true })
    await writeFile(
      real.allowlistPath,
      JSON.stringify({ repositories: { 'github.com/example/widgets': { code: true, families: ['mistral'], updated_at: 'x' } }, version: 1 }),
    )
    await mkdir(join(real.repo, 'src', 'auth'), { recursive: true })
    await writeFile(join(real.repo, 'src', 'auth', 'session.ts'), 'export const ttl = 60\n')
    const hook = (command: string) => JSON.stringify({ cwd: real.repo, tool_input: { command }, tool_name: 'Bash' })

    expect(await run(real.runtime, 'gate', '--json', hook('git commit -m x'))).toMatchObject({ reason: 'low-risk' })
    expect(await run(real.runtime, 'gate', '--json', hook('git add -A && git commit -m x'))).toMatchObject({ reason: 'unstaged' })
    git('add', '-A')
    expect(await run(real.runtime, 'gate', '--json', hook('git commit -m x'))).toMatchObject({ reason: 'unreviewed', status: 'blocked' })
    expect(await run(real.runtime, 'gate', '--json', hook('gh pr create'))).toMatchObject({ reason: 'no-feature-review', status: 'blocked' })
    expect(await run(real.runtime, 'gate', '--json', hook('git push origin feat/widgets'))).toMatchObject({ reason: 'feature-branch' })
  })

  it('defaults to the process environment', () => {
    expect(defaultCritiqueRuntime(ASSETS_ROOT, () => Promise.resolve('')).commands).toBeDefined()
  })
})
