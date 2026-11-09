import { isAbsolute, join } from 'node:path'

import { ClientRun, vibeSignedIn, type RunExtras } from '../../../model-gateway/index.js'
import type { EnvPort, ProcessPort } from '../../../../ports/index.js'
import type {
  CriticClientPort,
  CriticLaunch,
  CriticReport,
  CritiqueFilesPort,
  ModelAnswer,
  ModelAnswerReport,
  ModelQuestion,
  QuestionClientPort,
  SearchingClientPort,
} from '../../application/ports.js'
import type { CriticFailure, CriticOutcome, CriticTokens } from '../../domain/critic.js'
import { READ_ONLY_TOOLSET, VIBE_SEARCH_SERVER, type CriticToolset, type ToolCall } from '../../domain/critic-tools.js'
import { SEARCH_TOOLS } from '../../domain/packet-search.js'
import { parseFindings } from '../../domain/findings.js'
import { cliVersion } from '../cli-version.js'
import { isJsonObject, jsonObjectOrUndefined, type JsonObject } from '../../domain/json.js'
import { tokenCost, type TokenPrices } from '../../domain/pricing.js'
import { readToml, type TomlError, type TomlTable } from '../../../../shared-kernel/toml.js'
import type { ClientReadiness, RosterSlot, VibeCriticClient as VibeClientConfig } from '../../domain/routing.js'

/**
 * The Vibe critic client: runs one Vibe critic in its packet and reads its
 * review from stdout and the export file. The launch itself goes through the
 * model gateway's launch path, with the extras a critic needs: its agent, its
 * tools, and, when it searches, its own server with the home's switched off.
 * Everything Vibe-specific about reading the run (its exit codes, its journal
 * and export formats, its isolation) stays here.
 */
export class VibeCriticClient
  implements CriticClientPort<VibeClientConfig>, QuestionClientPort<VibeClientConfig>, SearchingClientPort<VibeClientConfig>
{
  private readonly gateway: ClientRun

  constructor(
    private readonly process: ProcessPort,
    private readonly files: CritiqueFilesPort,
    private readonly env: EnvPort,
    /** The platform the runner reports: it picks the keyring the sign-in is looked up in. */
    private readonly platform: NodeJS.Platform,
  ) {
    this.gateway = new ClientRun({ env, files, process })
  }

  /**
   * Whether the critic can run on the personal home: `vibe` on `PATH`, the
   * home signed in the way setup checks it (Vibe's item in the OS keyring:
   * the Keychain on macOS, the Secret Service on Linux; else a key in
   * `~/.vibe/.env`; `MISTRAL_API_KEY` does not count, since a critic runs
   * with it unset), and the `critic-<model>` agent setup installs. Nothing is
   * launched, so a client that is not ready falls back in prepare and costs
   * nothing.
   */
  async readiness(config: VibeClientConfig): Promise<ClientReadiness> {
    const signIn = await this.signIn(config)
    if (!signIn.ready) return signIn
    const home = join(this.env.require('HOME'), '.vibe')
    if (!(await this.files.exists(join(home, 'agents', `${config.agent}.toml`)))) {
      return { detail: `vibe: the critic agent ${config.agent} is not installed in ~/.vibe/agents; re-run setup`, ready: false }
    }
    return { ready: true }
  }

  /**
   * Why a critic of this agent cannot search, read from the installed agent
   * file before any launch. The agent file sits above the launch's
   * environment, so one that sets `enable_mcp = false` (an agent installed
   * before #255) switches the search server off whatever the launch asks,
   * and one that does not enable and approve the search tools has every
   * search call refused. Either way the critic reads alone, rather than
   * being launched to search and voided once it has spent.
   */
  async searchBlocked(config: VibeClientConfig): Promise<string | undefined> {
    const home = join(this.env.require('HOME'), '.vibe')
    const path = join(home, 'agents', `${config.agent}.toml`)
    if (!(await this.files.exists(path))) return `agent-file-missing: ~/.vibe/agents/${config.agent}.toml is not installed; re-run agent-kit setup`
    const agentProblem = agentFileSearchProblem(await this.files.readText(path), config.agent)
    if (agentProblem !== undefined) return agentProblem
    const homeConfig = join(home, 'config.toml')
    return (await this.files.exists(homeConfig)) ? homeConfigProblem(await this.files.readText(homeConfig)) : undefined
  }

  /** `vibe` on `PATH` and the personal home signed in, whatever critic agents are installed. */
  async signIn(config: VibeClientConfig): Promise<ClientReadiness> {
    if (!(await this.onPath(config.binary))) return { detail: `vibe: ${config.binary} is not on PATH`, ready: false }
    return (await this.signedIn(join(this.env.require('HOME'), '.vibe'))) ? { ready: true } : { detail: SIGN_IN, ready: false }
  }

  async version(config: VibeClientConfig): Promise<string | null> {
    return cliVersion(this.process, config.binary)
  }

  private async signedIn(home: string): Promise<boolean> {
    return vibeSignedIn({ files: this.files, process: this.process }, home, this.platform)
  }

  private async onPath(binary: string): Promise<boolean> {
    if (isAbsolute(binary)) return this.files.exists(binary)
    for (const dir of (this.env.get('PATH') ?? '').split(':').filter((entry) => entry.length > 0)) {
      if (await this.files.exists(join(dir, binary))) return true
    }
    return false
  }

  async review(launch: CriticLaunch, config: VibeClientConfig): Promise<CriticReport> {
    const exportDir = join(launch.criticDir, 'export')
    const exportPath = join(exportDir, 'export.json')
    const toolset = launch.toolset ?? READ_ONLY_TOOLSET
    const search = toolset.kind === 'vibe-search' ? toolset : undefined
    const paths = { exportDir, promptPath: launch.promptPath, workDir: launch.packetDir }
    const { result, run } = await this.launch(launch.critic, config, paths, search)
    const failure = search === undefined ? undefined : searchFailure(run.stdout)
    const tokens = tokensOf(run.exported)
    return {
      ...result,
      chargedUsd: vibeCharge(run, launch.critic.max_price_usd),
      exportPath,
      outcome: classifyVibeRun(run),
      ...(failure === undefined ? {} : { searchFailure: failure }),
      toolCalls: toolCalls(run.stdout),
      ...(tokens === undefined ? {} : { tokens }),
      vibeCostUsd: run.vibeCostUsd,
    }
  }

  /**
   * Puts one question to the agent's model, launched exactly as a critic is
   * (the same isolation, the same checks of the effective config), and
   * returns its final message as plain text.
   */
  async ask(question: ModelQuestion, config: VibeClientConfig): Promise<ModelAnswerReport> {
    const exportDir = join(question.outputDir, 'export')
    const { result, run } = await this.launch(question, config, { exportDir, promptPath: question.promptPath, workDir: question.workDir })
    const message = vibeMessage(run)
    const answer: ModelAnswer = message.status === 'failed' ? message : { cost_usd: message.cost_usd, status: 'answered', text: message.text }
    const tokens = tokensOf(run.exported)
    return {
      ...result,
      answer,
      chargedUsd: vibeCharge(run, question.max_price_usd),
      ...(tokens === undefined ? {} : { tokens }),
      vibeCostUsd: run.vibeCostUsd,
    }
  }

  /**
   * One Vibe launch through the gateway's launch path, and what its output and
   * export say about it. The gateway builds the command, applies the critic's
   * extras, runs it under its own time and price limits, and reads the export;
   * the runner prices the run and reads its journal itself.
   */
  private async launch(
    limits: VibeLimits,
    config: VibeClientConfig,
    paths: VibeLaunchPaths,
    search?: VibeSearch,
  ): Promise<{ readonly result: { exitCode: number; stdout: string; stderr: string }; readonly run: VibeRun }> {
    const launched = await this.gateway.launch({
      binary: config.binary,
      client: config.gateway?.client ?? 'vibe',
      extras: vibeExtras(config, search),
      model: config.gateway?.model ?? config.model,
      paths,
      prompt: await this.files.readText(paths.promptPath),
      time_limit_s: limits.time_limit_s,
      max_price_usd: limits.max_price_usd,
    })
    const exported = launched.exported
    const vibeCost = jsonObjectOrUndefined(exported)?.cost_usd
    const vibeCostUsd = typeof vibeCost === 'number' ? vibeCost : null
    const run: VibeRun = {
      costUsd: computedCost(jsonObjectOrUndefined(exported), config.prices) ?? vibeCostUsd,
      exitCode: launched.result.exitCode,
      expected: search === undefined ? { mcp: false } : { command: search.server.command, cwd: paths.workDir, mcp: true },
      exported,
      vibeCostUsd,
      stderr: launched.result.stderr,
      stdout: launched.result.stdout,
      timedOut: launched.result.timedOut === true,
      timeoutS: limits.time_limit_s + OUTER_TIMEOUT_GRACE_S,
    }
    return { result: { exitCode: launched.result.exitCode, stderr: launched.result.stderr, stdout: launched.result.stdout }, run }
  }
}

/** Setup's own warning for a personal Vibe home that is not signed in. */
const SIGN_IN = 'vibe: not signed in; run vibe and sign in with your Mistral account, then re-run setup'

/** Seconds the outer timer allows beyond Vibe's own `--time-limit`. */
const OUTER_TIMEOUT_GRACE_S = 60

/** The tools a critic agent may have enabled: `read_file`, and the search server's, which exist only while it runs. */
const AGENT_TOOLS: ReadonlySet<string> = new Set(['read_file', ...SEARCH_TOOLS.map((tool) => `${VIBE_SEARCH_SERVER}_${tool}`)])

/** A searching critic's launch: its search server, which the gateway attaches in the critic's packet copy. */
type VibeSearch = Extract<CriticToolset, { kind: 'vibe-search' }>

/**
 * What a Vibe critic asks the gateway for. Every critic runs its own agent,
 * which pins the model and its prices, with `read_file` and, when it searches
 * (ADR-0006), the runner's own server in its packet copy: the gateway
 * switches the home's servers off by name, so Vibe 3.0.4's joining of every
 * layer's servers leaves only the runner's live, and Mistral's hosted
 * connectors stay off. No sandbox profile is asked for: every Vibe launch is
 * the read-only profile, and the gateway refuses an extra it would not apply.
 */
function vibeExtras(config: VibeClientConfig, search: VibeSearch | undefined): RunExtras {
  const tools = ['read_file', ...(search?.search ?? []).map((tool) => `${VIBE_SEARCH_SERVER}_${tool}`)]
  if (search === undefined) return { agent: config.agent, tools }
  return {
    agent: config.agent,
    disable_home_mcp: true,
    env: { VIBE_ENABLE_CONNECTORS: 'false' },
    mcp_servers: [{ args: search.server.args, command: search.server.command, name: VIBE_SEARCH_SERVER }],
    tools,
  }
}

/** Where one launch works: a critic's packet copy, or a question's own directory. */
interface VibeLaunchPaths {
  /** The working directory: a critic's packet copy, or a question's own directory. */
  readonly workDir: string
  readonly promptPath: string
  readonly exportDir: string
}

/** The price cap and time limit one launch runs under. */
type VibeLimits = Pick<RosterSlot, 'max_price_usd' | 'time_limit_s'>

/** What the effective config must show: MCP off, or on with the runner's search server as its only live server. */
type ExpectedConfig = { readonly mcp: false } | { readonly mcp: true; readonly command: string; readonly cwd: string }

interface VibeRun {
  readonly exitCode: number
  readonly expected: ExpectedConfig
  readonly stdout: string
  readonly stderr: string
  /** Parsed `export.json`, or undefined when Vibe wrote none or it does not parse. */
  readonly exported: unknown
  /** The outer timer killed Vibe after `timeoutS` seconds. */
  readonly timedOut: boolean
  readonly timeoutS: number
  /** What the run cost: computed from the export's tokens at routing's prices, else Vibe's own figure. */
  readonly costUsd: number | null
  /** The export's `cost_usd`: what Vibe priced the run at by its own model table. */
  readonly vibeCostUsd: number | null
}

/**
 * The run's cost at the client's prices, from the export's token counts, or
 * undefined without prices or counts. Vibe's own `cost_usd` prices cached
 * input at the full rate when its model table has no cached price, which
 * made a cache-heavy critic look three times as costly as Mistral billed.
 */
function computedCost(exported: JsonObject | undefined, prices: TokenPrices | undefined): number | undefined {
  const usage = jsonObjectOrUndefined(exported?.usage)
  const count = (value: unknown) => (typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : undefined)
  const input = count(usage?.input_tokens)
  const output = count(usage?.output_tokens)
  if (prices === undefined || input === undefined || output === undefined) return undefined
  return tokenCost({ cached_input: count(usage?.cached_input_tokens) ?? 0, input, output }, prices)
}

/**
 * What the run used, from the export's token counts; undefined when the
 * export counts none. The gateway's ledger carries them beside the dollars,
 * so a critic run is counted the way a delegation is.
 */
function tokensOf(exported: unknown): CriticTokens | undefined {
  const usage = jsonObjectOrUndefined(jsonObjectOrUndefined(exported)?.usage)
  const count = (value: unknown) => (typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : undefined)
  const input = count(usage?.input_tokens)
  const output = count(usage?.output_tokens)
  if (input === undefined || output === undefined) return undefined
  return { cached: count(usage?.cached_input_tokens) ?? 0, input, output }
}

const EXIT_FAILURES: Readonly<Record<number, CriticFailure>> = {
  1: 'config-error',
  2: 'infrastructure-failure',
  3: 'stopped-early',
}

/** A critic's run is a review when its final message's last ```json block matches the output contract. */
function classifyVibeRun(run: VibeRun): CriticOutcome {
  const message = vibeMessage(run)
  if (message.status === 'failed') return message
  const parsed = parseFindings(message.text)
  if (!parsed.ok) return { cost_usd: message.cost_usd, detail: parsed.detail, failure: parsed.failure, status: 'failed' }
  return {
    cost_usd: message.cost_usd,
    findings: parsed.findings,
    status: 'ok',
    ...(parsed.no_findings_reason === undefined ? {} : { no_findings_reason: parsed.no_findings_reason }),
  }
}

type VibeMessage = { readonly status: 'ok'; readonly text: string; readonly cost_usd: number | null } | Extract<CriticOutcome, { status: 'failed' }>

/**
 * Exit code first: anything but 0 is a failed run, whatever stdout holds.
 * On 0, the export must show an isolated config, and stdout must end in an
 * assistant message, whose text is returned.
 */
function vibeMessage(run: VibeRun): VibeMessage {
  const exported = jsonObjectOrUndefined(run.exported)
  const cost = run.costUsd
  const failed = (failure: CriticFailure, detail: string): VibeMessage => ({ cost_usd: cost, detail, failure, status: 'failed' })

  if (run.timedOut) return failed('timeout', `vibe did not return within ${String(run.timeoutS)}s, so the outer timer killed it`)
  if (run.exitCode !== 0) {
    const outcome = typeof exported?.outcome === 'string' ? `outcome ${exported.outcome}` : run.stderr.trim()
    return failed(EXIT_FAILURES[run.exitCode] ?? `exit-${String(run.exitCode)}`, `vibe exited ${String(run.exitCode)}: ${outcome}`)
  }
  if (run.stdout.trim().length === 0) return failed('empty-output', 'vibe exited 0 with empty stdout')
  if (exported === undefined) return failed('missing-export', 'vibe wrote no readable export.json')
  const violations = isolationViolations(jsonObjectOrUndefined(exported.config), run.expected)
  if (violations.length > 0) return failed('unsafe-config', `effective config: ${violations.join('; ')}`)
  const message = finalAssistantText(run.stdout)
  if (message === undefined) return failed('unparseable-output', 'stdout is not a Vibe journal with an assistant message')
  return { cost_usd: cost, status: 'ok', text: message }
}

/**
 * What a Vibe run counts against the flow's cap. The run's cost is the base
 * (computed from its tokens, else Vibe's own figure), and it may exceed the
 * critic's cap: `--max-price` is checked only between turns. Where the
 * export cannot be trusted the cap is charged instead, as a conservative
 * estimate: no export or no cost (a killed run included), a deadline stop
 * (Vibe drops the interrupted turn's tokens and reports $0), and a zero cost
 * for tokens used (a model neither routing nor Vibe prices).
 */
function vibeCharge(run: VibeRun, cap: number): number {
  const exported = jsonObjectOrUndefined(run.exported)
  const cost = run.costUsd
  if (run.timedOut || cost === null) return cap
  if (exported?.outcome === 'deadline') return Math.max(cost, cap)
  return cost === 0 && usedTokens(jsonObjectOrUndefined(exported?.usage)) ? cap : cost
}

function usedTokens(usage: Readonly<Record<string, unknown>> | undefined): boolean {
  return [usage?.input_tokens, usage?.output_tokens].some((tokens) => typeof tokens === 'number' && tokens > 0)
}

function isolationViolations(config: Readonly<Record<string, unknown>> | undefined, expected: ExpectedConfig): readonly string[] {
  const violations: string[] = []
  if (config?.enable_mcp !== expected.mcp) violations.push(`enable_mcp is not ${String(expected.mcp)}`)
  if (expected.mcp) violations.push(...serverViolations(config?.mcp_servers, expected))
  const tools = config?.enabled_tools
  if (!Array.isArray(tools) || !tools.includes('read_file') || tools.some((tool) => typeof tool !== 'string' || !AGENT_TOOLS.has(tool))) {
    violations.push(`enabled_tools is not read_file and the search server's tools (${[...AGENT_TOOLS].join(', ')})`)
  }
  if (config?.include_project_context !== false) {
    violations.push('include_project_context is not false, so the AGENTS.md instructions pointer reaches the critic')
  }
  if (config?.outside_workdir !== 'deny') violations.push('outside_workdir is not deny')
  return violations
}

/** With MCP on, the only server not switched off must be the runner's own, in this critic's packet copy. */
function serverViolations(servers: unknown, expected: Extract<ExpectedConfig, { mcp: true }>): readonly string[] {
  const live = (Array.isArray(servers) ? servers.map(jsonObjectOrUndefined) : []).filter((server) => server?.disabled !== true)
  const others = live.filter((server) => server?.name !== VIBE_SEARCH_SERVER).map((server) => String(server?.name))
  const own = live.filter((server) => server?.name === VIBE_SEARCH_SERVER)
  const violations = others.length === 0 ? [] : [`MCP servers other than the search server are on: ${others.join(', ')}`]
  const [server] = own
  if (own.length !== 1 || server?.command !== expected.command || server.cwd !== expected.cwd) {
    violations.push(`the ${VIBE_SEARCH_SERVER} MCP server is not the runner's search server in this packet`)
  }
  return violations
}

/**
 * Every tool call the journal records, in order: its `effect` entries, each
 * with the tool's name (`file_system.bash`, `mcp_packet.grep`), its status
 * and, for a failed call, the error code (`tool_denied`).
 */
function toolCalls(stdout: string): readonly ToolCall[] {
  return journalEntries(stdout)
    .filter((entry) => entry?.type === 'effect')
    .map((entry) => {
      const state = jsonObjectOrUndefined(entry?.state)
      const code = jsonObjectOrUndefined(state?.error)?.code
      return {
        status: String(state?.status),
        tool: String(jsonObjectOrUndefined(entry?.detail)?.toolName),
        ...(typeof code === 'string' ? { code } : {}),
      }
    })
}

/**
 * Why a searching critic's searches failed, as its journal shows them: each
 * search call that did not complete, such as one made after the server
 * died. Undefined when every call completed.
 */
function searchFailure(stdout: string): string | undefined {
  const calls = journalEntries(stdout).filter(
    (entry) => entry?.type === 'effect' && String(jsonObjectOrUndefined(entry.detail)?.toolName).startsWith(`mcp_${VIBE_SEARCH_SERVER}.`),
  )
  const failed = calls.map((entry) => jsonObjectOrUndefined(entry?.state)).filter((state) => state?.status === 'failed')
  const [first] = failed
  if (first === undefined) return undefined
  const error = jsonObjectOrUndefined(first.error)
  return `${String(failed.length)} of ${String(calls.length)} search calls failed, the first with ${String(error?.code)}: ${String(error?.message)}`
}

/**
 * Why an agent file keeps its critics from searching, read as TOML: it
 * cannot be read, it switches MCP off, it declares MCP servers of its own
 * (they would run beside the search server, since Vibe joins every layer's
 * servers), or it leaves a search tool out of `enabled_tools` or without
 * `permission = "always"` (headless Vibe refuses every call that asks).
 * Undefined when it lets them search.
 */
export function agentFileSearchProblem(toml: string, agent: string): string | undefined {
  const file = `~/.vibe/agents/${agent}.toml`
  const fix = 're-run agent-kit setup to install the current critic agents'
  let doc: TomlTable
  try {
    doc = readToml(toml)
  } catch (error) {
    return `agent-file-unreadable: ${file} is not TOML the runner can read (${(error as TomlError).message}); ${fix}`
  }
  if (doc.enable_mcp === false) return `agent-file-disables-mcp: ${file} sets enable_mcp = false, which the launch cannot override; ${fix}`
  const servers = declaredServers(doc)
  if (servers !== undefined) {
    return `agent-file-declares-servers: ${file} declares MCP servers (${servers.join(', ')}) that would run beside the search server; ${fix}`
  }
  const enabled = Array.isArray(doc.enabled_tools) ? doc.enabled_tools : []
  const settings = isJsonObject(doc.tools) ? doc.tools : {}
  const unapproved = SEARCH_TOOLS.map((tool) => `${VIBE_SEARCH_SERVER}_${tool}`).filter(
    (tool) => !enabled.includes(tool) || jsonObjectOrUndefined(settings[tool])?.permission !== 'always',
  )
  if (unapproved.length > 0) return `agent-file-lacks-search-tools: ${file} does not enable and approve ${unapproved.join(', ')}; ${fix}`
  return undefined
}

/** The names of every `mcp_servers` the document declares, at any depth; undefined when it declares none. */
function declaredServers(table: TomlTable): readonly string[] | undefined {
  const declared: unknown[] = []
  const visit = (value: unknown): void => {
    if (Array.isArray(value)) value.forEach(visit)
    if (!isJsonObject(value)) return
    for (const [key, child] of Object.entries(value)) {
      if (key === 'mcp_servers') declared.push(child)
      visit(child)
    }
  }
  visit(table)
  return declared.length === 0 ? undefined : declared.flatMap(serverNames)
}

/** The `name` of each server in an `mcp_servers` value, `?` for one without. */
function serverNames(servers: unknown): readonly string[] {
  return (Array.isArray(servers) ? servers : [servers]).map((server) => {
    const name = jsonObjectOrUndefined(server)?.name
    return typeof name === 'string' ? name : '?'
  })
}

/**
 * Why the home's `config.toml` keeps a critic from searching: the runner
 * must read its MCP server names to switch them off, so a file it cannot
 * read blocks search. Undefined when it reads.
 */
export function homeConfigProblem(toml: string): string | undefined {
  try {
    readToml(toml)
    return undefined
  } catch (error) {
    return `home-config-unreadable: ~/.vibe/config.toml is not TOML the runner can read (${(error as TomlError).message}), so its MCP servers cannot be switched off by name`
  }
}

/**
 * The names of the MCP servers a Vibe `config.toml` declares, whether as
 * `[[mcp_servers]]` tables or an `mcp_servers = [...]` array of inline
 * tables; none for a file that is not TOML, which the pre-launch check has
 * already refused.
 */
export function mcpServerNames(toml: string): readonly string[] {
  try {
    const servers = readToml(toml).mcp_servers
    return servers === undefined ? [] : serverNames(servers).filter((name) => name !== '?')
  } catch {
    return []
  }
}

function journalEntries(stdout: string): readonly (Readonly<Record<string, unknown>> | undefined)[] {
  let journal: unknown
  try {
    journal = JSON.parse(stdout)
  } catch {
    return []
  }
  return Array.isArray(journal) ? journal.map(jsonObjectOrUndefined) : []
}

function finalAssistantText(stdout: string): string | undefined {
  const last = journalEntries(stdout).findLast((entry) => entry?.type === 'message' && entry.role === 'assistant')
  const content = Array.isArray(last?.content) ? last.content.map(jsonObjectOrUndefined) : undefined
  return content?.map((part) => (typeof part?.text === 'string' ? part.text : '')).join('')
}
