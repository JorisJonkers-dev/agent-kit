import type { GitRemote } from '../../../shared-kernel/index.js'
import type { EnvPort } from '../../../ports/index.js'

/** The files the gateway reads: the shipped defaults, this machine's override and the family allowlist. */
export interface ModelGatewayFilesPort {
  /** Rejects with the platform's error (its `code` is `ENOENT` for a missing file). */
  readText(path: string): Promise<string>
  exists(path: string): Promise<boolean>
}

/** Whether a client's program is on PATH. It never runs the client itself. */
export interface ModelGatewayProcessPort {
  which(binary: string): Promise<boolean>
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
  /** The environment the override's and the allowlist's directories come from: XDG_CONFIG_HOME, else HOME. */
  readonly env: EnvPort
  /** The shipped defaults file, beside the running bundle. */
  readonly defaultsPath: string
}

/** One gateway command: its input in, its result out, or a ModelGatewayError that says what went wrong. */
export interface ModelGatewayCommand<Input, Result> {
  run(input: Input): Promise<Result>
}
