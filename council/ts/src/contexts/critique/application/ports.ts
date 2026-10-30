import type { ClockPort, EnvPort, GitPort } from '../../../ports/index.js'
import type { CriticOutcome } from '../domain/critic.js'
import type { CriticClient, RosterCritic } from '../domain/routing.js'

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

/** Where one critic of a run works. */
export interface CriticLaunch {
  readonly critic: RosterCritic
  /** The packet: the critic's working directory, and all it may read. */
  readonly packetDir: string
  /** The assembled prompt, written outside the packet. */
  readonly promptPath: string
  /** The critic's own directory in the run, for whatever its client writes there. */
  readonly criticDir: string
}

/** A critic's run as its client saw it: the raw output, and what that output amounts to. */
export interface CriticReport {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
  /** The file the client read cost and effective config from. */
  readonly exportPath: string
  readonly outcome: CriticOutcome
}

/** One critic client: launches a critic in its packet and reads its review. */
export interface CriticClientPort {
  review(launch: CriticLaunch): Promise<CriticReport>
}

export interface CritiqueDeps {
  /** Directory holding the shipped prompts, lenses, schema and routing. */
  readonly assetsRoot: string
  readonly clock: ClockPort
  readonly env: EnvPort
  readonly files: CritiqueFilesPort
  readonly git: CritiqueGitPort
  /** The client each kind of critic runs through. */
  readonly clients: Readonly<Record<CriticClient['kind'], CriticClientPort>>
}
