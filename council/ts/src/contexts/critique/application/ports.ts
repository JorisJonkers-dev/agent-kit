import type { ClockPort, EnvPort, ProcessPort } from '../../../ports/index.js'
import type { GitRemote } from '../domain/family-allowlist.js'

/** The repository facts the critique needs from git. */
export interface CritiqueGitPort {
  /** Absolute path of the working tree's top level. */
  root(cwd: string): Promise<string>
  /** Absolute path of the git common directory, shared by every worktree. */
  commonDir(cwd: string): Promise<string>
  remotes(cwd: string): Promise<readonly GitRemote[]>
}

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
