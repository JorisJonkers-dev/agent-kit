import type { ClockPort, EnvPort, GitPort, ProcessPort } from '../../../ports/index.js'

/** The repository facts the critique needs: council's git port, narrowed to what it reads. */
export type CritiqueGitPort = Pick<GitPort, 'root' | 'commonDir' | 'remotes'>

/** Plain file access; every path is absolute. */
export interface CritiqueFilesPort {
  readText(path: string): Promise<string>
  /** Writes the file, creating parent directories. */
  writeText(path: string, content: string): Promise<void>
  /** Appends to the file, creating it and its parent directories. */
  appendText(path: string, content: string): Promise<void>
  exists(path: string): Promise<boolean>
}

export interface CritiqueDeps {
  /** Directory holding the shipped prompts, lenses, schema and routing. */
  readonly assetsRoot: string
  readonly clock: ClockPort
  readonly env: EnvPort
  readonly files: CritiqueFilesPort
  readonly git: CritiqueGitPort
  readonly process: ProcessPort
}
