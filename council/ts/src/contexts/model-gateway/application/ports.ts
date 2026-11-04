import type { ClockPort, EnvPort, ProcessCommand, ProcessResult } from '../../../ports/index.js'
import type { GitRemote } from '../../../shared-kernel/index.js'
import type { LedgerRecord } from '../domain/ledger.js'
import type { ClientListing } from '../domain/models.js'
import type { ProofFiles } from '../domain/run.js'

/**
 * The files the gateway reads and writes: the shipped defaults, this machine's
 * override, a client's configuration, the family allowlist, and a run's own
 * directory.
 */
export interface ModelGatewayFilesPort {
  /** Rejects with the platform's error (its `code` is `ENOENT` for a missing file). */
  readText(path: string): Promise<string>
  exists(path: string): Promise<boolean>
  /** Writes the file, creating its parent directories. */
  writeText(path: string, content: string): Promise<void>
  /** Creates a directory and its parents; an existing one is left as it is. */
  mkdir(path: string): Promise<void>
  /** A fresh, empty directory under the system's temporary directory, named after `prefix`. */
  tempDir(prefix: string): Promise<string>
  /** Removes a file or a directory with everything in it; a path that is not there is left as it is. */
  remove(path: string): Promise<void>
}

/** Whether a client's program is on PATH, and the listing commands it answers. */
export interface ModelGatewayProcessPort {
  which(binary: string): Promise<boolean>
  exec(command: ProcessCommand): Promise<ProcessResult>
}

/** A client's own model listing: what it offers now, or why it could not be read. */
export interface ModelSourcePort {
  list(): Promise<ClientListing>
}

/** The repository a path sits in, and the throwaway worktrees of it. It runs git, never a client. */
export interface ModelGatewayGitPort {
  /** The working tree's top level, or undefined when the path is in no repository. */
  root(path: string): Promise<string | undefined>
  /** The repository's common directory, shared by every worktree of it. */
  commonDir(path: string): Promise<string>
  /** Every remote with its URL; empty when the repository has none. */
  remotes(path: string): Promise<readonly GitRemote[]>
  /** Checks out the repository's HEAD, detached, into `path`, which is an existing empty directory. Rejects with git's message. */
  addWorktree(repo: string, path: string): Promise<void>
  /**
   * Creates a worktree at `path` on a new branch `branch` that starts at `base`
   * (any ref git resolves), so the delegate's edits land on a branch of its
   * own. Rejects with git's message when the base is unknown or the branch exists.
   */
  createWorktree(repo: string, path: string, branch: string, base: string): Promise<void>
  /** Stages everything in the worktree, new files and deletions included (`git add -A`). It commits nothing. */
  stageAll(path: string): Promise<void>
  /** The files staged in the worktree, relative to it. */
  stagedFiles(path: string): Promise<readonly string[]>
  /** Removes a worktree added with `addWorktree` or `createWorktree`, whatever it holds now; its directory is gone afterwards. */
  removeWorktree(repo: string, path: string): Promise<void>
  /** Deletes a branch with whatever it holds (`git branch -D`); rejects with git's message when it cannot. */
  deleteBranch(repo: string, branch: string): Promise<void>
}

/** A writing run's worktree the gateway kept, so the workflow's end can remove it. */
export interface KeptWorktree {
  readonly run_id: string
  readonly workflow: string
  /** The repository's top level, which git commands about the worktree run in. */
  readonly repo: string
  readonly path: string
  readonly branch: string
  /** When the worktree was made, as an ISO timestamp. */
  readonly at: string
}

/**
 * The writing worktrees the gateway made and has not removed, machine-wide.
 * Every operation takes the same exclusive lock, so two gateway processes
 * never lose each other's entries.
 */
export interface ModelGatewayKeptPort {
  add(worktree: KeptWorktree): Promise<void>
  list(): Promise<readonly KeptWorktree[]>
  /** Forgets the worktrees of these runs. */
  remove(runIds: readonly string[]): Promise<void>
}

/**
 * The gateway's ledger: one line per run, machine-wide. Appending takes an
 * exclusive lock beside the file, so two gateway processes never interleave a
 * line.
 */
export interface ModelGatewayLedgerPort {
  append(record: LedgerRecord): Promise<void>
}

/**
 * What setup and critique left on this machine, as they wrote it: the
 * read-only proofs setup recorded, and the routing that says which critic
 * client runs which model. Any of them may be missing.
 */
export interface ModelGatewayProofPort {
  /** What setup and critique left on this machine; any of it may be missing. */
  read(): Promise<ProofFiles>
}

export interface ModelGatewayDependencies {
  readonly files: ModelGatewayFilesPort
  readonly process: ModelGatewayProcessPort
  readonly git: ModelGatewayGitPort
  /** The environment the override's and the allowlist's directories come from, and Vibe's home: XDG_CONFIG_HOME, else HOME. */
  readonly env: EnvPort
  /** The shipped defaults file, beside the running bundle. */
  readonly defaultsPath: string
  /** The clients whose models come from the client itself or its configuration, by client id. */
  readonly sources: Readonly<Record<string, ModelSourcePort>>
  /** The machine-wide ledger every run appends to. */
  readonly ledger: ModelGatewayLedgerPort
  /** The read-only proofs a run in place needs. */
  readonly proof: ModelGatewayProofPort
  /** The writing worktrees kept until their workflow ends. */
  readonly kept: ModelGatewayKeptPort
  /** The clock a run's id, its duration and its ledger line are taken from. */
  readonly clock: ClockPort
}

/** One gateway command: its input in, its result out, or a ModelGatewayError that says what went wrong. */
export interface ModelGatewayCommand<Input, Result> {
  run(input: Input): Promise<Result>
}
