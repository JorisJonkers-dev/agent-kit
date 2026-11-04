import type { EnvPort, ProcessCommand, ProcessResult } from '../../../ports/index.js'
import type { ClientListing } from '../domain/models.js'

/** The files the gateway reads: the shipped defaults, this machine's override and a client's configuration. */
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

export interface ModelGatewayDependencies {
  readonly files: ModelGatewayFilesPort
  readonly process: ModelGatewayProcessPort
  /** The environment the override's directory and Vibe's home come from. */
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
