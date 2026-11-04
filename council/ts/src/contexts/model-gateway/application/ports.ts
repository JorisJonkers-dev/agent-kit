import type { ClockPort, EnvPort, ProcessCommand, ProcessResult } from '../../../ports/index.js'
import type { GitRemote } from '../../../shared-kernel/index.js'
import type { LedgerRecord } from '../domain/ledger.js'
import type { ClientListing } from '../domain/models.js'
import type { ProofFiles } from '../domain/run.js'
import type { WorkflowChange, WorkflowRecord } from '../domain/workflows.js'

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

/** The repository a path sits in. It runs git, never a client. */
export interface ModelGatewayGitPort {
  /** The working tree's top level, or undefined when the path is in no repository. */
  root(path: string): Promise<string | undefined>
  /** The repository's common directory, shared by every worktree of it. */
  commonDir(path: string): Promise<string>
  /** Every remote with its URL; empty when the repository has none. */
  remotes(path: string): Promise<readonly GitRemote[]>
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
 * The workflows this machine has started: one file, read and written under a
 * lock, so two gateway processes never lose each other's start or end.
 */
export interface ModelGatewayWorkflowsPort {
  read(): Promise<readonly WorkflowRecord[]>
  /**
   * Reads the workflows, applies `change` to them, writes what it returns, and
   * hands back the workflow the change touched — all under the file's lock, so
   * two gateway processes never lose each other's start or end.
   */
  update(change: (workflows: readonly WorkflowRecord[]) => WorkflowChange): Promise<WorkflowRecord>
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
  /** The workflows every run names, and the moments they started and ended. */
  readonly workflows: ModelGatewayWorkflowsPort
  /** The clock a run's id, its duration and its ledger line are taken from. */
  readonly clock: ClockPort
}

/** One gateway command: its input in, its result out, or a ModelGatewayError that says what went wrong. */
export interface ModelGatewayCommand<Input, Result> {
  run(input: Input): Promise<Result>
}
