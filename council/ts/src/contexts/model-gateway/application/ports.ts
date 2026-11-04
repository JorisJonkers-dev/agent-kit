import type { EnvPort } from '../../../ports/index.js'

/** The files the gateway reads: the shipped defaults and this machine's override. */
export interface ModelGatewayFilesPort {
  /** Rejects with the platform's error (its `code` is `ENOENT` for a missing file). */
  readText(path: string): Promise<string>
  exists(path: string): Promise<boolean>
}

/** Whether a client's program is on PATH. It never runs the client itself. */
export interface ModelGatewayProcessPort {
  which(binary: string): Promise<boolean>
}

export interface ModelGatewayDependencies {
  readonly files: ModelGatewayFilesPort
  readonly process: ModelGatewayProcessPort
  /** The environment the override's directory comes from: XDG_CONFIG_HOME, else HOME. */
  readonly env: EnvPort
  /** The shipped defaults file, beside the running bundle. */
  readonly defaultsPath: string
}

/** One gateway command: its input in, its result out, or a ModelGatewayError that says what went wrong. */
export interface ModelGatewayCommand<Input, Result> {
  run(input: Input): Promise<Result>
}
