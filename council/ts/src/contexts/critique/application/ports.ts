import type { ClockPort, EnvPort, GhPort, GitIndexPort, GitPort, GitRevisionPort } from '../../../ports/index.js'
import type { CriticFailure, CriticOutcome } from '../domain/critic.js'
import type { AgyCriticClient, ClientReadiness, CriticClient, RosterCritic, VibeCriticClient } from '../domain/routing.js'

/**
 * The repository facts the critique needs: council's git port, narrowed to
 * what it reads, the commits a code stage freezes, and the index, which
 * holds a change before it is committed.
 */
export type CritiqueGitPort = Pick<GitPort, 'root' | 'commonDir' | 'remotes' | 'porcelainStatus' | 'currentBranch'> & GitRevisionPort & GitIndexPort

/** What the critique reads from GitHub: council's gh port, narrowed to reading issues and their links. */
export type CritiqueGhPort = Pick<GhPort, 'viewIssue' | 'subIssues' | 'blockedBy'>

/** Plain file access; every path is absolute. */
export interface CritiqueFilesPort {
  readText(path: string): Promise<string>
  readBytes(path: string): Promise<Uint8Array>
  /** Writes the file, creating parent directories. */
  writeText(path: string, content: string): Promise<void>
  /** Appends to the file, creating it and its parent directories. */
  appendText(path: string, content: string): Promise<void>
  exists(path: string): Promise<boolean>
  /** The names of the plain files directly inside a directory, sorted; empty when it does not exist. */
  listFiles(path: string): Promise<readonly string[]>
  /** Every file under the directory, at any depth, relative to it; none when it does not exist. */
  listTree(dir: string): Promise<readonly string[]>
  /** The names of the directories directly inside a directory, sorted; empty when it does not exist. */
  listDirectories(path: string): Promise<readonly string[]>
  remove(path: string): Promise<void>
}

/** Where one critic of a run works. */
export interface CriticLaunch {
  readonly critic: RosterCritic
  /** The critic's own copy of the packet: its working directory, and all it may read. */
  readonly packetDir: string
  /** The assembled prompt, written outside the packet. */
  readonly promptPath: string
  /** The critic's own directory in the run, for whatever its client writes there. */
  readonly criticDir: string
  /** The finding schema, for a client that can enforce it. */
  readonly schemaPath: string
}

/** A critic's run as its client saw it: the raw output, and what that output amounts to. */
export interface CriticReport {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
  /** The file the client read cost and effective config from. */
  readonly exportPath: string
  readonly outcome: CriticOutcome
  /**
   * What the run counts against the flow's Vibe cap: the client's own best
   * estimate of its spend, never less than it reported.
   */
  readonly chargedUsd: number
}

/** One critic client: launches a critic in its packet and reads its review. */
export interface CriticClientPort<Config extends CriticClient = CriticClient> {
  /** Whether the client can run on this machine now. Prepare asks before it puts the client on the roster. */
  readiness(config: Config): Promise<ClientReadiness>
  /** Runs one critic; `config` is the launch's client config, narrowed to this client's kind. */
  review(launch: CriticLaunch, config: Config): Promise<CriticReport>
}

/**
 * One question put to a model in a working directory of its own: a yes/no
 * question such as triage asks, never a review, so its answer is plain text
 * rather than findings.
 */
export interface ModelQuestion {
  /** The model's working directory, and all it may read. */
  readonly workDir: string
  /** The question, written outside the working directory. */
  readonly promptPath: string
  /** Where the client writes what it keeps of the run. */
  readonly outputDir: string
  readonly max_price_usd: number
  readonly time_limit_s: number
}

/** The model's final message, or why there is none. */
export type ModelAnswer =
  | { readonly status: 'answered'; readonly text: string; readonly cost_usd: number | null }
  | { readonly status: 'failed'; readonly failure: CriticFailure; readonly detail: string; readonly cost_usd: number | null }

export interface ModelAnswerReport {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
  readonly answer: ModelAnswer
  /** What the run counts against the flow's Vibe cap, as for a critic. */
  readonly chargedUsd: number
}

/** A client that can also put one question to its model. */
export interface QuestionClientPort<Config extends CriticClient> {
  ask(question: ModelQuestion, config: Config): Promise<ModelAnswerReport>
}

/** The client each kind of critic runs through. Vibe also answers triage's question. */
export interface CriticClients {
  readonly agy: CriticClientPort<AgyCriticClient>
  readonly vibe: CriticClientPort<VibeCriticClient> & QuestionClientPort<VibeCriticClient>
}

/** Asks the client of `config`'s kind whether it can run. */
export function clientReadiness(clients: CriticClients, config: CriticClient): Promise<ClientReadiness> {
  return config.kind === 'agy' ? clients.agy.readiness(config) : clients.vibe.readiness(config)
}

/** Runs the launch's critic through the client of its kind. */
export function reviewWith(clients: CriticClients, launch: CriticLaunch): Promise<CriticReport> {
  const config = launch.critic.client_config
  return config.kind === 'agy' ? clients.agy.review(launch, config) : clients.vibe.review(launch, config)
}

/** One runner command: it takes the JSON the skill sends and returns a JSON result. */
export interface CritiqueCommand<Result> {
  run(input: unknown): Promise<Result>
}

export interface CritiqueDeps {
  /** Directory holding the shipped prompts, lenses, schema and routing. */
  readonly assetsRoot: string
  readonly clock: ClockPort
  readonly env: EnvPort
  readonly files: CritiqueFilesPort
  readonly git: CritiqueGitPort
  /** Reads spec and ticket issues when the repository's tracker is GitHub. */
  readonly gh: CritiqueGhPort
  /** The client each kind of critic runs through. */
  readonly clients: CriticClients
}
