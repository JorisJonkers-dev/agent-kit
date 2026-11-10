import type { ClockPort, EnvPort, GhPort, GitHistoryPort, GitIndexPort, GitPort, GitRevisionPort } from '../../../ports/index.js'
import type { ModelGatewayLedgerPort, PlanLimitReading } from '../../model-gateway/index.js'
import type { CriticFailure, CriticOutcome, CriticTokens } from '../domain/critic.js'
import type { CriticToolset, SearchServerLaunch, ToolCall } from '../domain/critic-tools.js'
import type { SearchCall, SearchLimits, SearchToolName } from '../domain/packet-search.js'
import type { AgyCriticClient, ClaudeCriticClient, ClientReadiness, CodexCriticClient, CriticClient, RosterCritic, VibeCriticClient } from '../domain/routing.js'

/**
 * The repository facts the critique needs: council's git port, narrowed to
 * what it reads, the commits a code stage freezes, and the index, which
 * holds a change before it is committed.
 */
export type CritiqueGitPort = Pick<GitPort, 'root' | 'commonDir' | 'remotes' | 'porcelainStatus' | 'currentBranch'> & GitRevisionPort & GitIndexPort & GitHistoryPort

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
  /** Creates a symlink at `path` pointing to `target`, creating parent directories; any file already there is replaced. */
  symlink(target: string, path: string): Promise<void>
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
  /** The tools the critic has, which its prompt names; a Vibe critic without it reads alone. */
  readonly toolset?: CriticToolset
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
   * What the run counts against the flow's Vibe cap: the client's best
   * estimate of its spend. For Vibe that is the cost the runner computed from
   * the tokens (or Vibe's own figure without prices), which may be less than
   * Vibe reported; the critic's price cap where neither can be trusted.
   */
  readonly chargedUsd: number
  /**
   * What Vibe itself reported, kept beside the cost the runner computed from
   * the tokens; null when it reported nothing. Absent for any other client.
   */
  readonly vibeCostUsd?: number | null
  /** What the run used, as its client reported it; absent when the client reports none. */
  readonly tokens?: CriticTokens
  /** Why the critic's searches failed during the run, such as a search server that died: the report names it. */
  readonly searchFailure?: string
  /** Every tool call the critic made, in order, when its client's journal records them. */
  readonly toolCalls?: readonly ToolCall[]
}

/** One critic client: launches a critic in its packet and reads its review. */
export interface CriticClientPort<Config extends CriticClient = CriticClient> {
  /** Whether the client can run on this machine now. Prepare asks before it puts the client on the roster. */
  readiness(config: Config): Promise<ClientReadiness>
  /**
   * Whether the CLI is installed and signed in, leaving out what one critic
   * needs beyond that (a Vibe critic's agent file). Launches no model.
   */
  signIn(config: Config): Promise<ClientReadiness>
  /** The first line `<binary> --version` prints, or null when it cannot be read. Launches no model. */
  version(config: Config): Promise<string | null>
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
  /** What Vibe itself reported, as for a critic. */
  readonly vibeCostUsd: number | null
}

/** A client that can also put one question to its model. */
export interface QuestionClientPort<Config extends CriticClient> {
  ask(question: ModelQuestion, config: Config): Promise<ModelAnswerReport>
}

/** A client whose critics can search through the runner's search server. */
export interface SearchingClientPort<Config extends CriticClient> {
  /**
   * Why a critic of this config cannot search, read before it is launched
   * and without launching a model: undefined when nothing on this machine
   * stops it.
   */
  searchBlocked(config: Config): Promise<string | undefined>
}

/**
 * The client each kind of critic runs through. Vibe also answers triage's
 * question, and its critics search through the runner's server. Codex and the
 * work Claude agent serve only a `nedap` repository's work roster; a machine
 * that wires neither has no work critic, and prepare drops it as unavailable.
 */
export interface CriticClients {
  readonly agy: CriticClientPort<AgyCriticClient>
  readonly vibe: CriticClientPort<VibeCriticClient> & QuestionClientPort<VibeCriticClient> & SearchingClientPort<VibeCriticClient>
  readonly codex?: CriticClientPort<CodexCriticClient>
  readonly claude?: CriticClientPort<ClaudeCriticClient>
}

/** Why a work critic cannot run on a machine that wires no client for its kind. */
function unwired(kind: CriticClient['kind']): ClientReadiness {
  return { detail: `no ${kind} critic client is wired on this machine`, ready: false }
}

/** Asks the client of `config`'s kind whether it can run. */
export function clientReadiness(clients: CriticClients, config: CriticClient): Promise<ClientReadiness> {
  switch (config.kind) {
    case 'agy':
      return clients.agy.readiness(config)
    case 'vibe':
      return clients.vibe.readiness(config)
    case 'codex':
      return clients.codex?.readiness(config) ?? Promise.resolve(unwired('codex'))
    case 'claude':
      return clients.claude?.readiness(config) ?? Promise.resolve(unwired('claude'))
  }
}

/** Asks the client of `config`'s kind whether its CLI is installed and signed in. */
export function clientSignIn(clients: CriticClients, config: CriticClient): Promise<ClientReadiness> {
  switch (config.kind) {
    case 'agy':
      return clients.agy.signIn(config)
    case 'vibe':
      return clients.vibe.signIn(config)
    case 'codex':
      return clients.codex?.signIn(config) ?? Promise.resolve(unwired('codex'))
    case 'claude':
      return clients.claude?.signIn(config) ?? Promise.resolve(unwired('claude'))
  }
}

/** The version of the CLI `config` runs through. */
export function clientVersion(clients: CriticClients, config: CriticClient): Promise<string | null> {
  switch (config.kind) {
    case 'agy':
      return clients.agy.version(config)
    case 'vibe':
      return clients.vibe.version(config)
    case 'codex':
      return clients.codex?.version(config) ?? Promise.resolve(null)
    case 'claude':
      return clients.claude?.version(config) ?? Promise.resolve(null)
  }
}

/** Runs the launch's critic through the client of its kind. */
export function reviewWith(clients: CriticClients, launch: CriticLaunch): Promise<CriticReport> {
  const config = launch.critic.client_config
  switch (config.kind) {
    case 'agy':
      return clients.agy.review(launch, config)
    case 'vibe':
      return clients.vibe.review(launch, config)
    case 'codex':
      return reviewThrough(clients.codex, launch, config)
    case 'claude':
      return reviewThrough(clients.claude, launch, config)
  }
}

function reviewThrough<Config extends CriticClient>(
  client: CriticClientPort<Config> | undefined,
  launch: CriticLaunch,
  config: Config,
): Promise<CriticReport> {
  if (client === undefined) return Promise.reject(new Error(unwiredDetail(config.kind)))
  return client.review(launch, config)
}

function unwiredDetail(kind: CriticClient['kind']): string {
  return `no ${kind} critic client is wired on this machine`
}

/** One runner command: it takes the JSON the skill sends and returns a JSON result. */
export interface CritiqueCommand<Result> {
  run(input: unknown): Promise<Result>
}

/** How the runner starts itself as a search server: the command and the arguments before `search-server`. */
export interface SearchServerCommand {
  readonly command: string
  readonly args: readonly string[]
}

export type { SearchServerLaunch }

/** One session with a search server: the tools it offered and an answer per call, or why it did not start. */
export type SearchSession =
  | { readonly status: 'started'; readonly tools: readonly SearchToolName[]; readonly answers: readonly string[] }
  | { readonly status: 'failed'; readonly detail: string }

/** The runner's own search server (ADR-0006), as the runner starts and stops it. */
export interface SearchClientPort {
  /** The command line a critic's client starts the server with, confined to `root`. */
  server(root: string, limits: SearchLimits): SearchServerLaunch
  /** Starts the server on `root` as an MCP client would, asks it the calls, and closes it. */
  session(root: string, limits: SearchLimits, calls: readonly SearchCall[]): Promise<SearchSession>
  /** Stops every search server still running on `root`. */
  stop(root: string): Promise<void>
}

/**
 * A client's plan limit as the model gateway reads it: each window of the
 * plan with what the machine-wide ledger says is spent in it, the gateway's
 * warning for every window at or above the warning ratio, and why the client
 * cannot start at all. Critique asks before it plans a Vibe critic, so it
 * counts Vibe the way the gateway does.
 */
export interface PlanLimitPort {
  read(client: string): Promise<PlanLimitReading>
}

/**
 * The model gateway's grants as critique reads them: only whether a live
 * grant covers a workflow for a client, which is what lets Codex critique a
 * `nedap` repository (ADR-0008). Critique never writes or renews one.
 */
export interface GrantsPort {
  covers(workflow: string, client: string): Promise<boolean>
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
  /** The runner's search server for Vibe critics; without it every Vibe critic reads alone. */
  readonly search?: SearchClientPort
  /** The gateway's ledger, where every critic run is recorded beside the gateway's own runs. */
  readonly ledger: ModelGatewayLedgerPort
  /** The gateway's grants, which Codex critics of a `nedap` repository need. */
  readonly grants: GrantsPort
  /** The gateway's plan-limit reading, which replaces the per-flow Vibe cap. */
  readonly plan: PlanLimitPort
}
