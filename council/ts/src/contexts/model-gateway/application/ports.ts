import type { EnvPort, ProcessCommand, ProcessResult } from '../../../ports/index.js'
import type { GitRemote } from '../../../shared-kernel/index.js'
import type { ClientListing } from '../domain/models.js'

/** The files the gateway reads: the shipped defaults, this machine's override, a client's configuration and the family allowlist. */
export interface ModelGatewayFilesPort {
  /** Rejects with the platform's error (its `code` is `ENOENT` for a missing file). */
  readText(path: string): Promise<string>
  exists(path: string): Promise<boolean>
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
  /** Every remote with its URL; empty when the repository has none. */
  remotes(path: string): Promise<readonly GitRemote[]>
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
}

/** One gateway command: its input in, its result out, or a ModelGatewayError that says what went wrong. */
export interface ModelGatewayCommand<Input, Result> {
  run(input: Input): Promise<Result>
}
