import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

import type {
  Grant,
  GrantChange,
  LedgerChange,
  LedgerEntry,
  ModelGatewayFilesPort,
  KeptWorktree,
  ModelGatewayGitPort,
  ModelGatewayGrantsPort,
  ModelGatewayKeptPort,
  ModelGatewayLedgerPort,
  ModelGatewayProcessPort,
  ModelGatewayProofPort,
  ModelGatewayWorkflowsPort,
  ProofFiles,
  WorkflowChange,
  WorkflowRecord,
} from '../../src/contexts/model-gateway/index.js'
import type { ClockPort, EnvPort, ProcessCommand, ProcessResult } from '../../src/ports/index.js'
import type { GitRemote } from '../../src/shared-kernel/index.js'

/** The environment a gateway test gives it: exactly the variables it names. */
export class FakeEnv implements EnvPort {
  constructor(private readonly values: Readonly<Record<string, string>>) {}

  get(name: string): string | undefined {
    return this.values[name]
  }

  require(name: string): string {
    const value = this.get(name)
    if (value === undefined) throw new Error(`required environment variable is missing: ${name}`)
    return value
  }

  all(): Readonly<Record<string, string>> {
    return this.values
  }
}

/** A PATH that holds exactly the programs the test names, and the listings they answer. */
export class FakeProcess implements ModelGatewayProcessPort {
  readonly asked: string[] = []
  readonly ran: ProcessCommand[] = []

  constructor(
    private readonly installed: readonly string[],
    private readonly listings: Readonly<Record<string, ProcessResult>> = {},
  ) {}

  which(binary: string): Promise<boolean> {
    this.asked.push(binary)
    return Promise.resolve(this.installed.includes(binary))
  }

  exec(command: ProcessCommand): Promise<ProcessResult> {
    this.ran.push(command)
    const result = this.listings[command.command]
    if (result === undefined) return Promise.reject(new Error(`cannot run ${command.command}`))
    return Promise.resolve(result)
  }
}

/**
 * One path the fake git knows: a repository with its root, its common
 * directory and its remotes, or a path git cannot read, with the message it
 * fails with.
 */
export type FakeRepository =
  | {
      readonly root: string
      readonly remotes: readonly GitRemote[]
      readonly commonDir?: string
      /** What git says when it cannot name the common directory, as a worktree git refuses. */
      readonly commonDirError?: string
      /** What git says when it cannot add a worktree, as a repository with no commit does. */
      readonly worktreeError?: string
      /** What git says when it cannot remove a worktree. */
      readonly removeError?: string
    }
  | { readonly error: string }

/** One worktree call a fake git was asked for. */
export interface FakeWorktreeCall {
  /** `create` is a worktree on a new branch off a base; `add` is a detached one. */
  readonly action: 'add' | 'create' | 'remove'
  readonly repo: string
  readonly path: string
  readonly branch?: string
  readonly base?: string
}

/** What a fake git was asked to delete: a branch of a repository. */
export interface FakeBranchCall {
  readonly repo: string
  readonly branch: string
}

/** The file a fake worktree holds, as the checkout of HEAD a real one holds. */
export const FAKE_CHECKOUT_FILE = 'CHECKED-OUT'

/**
 * A git that knows exactly the paths the test names; any other path is in no
 * repository. A worktree it adds is the directory the caller made, with a
 * marker file in it; one it removes is gone.
 */
export class FakeGit implements ModelGatewayGitPort {
  readonly worktrees: FakeWorktreeCall[] = []
  /** Every git operation asked for, by the port's method name: a test asserts which ones a run is allowed to make. */
  readonly calls: string[] = []
  readonly stagedIn: string[] = []
  readonly deletedBranches: FakeBranchCall[] = []
  /** What `stageAll` fails with, as a worktree whose `.git` file the delegate overwrote does; none by default. */
  stageError: string | undefined
  /** What `stagedFiles` fails with, as a corrupt index does; none by default. */
  stagedFilesError: string | undefined
  /** What `deleteBranch` fails with, as a branch checked out elsewhere does; none by default. */
  deleteBranchError: string | undefined

  constructor(private readonly repositories: Readonly<Record<string, FakeRepository>>) {}

  async createWorktree(repo: string, path: string, branch: string, base: string): Promise<void> {
    this.calls.push('createWorktree')
    this.worktrees.push({ action: 'create', base, branch, path, repo })
    const entry = this.repositories[repo]
    if (entry !== undefined && 'root' in entry && entry.worktreeError !== undefined) throw new Error(entry.worktreeError)
    await mkdir(path, { recursive: true })
    await writeFile(join(path, FAKE_CHECKOUT_FILE), base)
  }

  /** Stages everything: the fake worktree's files are what `stagedFiles` then names. */
  stageAll(path: string): Promise<void> {
    this.calls.push('stageAll')
    if (this.stageError !== undefined) return Promise.reject(new Error(this.stageError))
    this.stagedIn.push(path)
    return Promise.resolve()
  }

  /** Every file in the worktree but the checkout marker, relative to it and sorted: what a real `git add -A` would stage. */
  async stagedFiles(path: string): Promise<readonly string[]> {
    this.calls.push('stagedFiles')
    if (this.stagedFilesError !== undefined) throw new Error(this.stagedFilesError)
    const entries = await readdir(path, { recursive: true, withFileTypes: true })
    return entries
      .filter((entry) => entry.isFile() && entry.name !== FAKE_CHECKOUT_FILE)
      .map((entry) => join(entry.parentPath, entry.name).slice(path.length + 1))
      .sort()
  }

  deleteBranch(repo: string, branch: string): Promise<void> {
    this.calls.push('deleteBranch')
    if (this.deleteBranchError !== undefined) return Promise.reject(new Error(this.deleteBranchError))
    this.deletedBranches.push({ branch, repo })
    return Promise.resolve()
  }

  async addWorktree(repo: string, path: string): Promise<void> {
    this.calls.push('addWorktree')
    this.worktrees.push({ action: 'add', path, repo })
    const entry = this.repositories[repo]
    if (entry !== undefined && 'root' in entry && entry.worktreeError !== undefined) throw new Error(entry.worktreeError)
    await writeFile(join(path, FAKE_CHECKOUT_FILE), 'HEAD')
  }

  async removeWorktree(repo: string, path: string): Promise<void> {
    this.calls.push('removeWorktree')
    this.worktrees.push({ action: 'remove', path, repo })
    const entry = this.repositories[repo]
    if (entry !== undefined && 'root' in entry && entry.removeError !== undefined) throw new Error(entry.removeError)
    await rm(path, { force: true, recursive: true })
  }

  root(path: string): Promise<string | undefined> {
    this.calls.push('root')
    const entry = this.repositories[path]
    if (entry === undefined) return Promise.resolve(undefined)
    if ('error' in entry) return Promise.reject(new Error(entry.error))
    return Promise.resolve(entry.root)
  }

  commonDir(path: string): Promise<string> {
    this.calls.push('commonDir')
    const entry = this.repositories[path]
    if (entry === undefined) return Promise.reject(new Error(`${path}: not a git repository`))
    if ('error' in entry) return Promise.reject(new Error(entry.error))
    if (entry.commonDirError !== undefined) return Promise.reject(new Error(entry.commonDirError))
    return Promise.resolve(entry.commonDir ?? join(entry.root, '.git'))
  }

  remotes(path: string): Promise<readonly GitRemote[]> {
    this.calls.push('remotes')
    const entry = this.repositories[path]
    if (entry === undefined) return Promise.resolve([])
    if ('error' in entry) return Promise.reject(new Error(entry.error))
    return Promise.resolve(entry.remotes)
  }
}

/** The real file system, which a test points at its own temporary directories. */
export class FakeFiles implements ModelGatewayFilesPort {
  async readText(path: string): Promise<string> {
    return readFile(path, 'utf8')
  }

  async exists(path: string): Promise<boolean> {
    try {
      await readFile(path)
      return true
    } catch {
      return false
    }
  }

  async writeText(path: string, content: string): Promise<void> {
    await mkdir(dirname(path), { recursive: true })
    await writeFile(path, content)
  }

  async mkdir(path: string): Promise<void> {
    await mkdir(path, { recursive: true })
  }

  async tempDir(prefix: string): Promise<string> {
    return mkdtemp(join(tmpdir(), prefix))
  }

  async remove(path: string): Promise<void> {
    await rm(path, { force: true, recursive: true })
  }
}

/** A ledger that keeps every record in memory, so a test can read what a run appended. */
export class FakeLedger implements ModelGatewayLedgerPort {
  readonly records: LedgerEntry[] = []

  append(record: LedgerEntry): Promise<void> {
    this.records.push(record)
    return Promise.resolve()
  }

  read(): Promise<readonly LedgerEntry[]> {
    return Promise.resolve([...this.records])
  }

  update<T>(change: (entries: readonly LedgerEntry[]) => LedgerChange<T>): Promise<T> {
    const changed = change([...this.records])
    this.records.push(...changed.append)
    return Promise.resolve(changed.result)
  }
}

/** The kept-worktrees list, held in memory, for a test that does not run a writing delegation. */
export class FakeKept implements ModelGatewayKeptPort {
  readonly worktrees: KeptWorktree[] = []
  /** What `add` rejects with, as a state directory that cannot be written does; none by default. */
  addError: Error | undefined

  add(worktree: KeptWorktree): Promise<void> {
    if (this.addError !== undefined) return Promise.reject(this.addError)
    this.worktrees.push(worktree)
    return Promise.resolve()
  }

  list(): Promise<readonly KeptWorktree[]> {
    return Promise.resolve([...this.worktrees])
  }

  remove(runIds: readonly string[]): Promise<void> {
    for (const runId of runIds) {
      const at = this.worktrees.findIndex((worktree) => worktree.run_id === runId)
      if (at !== -1) this.worktrees.splice(at, 1)
    }
    return Promise.resolve()
  }
}

/** The proof files a test names; the ones it leaves out are missing, as they are on a machine without setup. */
export class FakeProof implements ModelGatewayProofPort {
  constructor(private files: Partial<ProofFiles> = {}) {}

  read(): Promise<ProofFiles> {
    return Promise.resolve({ routing: undefined, routingOverride: undefined, setupResults: undefined, ...this.files })
  }
}

/** A workflows file that keeps its records in memory, so a test can read what a command wrote. */
export class FakeWorkflows implements ModelGatewayWorkflowsPort {
  /** What `update` rejects with, as a workflow another process ended meanwhile does; none by default. */
  updateError: Error | undefined

  constructor(private records: readonly WorkflowRecord[] = []) {}

  read(): Promise<readonly WorkflowRecord[]> {
    return Promise.resolve(this.records)
  }

  update(change: (workflows: readonly WorkflowRecord[]) => WorkflowChange): Promise<WorkflowRecord> {
    if (this.updateError !== undefined) return Promise.reject(this.updateError)
    const changed = change(this.records)
    this.records = changed.workflows
    return Promise.resolve(changed.workflow)
  }
}

/** A grants file that keeps its records in memory, so a test can read what a command wrote. */
export class FakeGrants implements ModelGatewayGrantsPort {
  constructor(private records: readonly Grant[] = []) {}

  read(): Promise<readonly Grant[]> {
    return Promise.resolve(this.records)
  }

  update(change: (grants: readonly Grant[]) => GrantChange): Promise<Grant> {
    const changed = change(this.records)
    this.records = changed.grants
    return Promise.resolve(changed.grant)
  }
}

/** A clock that stands still unless a test moves it, so a run's id and duration are predictable. */
export class FixedClock implements ClockPort {
  constructor(
    private at: Date = new Date('2026-11-04T10:52:15.000Z'),
    private elapsedMs = 0,
  ) {}

  now(): Date {
    return this.at
  }

  monotonicMs(): number {
    return this.elapsedMs
  }

  sleep(): Promise<void> {
    return Promise.resolve()
  }

  /** Moves the clock on, as a run that took `ms` would. */
  advance(ms: number): void {
    this.elapsedMs += ms
    this.at = new Date(this.at.getTime() + ms)
  }
}

/** What `agy --output-format json` prints: one object, with the response and its usage. */
export function agyResult(response: string, extra: Readonly<Record<string, unknown>> = {}): string {
  return JSON.stringify({
    conversation_id: 'c-1',
    duration_seconds: 2.5,
    num_turns: 1,
    response,
    status: 'SUCCESS',
    usage: { cache_read_tokens: 100, input_tokens: 7661, output_tokens: 29, thinking_tokens: 28, total_tokens: 7690 },
    ...extra,
  })
}

/** What `agy --output-format stream-json` prints: an init event, then the result event. */
export function agyStream(result: string): string {
  return [JSON.stringify({ conversation_id: 'c-1', event: 'init' }), JSON.stringify({ event: 'result', result: JSON.parse(result) as unknown })].join('\n')
}

/** What one fake client launch answers: its stdout, its exit, and the export Vibe writes where it was told to. */
export interface VibeScript {
  /** What the client prints on stdout: Vibe's journal, agy's or Claude Code's JSON object, OpenCode's event lines. */
  readonly stdout: string
  readonly stderr?: string
  readonly exitCode?: number
  readonly timedOut?: boolean
  /** What Vibe writes to `export.json` in its output directory; nothing when it writes none. */
  readonly exportJson?: unknown
  /** Runs before the launch returns, as a client that writes into its workdir would. */
  readonly effect?: (command: ProcessCommand) => Promise<void>
}

/**
 * What a client's own status command answers in the fakes that play launches:
 * signed in, so a delegation's check that a client can start never takes a
 * launch's scripted answer. Undefined for any command that is not one.
 */
function statusAnswer(command: ProcessCommand): ProcessResult | undefined {
  const [first, second] = command.args
  const credentials = ['hermes', 'opencode'].includes(command.command) && first === 'auth' && second === 'list'
  if (credentials) return { exitCode: 0, stderr: '', stdout: 'signed in\n' }
  const codex = command.command === 'codex' && first === 'login' && second === 'status'
  return codex ? { exitCode: 0, stderr: '', stdout: 'Logged in using ChatGPT\n' } : undefined
}

/** What one fake Hermes launch answers: its one-shot text, its exit, and the usage report it writes. */
export interface HermesScript {
  /** What `hermes -z` prints: the final response text alone. */
  readonly stdout: string
  readonly stderr?: string
  readonly exitCode?: number
  readonly timedOut?: boolean
  /** What Hermes writes to the `--usage-file` it was given; nothing when it writes none. */
  readonly usage?: unknown
  /** Runs before the launch returns, as a client that writes into its workdir would. */
  readonly effect?: (command: ProcessCommand) => Promise<void>
}

/**
 * A Hermes that plays the scripts a test queues, one per launch, and writes
 * each launch's usage report where `--usage-file` points. It starts nothing and
 * spends nothing.
 */
export class FakeHermesProcess implements ModelGatewayProcessPort {
  readonly commands: ProcessCommand[] = []

  constructor(private readonly scripts: HermesScript[] = []) {}

  /** Adds launches to play, for a test that needs the run's paths before it writes them. */
  queue(...scripts: HermesScript[]): void {
    this.scripts.push(...scripts)
  }

  which(): Promise<boolean> {
    return Promise.resolve(true)
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    const status = statusAnswer(command)
    if (status !== undefined) return status
    this.commands.push(command)
    const script = this.scripts.shift()
    if (script === undefined) throw new Error('unexpected process launch')
    const usagePath = command.args[command.args.indexOf('--usage-file') + 1]
    if (script.usage !== undefined && usagePath !== undefined) {
      await mkdir(dirname(usagePath), { recursive: true })
      await writeFile(usagePath, JSON.stringify(script.usage))
    }
    await script.effect?.(command)
    if (script.timedOut === true) return { exitCode: 124, stderr: script.stderr ?? '', stdout: script.stdout, timedOut: true }
    return { exitCode: script.exitCode ?? 0, stderr: script.stderr ?? '', stdout: script.stdout }
  }
}

/**
 * A client that plays the scripts a test queues, one per launch, and writes
 * Vibe's export where `--output-dir` points (the other clients have none). It
 * starts nothing and spends nothing. It stands in for whichever client a test
 * runs: Vibe, Antigravity, Claude Code or OpenCode.
 */
export class FakeVibeProcess implements ModelGatewayProcessPort {
  readonly commands: ProcessCommand[] = []

  constructor(private readonly scripts: VibeScript[] = []) {}

  /** Adds launches to play, for a test that needs the run's paths before it writes them. */
  queue(...scripts: VibeScript[]): void {
    this.scripts.push(...scripts)
  }

  which(): Promise<boolean> {
    return Promise.resolve(true)
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    const status = statusAnswer(command)
    if (status !== undefined) return status
    this.commands.push(command)
    const script = this.scripts.shift()
    if (script === undefined) throw new Error('unexpected process launch')
    const outputDir = command.args[command.args.indexOf('--output-dir') + 1]
    if (script.exportJson !== undefined && outputDir !== undefined) {
      await mkdir(outputDir, { recursive: true })
      await writeFile(join(outputDir, 'export.json'), JSON.stringify(script.exportJson))
    }
    await script.effect?.(command)
    if (script.timedOut === true) return { exitCode: 124, stderr: script.stderr ?? '', stdout: script.stdout, timedOut: true }
    return { exitCode: script.exitCode ?? 0, stderr: script.stderr ?? '', stdout: script.stdout }
  }
}
