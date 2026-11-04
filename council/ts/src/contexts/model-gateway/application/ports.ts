import type { ClockPort, EnvPort, ProcessCommand, ProcessResult } from '../../../ports/index.js'
import type { GitRemote } from '../../../shared-kernel/index.js'
import type { LedgerRecord } from '../domain/ledger.js'
import type { ClientListing } from '../domain/models.js'
import type { ProofFiles } from '../domain/run.js'
import type { ClientStatusReading } from '../domain/usage.js'

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
  /** Removes a worktree added with `addWorktree`, whatever it holds now; its directory is gone afterwards. */
  removeWorktree(repo: string, path: string): Promise<void>
}

/**
 * The gateway's ledger: one line per run, machine-wide. Appending takes an
 * exclusive lock beside the file, so two gateway processes never interleave a
 * line.
 */
export interface ModelGatewayLedgerPort {
  append(record: LedgerRecord): Promise<void>
  /** Every record the ledger holds, oldest first; a line that does not parse is skipped. */
  read(): Promise<readonly LedgerRecord[]>
}

/**
 * A client's own status command, run read-only: it says whether the client is
 * signed in, and never sends a prompt. A client without one has no entry.
 */
export interface ClientStatusPort {
  read(): Promise<ClientStatusReading>
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
  /** The clients whose own status command says whether they are signed in, by client id. */
  readonly statuses: Readonly<Record<string, ClientStatusPort>>
  /** The machine-wide ledger every run appends to, and usage reads. */
  readonly ledger: ModelGatewayLedgerPort
  /** The read-only proofs a run in place needs. */
  readonly proof: ModelGatewayProofPort
  /** The clock a run's id, its duration and its ledger line are taken from. */
  readonly clock: ClockPort
}

/** One gateway command: its input in, its result out, or a ModelGatewayError that says what went wrong. */
export interface ModelGatewayCommand<Input, Result> {
  run(input: Input): Promise<Result>
}
