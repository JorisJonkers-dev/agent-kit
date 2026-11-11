import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  FAKE_BRIEF_FILE,
  FakeEnv,
  FakeFiles,
  FakeGit,
  FakeGrants,
  FakeKept,
  FakeLedger,
  FakeProof,
  FakeWorkflows,
  FixedClock,
  type FakeRepository,
} from '../../../../test/model-gateway/harness.js'
import type { ProcessCommand, ProcessResult } from '../../../ports/index.js'
import type { GitRemote } from '../../../shared-kernel/index.js'
import { CODEX_WRITE_RULE, WRITE_RULE } from '../domain/run.js'
import { GrantWriteCommand } from './grants.js'
import type { ModelGatewayDependencies, ModelGatewayProcessPort } from './ports.js'
import { RunCommand } from './run.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

/** The defaults agent-kit ships, which the bundle reads from its own folder. */
const SHIPPED_DEFAULTS = join(import.meta.dirname, '../../../../../../skills/workstation/model-gateway/defaults.json')

const KEY = 'github.com/jorisjonkers-dev/agent-kit'
const REMOTE: GitRemote = { name: 'origin', url: 'git@github.com:JorisJonkers-dev/agent-kit.git' }

/** What `codex exec --json` prints for a run that answered: one agent message, and the tokens its turn counted. */
const CODEX_STDOUT = [
  JSON.stringify({ thread_id: 't-1', type: 'thread.started' }),
  JSON.stringify({ item: { id: 'item_0', text: 'did it\nSUMMARY: did it', type: 'agent_message' }, type: 'item.completed' }),
  JSON.stringify({ type: 'turn.completed', usage: { cached_input_tokens: 0, input_tokens: 1000, output_tokens: 200 } }),
].join('\n')

/** What `claude -p --output-format json` prints for a run that answered. */
const CLAUDE_STDOUT = JSON.stringify({
  is_error: false,
  result: 'did it\nSUMMARY: did it',
  subtype: 'success',
  total_cost_usd: 0.01,
  type: 'result',
  usage: { input_tokens: 10, output_tokens: 40 },
})

/**
 * A client that answers every launch with the same events, and every check
 * with exit 0. It starts nothing and spends nothing.
 */
class StubClient implements ModelGatewayProcessPort {
  readonly commands: ProcessCommand[] = []

  which(): Promise<boolean> {
    return Promise.resolve(true)
  }

  resolve(binary: string): Promise<string | undefined> {
    return Promise.resolve(`/usr/bin/${binary}`)
  }

  exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    if (command.command === 'sh') return Promise.resolve({ exitCode: 0, stderr: '', stdout: '' })
    if (command.command === 'claude') return Promise.resolve({ exitCode: 0, stderr: '', stdout: CLAUDE_STDOUT })
    return Promise.resolve({ exitCode: 0, stderr: '', stdout: CODEX_STDOUT })
  }
}

/**
 * A git that loses the repository's common directory once the run's worktree
 * exists, as a common directory that goes away mid-run does: the run's own
 * directories are made before the worktree, so only the Codex worker's own
 * question about it is refused.
 */
class CommonDirLostAfterWorktree extends FakeGit {
  override async commonDir(path: string): Promise<string> {
    const made = await readdir(join(path, '.git', 'model-gateway', 'worktrees')).catch(() => undefined)
    if (made !== undefined) throw new Error('the common directory is gone')
    return super.commonDir(path)
  }
}

interface Workspace {
  readonly deps: ModelGatewayDependencies
  readonly git: FakeGit
  readonly process: StubClient
  readonly repo: string
  /** Writes the grant a metered-employer Codex run needs for a workflow. */
  grant(workflow: string): Promise<void>
}

interface WorkspaceOptions {
  /** What the repository keeps in its `codex-brief.md`; none when it keeps none. */
  readonly brief?: string
  /** What git says about the repository when it cannot name its git directory or its common directory. */
  readonly repository?: { readonly commonDirError?: string; readonly gitDirError?: string }
  /** A git that answers differently, for a test of one that loses the common directory mid-run. */
  readonly git?: (repositories: Readonly<Record<string, FakeRepository>>) => FakeGit
}

/** A machine with one repository, its allowlist entry, and a client that only plays launches. */
async function workspace(options: WorkspaceOptions = {}): Promise<Workspace> {
  const dir = await mkdtemp(join(tmpdir(), 'model-gateway-codex-worker-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const home = join(dir, 'home')
  const repo = join(dir, 'repo')
  await mkdir(repo, { recursive: true })
  if (options.brief !== undefined) await writeFile(join(repo, FAKE_BRIEF_FILE), options.brief)
  const defaultsPath = join(dir, 'defaults.json')
  await writeFile(defaultsPath, await readFile(SHIPPED_DEFAULTS, 'utf8'))
  const config = join(home, '.config', 'agent-kit')
  await mkdir(config, { recursive: true })
  await writeFile(
    join(config, 'critique-allowlist.json'),
    JSON.stringify({
      repositories: { [KEY]: { code: true, families: ['anthropic', 'openai'], history: true, updated_at: '2026-11-04T00:00:00.000Z' } },
      version: 1,
    }),
  )
  const repositories: Readonly<Record<string, FakeRepository>> = {
    [repo]: { remotes: [REMOTE], root: repo, ...options.repository },
  }
  const git = options.git === undefined ? new FakeGit(repositories) : options.git(repositories)
  const process = new StubClient()
  const deps: ModelGatewayDependencies = {
    clock: new FixedClock(),
    defaultsPath,
    env: new FakeEnv({ HOME: home }),
    files: new FakeFiles(),
    git,
    grants: new FakeGrants(),
    kept: new FakeKept(),
    ledger: new FakeLedger(),
    process,
    proof: new FakeProof(),
    sources: {},
    statuses: {},
    workflows: new FakeWorkflows(),
  }
  return {
    deps,
    git,
    process,
    repo,
    grant: async (workflow) => {
      await new GrantWriteCommand(deps).run({
        answer: 'yes, five dollars',
        cap_usd: 5,
        client: 'codex',
        question: 'may codex work on this?',
        workflow,
      })
    },
  }
}

/** A Codex writing run of the test workflow, one attempt, off `main`. */
function writingRun(overrides: Readonly<Record<string, unknown>> = {}): Record<string, unknown> {
  return {
    base: 'main',
    check: 'true',
    client: 'codex',
    escalate: false,
    mode: 'write',
    model: 'gpt-5.5',
    prompt: 'change the thing',
    time_limit_s: 900,
    workflow: 'plan-1',
    ...overrides,
  }
}

/** The one launch of a client, which a run makes once. */
function launchOf(process: StubClient, client: string): ProcessCommand {
  const command = process.commands.find((one) => one.command === client)
  if (command === undefined) throw new Error(`the run launched no ${client}`)
  return command
}

/** The prompt a client was sent on its stdin. */
function promptOf(process: StubClient, client: string): string {
  return launchOf(process, client).stdin ?? ''
}

/** The directories a launch names with `--add-dir`, in order. */
function addDirs(command: ProcessCommand): string[] {
  return command.args.flatMap((arg, at) => (arg === '--add-dir' ? [command.args[at + 1] ?? ''] : []))
}

/** The worktree the run made, which its git directories are worked out from. */
function worktreeOf(git: FakeGit): string {
  const made = git.worktrees[0]
  if (made === undefined) throw new Error('the run made no worktree')
  return made.path
}

describe('a Codex writing run', () => {
  it('lets its sandbox write the worktree’s git directory and the repository’s shared objects, refs and logs, never the common directory itself', async () => {
    const w = await workspace()
    await w.grant('plan-1')

    const result = await new RunCommand(w.deps).run({ repo: w.repo, ...writingRun() })

    expect(result).toMatchObject({ status: 'ok' })
    const common = join(w.repo, '.git')
    const dirs = addDirs(launchOf(w.process, 'codex'))
    expect(dirs).toEqual([
      join(common, 'worktrees', basename(worktreeOf(w.git))),
      join(common, 'objects'),
      join(common, 'refs'),
      join(common, 'logs'),
    ])
    expect(dirs).not.toContain(common)
  })

  it('sends the repository’s brief above the prompt, then the rule that the delegate commits its own change', async () => {
    const brief = '# Brief\n\nWork in small steps.'
    const w = await workspace({ brief })
    await w.grant('plan-1')

    await new RunCommand(w.deps).run({ repo: w.repo, ...writingRun() })

    const prompt = promptOf(w.process, 'codex')
    expect(prompt.slice(0, brief.length)).toBe(brief)
    expect(prompt).toContain(`${brief}\n\n---\n\nchange the thing`)
    expect(prompt).toContain(CODEX_WRITE_RULE)
    expect(prompt).not.toContain('Do not commit.')
  })

  it('sends no brief when the repository keeps none, and still the commit rule', async () => {
    const w = await workspace()
    await w.grant('plan-1')

    await new RunCommand(w.deps).run({ repo: w.repo, ...writingRun() })

    const prompt = promptOf(w.process, 'codex')
    expect(prompt.startsWith('change the thing')).toBe(true)
    expect(prompt).not.toContain('---')
    expect(prompt).toContain(CODEX_WRITE_RULE)
  })

  it('cuts a brief longer than 20000 characters at 20000', async () => {
    const w = await workspace({ brief: 'x'.repeat(20_001) })
    await w.grant('plan-1')

    await new RunCommand(w.deps).run({ repo: w.repo, ...writingRun() })

    const prompt = promptOf(w.process, 'codex')
    expect(prompt.slice(0, 20_000)).toBe('x'.repeat(20_000))
    expect(prompt.slice(20_000, 20_001)).toBe('\n')
    expect(prompt).not.toContain('x'.repeat(20_001))
  })

  it('runs without --add-dir when git cannot name the worktree’s git directory', async () => {
    const w = await workspace({ repository: { gitDirError: 'the git directory is gone' } })
    await w.grant('plan-1')

    const result = await new RunCommand(w.deps).run({ repo: w.repo, ...writingRun() })

    expect(result).toMatchObject({ status: 'ok' })
    expect(addDirs(launchOf(w.process, 'codex'))).toEqual([])
  })

  it('runs without --add-dir when git cannot name the repository’s common directory', async () => {
    const w = await workspace({ git: (repositories) => new CommonDirLostAfterWorktree(repositories) })
    await w.grant('plan-1')

    const result = await new RunCommand(w.deps).run({ repo: w.repo, ...writingRun() })

    expect(result).toMatchObject({ status: 'ok' })
    expect(addDirs(launchOf(w.process, 'codex'))).toEqual([])
  })

  it('lists what the delegate staged against the run’s own base', async () => {
    const w = await workspace()
    await w.grant('plan-1')

    await new RunCommand(w.deps).run({ repo: w.repo, ...writingRun({ base: 'release/1.2' }) })

    expect(w.git.stagedBases).toEqual(['release/1.2'])
  })

  it('gives a Claude Code writing run no --add-dir, the old rule and no brief', async () => {
    const w = await workspace({ brief: '# Brief' })

    const result = await new RunCommand(w.deps).run({
      repo: w.repo,
      ...writingRun({ client: 'claude', model: 'claude-sonnet-5-5' }),
    })

    expect(result).toMatchObject({ status: 'ok' })
    const command = launchOf(w.process, 'claude')
    expect(command.args).not.toContain('--add-dir')
    const prompt = command.stdin ?? ''
    expect(prompt).toContain(WRITE_RULE)
    expect(prompt).toContain('Do not commit.')
    expect(prompt).not.toContain('# Brief')
  })

  it('gives a Codex read-only run no --add-dir, no brief and no writing rule', async () => {
    const w = await workspace({ brief: '# Brief' })
    await w.grant('plan-1')

    const result = await new RunCommand(w.deps).run({
      client: 'codex',
      escalate: false,
      mode: 'read-only',
      model: 'gpt-5.5',
      prompt: 'read the code',
      repo: w.repo,
      time_limit_s: 900,
      workflow: 'plan-1',
    })

    expect(result).toMatchObject({ status: 'ok' })
    const command = launchOf(w.process, 'codex')
    expect(command.args).not.toContain('--add-dir')
    const prompt = command.stdin ?? ''
    expect(prompt).not.toContain('# Brief')
    expect(prompt).not.toContain(CODEX_WRITE_RULE)
    expect(prompt).not.toContain('Do not commit.')
  })
})
