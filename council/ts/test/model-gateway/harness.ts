import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

import type {
  LedgerRecord,
  ModelGatewayFilesPort,
  ModelGatewayGitPort,
  ModelGatewayLedgerPort,
  ModelGatewayProcessPort,
  ModelGatewayProofPort,
  ProofFiles,
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
  readonly action: 'add' | 'remove'
  readonly repo: string
  readonly path: string
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

  constructor(private readonly repositories: Readonly<Record<string, FakeRepository>>) {}

  async addWorktree(repo: string, path: string): Promise<void> {
    this.worktrees.push({ action: 'add', path, repo })
    const entry = this.repositories[repo]
    if (entry !== undefined && 'root' in entry && entry.worktreeError !== undefined) throw new Error(entry.worktreeError)
    await writeFile(join(path, FAKE_CHECKOUT_FILE), 'HEAD')
  }

  async removeWorktree(repo: string, path: string): Promise<void> {
    this.worktrees.push({ action: 'remove', path, repo })
    const entry = this.repositories[repo]
    if (entry !== undefined && 'root' in entry && entry.removeError !== undefined) throw new Error(entry.removeError)
    await rm(path, { force: true, recursive: true })
  }

  root(path: string): Promise<string | undefined> {
    const entry = this.repositories[path]
    if (entry === undefined) return Promise.resolve(undefined)
    if ('error' in entry) return Promise.reject(new Error(entry.error))
    return Promise.resolve(entry.root)
  }

  commonDir(path: string): Promise<string> {
    const entry = this.repositories[path]
    if (entry === undefined) return Promise.reject(new Error(`${path}: not a git repository`))
    if ('error' in entry) return Promise.reject(new Error(entry.error))
    if (entry.commonDirError !== undefined) return Promise.reject(new Error(entry.commonDirError))
    return Promise.resolve(entry.commonDir ?? join(entry.root, '.git'))
  }

  remotes(path: string): Promise<readonly GitRemote[]> {
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
  readonly records: LedgerRecord[] = []

  append(record: LedgerRecord): Promise<void> {
    this.records.push(record)
    return Promise.resolve()
  }

  read(): Promise<readonly LedgerRecord[]> {
    return Promise.resolve([...this.records])
  }
}

/** The proof files a test names; the ones it leaves out are missing, as they are on a machine without setup. */
export class FakeProof implements ModelGatewayProofPort {
  constructor(private files: Partial<ProofFiles> = {}) {}

  read(): Promise<ProofFiles> {
    return Promise.resolve({ routing: undefined, routingOverride: undefined, setupResults: undefined, ...this.files })
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
