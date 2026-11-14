import { isAbsolute, join } from 'node:path'

import type { EnvPort, ProcessPort, ProcessResult } from '../../../../ports/index.js'
import type { CriticClientPort, CriticLaunch, CriticReport, CritiqueFilesPort } from '../../application/ports.js'
import type { CriticFailure, CriticOutcome, CriticTokens } from '../../domain/critic.js'
import { PI_TOOLS } from '../../domain/critic-tools.js'
import { parseFindings } from '../../domain/findings.js'
import { jsonObjectOrUndefined, type JsonObject } from '../../domain/json.js'
import type { ClientReadiness, PiCriticClient as PiClientConfig } from '../../domain/routing.js'
import { cliVersion } from '../cli-version.js'

/** The longest `pi auth check` and `pi --list-models` may take; they read local files and call no model. */
const STATUS_TIMEOUT_MS = 20_000

/**
 * Pi's system prompt for a critic, in place of its own, which offers to run
 * commands and points at Pi's documentation outside the packet. The critic's
 * instructions are the prompt itself, on stdin.
 */
export const PI_SYSTEM_PROMPT =
  'You are a read-only reviewer. Read and search only the files of your working directory, and answer exactly as the user message asks.'

/**
 * The Pi critic client: one critic through Pi on a LiteLLM route, in print
 * mode with its JSON event stream, the prompt on stdin and the critic's copy
 * of the packet as its working directory. Pi is started directly, not
 * through the gateway: the gateway has no Pi client. The launch is read-only
 * by its tools: only `read`, `grep`, `find` and `ls`, with every extension,
 * skill, prompt template, theme, context file, MCP server and trust-gated
 * project file switched off, and no session kept. Pi has no sandbox of its
 * own and its read tools run inside its own process, so the whole process is
 * confined by the operating system (`agent-launcher confine`, Landlock): it
 * can read only its packet copy, the Pi and node installs, the system runtime
 * and its own agent directory, and reach only the proxy's port. The agent
 * directory is made for the one launch, holds only the route's provider with
 * its key and is removed afterwards. A critic that cannot be confined does not
 * start. The stream is still checked afterwards as a second layer: any other
 * tool, or a path outside the packet, voids the review, and the tripwire
 * catches a write.
 */
export class PiCriticClient implements CriticClientPort<PiClientConfig> {
  constructor(
    private readonly process: ProcessPort,
    private readonly files: CritiqueFilesPort,
    private readonly env: EnvPort,
  ) {}

  /**
   * `pi` on `PATH` and the route's provider ready: `pi auth check` resolves
   * the provider's key the way a request does (setup writes it as a command
   * that reads the secret store) without printing it or calling a model.
   */
  async signIn(config: PiClientConfig): Promise<ClientReadiness> {
    if (!(await this.onPath(config.binary))) return { detail: `pi: ${config.binary} is not on PATH`, ready: false }
    const checked = await this.status(config.binary, ['auth', 'check', '--provider', config.provider, '--json'])
    return authReadiness(checked, config)
  }

  /**
   * Signed in, the provider offers the routed model, as `pi --list-models`
   * reads `models.json`, and the operating system can confine a critic.
   */
  async readiness(config: PiClientConfig): Promise<ClientReadiness> {
    const signIn = await this.signIn(config)
    if (!signIn.ready) return signIn
    const models = modelsReadiness(await this.status(config.binary, ['--offline', '--list-models', config.model]), config)
    if (!models.ready) return models
    const route = await this.route(config)
    if (typeof route === 'string') return { detail: route, ready: false }
    const launcher = await this.launcher()
    if (launcher === undefined) return { detail: NO_LAUNCHER, ready: false }
    const checked = await this.status(launcher, ['confine', '--check', ...(await this.grants(config, route))])
    return confineReadiness(checked)
  }

  async version(config: PiClientConfig): Promise<string | null> {
    return cliVersion(this.process, config.binary)
  }

  async review(launch: CriticLaunch, config: PiClientConfig): Promise<CriticReport> {
    const prompt = await this.files.readText(launch.promptPath)
    const agentDir = join(launch.criticDir, 'pi-agent')
    const prepared = await this.prepare(config, agentDir)
    if (typeof prepared === 'string') return unconfinedReport(launch, prepared)
    const command = confinedCommand(prepared.launcher, await this.grants(config, prepared.route, launch.packetDir, agentDir), piCommand(config, launch, prompt), this.env.get('PATH'), agentDir, prepared.bin)
    const result = await this.process.exec(command).finally(() => this.files.removeTree(agentDir))
    const stream = readPiStream(result.stdout)
    const exportPath = join(launch.criticDir, 'result.json')
    await this.files.writeText(
      exportPath,
      `${JSON.stringify({ answer: stream.answer ?? null, cost_usd: stream.costUsd, tokens: stream.tokens ?? null }, null, 2)}\n`,
    )
    return {
      ...result,
      // Pi spends on its LiteLLM route's budget, never the flow's Vibe cap; the gateway ledger carries what Pi priced it at.
      chargedUsd: stream.costUsd ?? 0,
      exportPath,
      outcome: classifyPiRun(result, stream, launch.packetDir, launch.critic.time_limit_s),
      ...(stream.tokens === undefined ? {} : { tokens: stream.tokens }),
    }
  }

  /**
   * The launch's own agent directory: a `models.json` holding only the
   * route's provider, its key read from the secret store just now and written
   * in plain (mode 0600), and an empty `settings.json`; or why there is none.
   */
  private async prepare(config: PiClientConfig, agentDir: string): Promise<string | { launcher: string; route: PiRoute; bin: string | undefined }> {
    const route = await this.route(config)
    if (typeof route === 'string') return route
    const launcher = await this.launcher()
    if (launcher === undefined) return NO_LAUNCHER
    const key = await this.key(config, route.apiKey)
    if (key.error !== undefined) return key.error
    const provider = { api: route.api, apiKey: key.value, baseUrl: route.baseUrl, models: [route.model] }
    // The key goes in last, so a write that fails leaves no key behind.
    await this.files.writePrivate(join(agentDir, 'settings.json'), '{}\n')
    await this.files.writePrivate(join(agentDir, 'models.json'), `${JSON.stringify({ providers: { [config.provider]: provider } }, null, 2)}\n`)
    return { bin: await this.homeBin(), launcher, route }
  }

  /** The route's provider as the home's `models.json` holds it, or why a confined critic cannot use it. */
  private async route(config: PiClientConfig): Promise<PiRoute | string> {
    const path = join(this.homeAgentDir(), 'models.json')
    const text = await this.files.readText(path).catch(() => undefined)
    const providers = jsonObjectOrUndefined(text === undefined ? undefined : parseObject(text)?.providers)
    return routeOf(jsonObjectOrUndefined(providers?.[config.provider]), config, path)
  }

  /** The key: a literal as it is, or `!<program> get <NAME>` run without a shell; never a shell command. */
  private async key(config: PiClientConfig, apiKey: string): Promise<{ value: string; error?: undefined } | { error: string }> {
    if (!apiKey.startsWith('!')) return { value: apiKey }
    const words = apiKey.slice(1).trim().split(/\s+/)
    if (words.length !== 3 || words[1] !== 'get') return { error: `pi: the ${config.provider} key is a shell command the runner does not run; setup writes it as !agent-kit-secret get <NAME>` }
    const [program = '', , name = ''] = words
    const read = await this.status(program, ['get', name])
    const value = typeof read === 'string' || read.timedOut === true || read.exitCode !== 0 ? '' : read.stdout.trim()
    if (value.length > 0) return { value }
    // Only the error stream is shown: a failed read never puts anything of the key in a message.
    const said = typeof read === 'string' ? read : (firstLine(read.stderr) ?? `exit ${String(read.exitCode)}`)
    return { error: `pi: the ${config.provider} key could not be read from the secret store (${name}): ${said}` }
  }

  /** The grants of a confined launch: the packet and the agent directory when there is a launch, Pi, its tools and the proxy's port. */
  private async grants(config: PiClientConfig, route: PiRoute, packetDir?: string, agentDir?: string): Promise<readonly string[]> {
    const bin = await this.homeBin()
    return [
      ...(packetDir === undefined ? [] : ['--read', packetDir]),
      '--run-from',
      config.binary,
      ...(bin === undefined ? [] : ['--run', bin]),
      ...(agentDir === undefined ? [] : ['--write', agentDir]),
      '--net-connect',
      String(route.port),
    ]
  }

  /** The home's Pi directory: `PI_CODING_AGENT_DIR`, else `~/.pi/agent`. */
  private homeAgentDir(): string {
    const own = this.env.get('PI_CODING_AGENT_DIR') ?? ''
    return own.length > 0 ? own : join(this.env.get('HOME') ?? '', '.pi', 'agent')
  }

  /** The home's Pi `bin`, where Pi keeps the `fd` its find tool runs, when there is one. */
  private async homeBin(): Promise<string | undefined> {
    const bin = join(this.homeAgentDir(), 'bin')
    return (await this.files.exists(bin)) ? bin : undefined
  }

  /** The launcher that confines a critic: `AGENT_KIT_LAUNCHER_BIN`, else `agent-launcher` on `PATH`. */
  private async launcher(): Promise<string | undefined> {
    const named = this.env.get('AGENT_KIT_LAUNCHER_BIN') ?? ''
    if (named.length > 0) return named
    return (await this.onPath('agent-launcher')) ? 'agent-launcher' : undefined
  }

  private async status(binary: string, args: readonly string[]): Promise<ProcessResult | string> {
    return this.process
      .exec({ args, command: binary, timeoutMs: STATUS_TIMEOUT_MS })
      .catch((error: unknown) => (error instanceof Error ? error.message : String(error)))
  }

  private async onPath(binary: string): Promise<boolean> {
    if (isAbsolute(binary)) return this.files.exists(binary)
    for (const dir of (this.env.get('PATH') ?? '').split(':').filter((entry) => entry.length > 0)) {
      if (await this.files.exists(join(dir, binary))) return true
    }
    return false
  }
}

/** Why a Pi critic cannot run without the launcher. */
const NO_LAUNCHER = 'pi: agent-launcher is not on PATH, and a Pi critic only runs confined to its packet; re-run agent-kit setup, which installs it'

/** The launcher's exit code when it could not apply the confinement and started nothing. */
export const CONFINE_REFUSED = 125

/** What a confined launch needs of the route's provider in the home's `models.json`. */
export interface PiRoute {
  readonly baseUrl: string
  readonly api: string
  readonly apiKey: string
  /** The routed model's entry, as the home holds it (its prices included). */
  readonly model: JsonObject
  /** The proxy's TCP port, the one port a critic may connect to. */
  readonly port: number
}

/** The port of an `http` or `https` URL: the one after the host, else the scheme's own. */
export function urlPort(url: string): number | undefined {
  const scheme = url.startsWith('https://') ? 'https' : url.startsWith('http://') ? 'http' : undefined
  if (scheme === undefined) return undefined
  const authority = url.slice(scheme.length + 3).split('/')[0] ?? ''
  const colon = authority.lastIndexOf(':')
  if (colon < 0) return scheme === 'https' ? 443 : 80
  const port = authority.slice(colon + 1)
  return /^[0-9]{1,5}$/.test(port) && Number(port) > 0 && Number(port) < 65536 ? Number(port) : undefined
}

/** The route from the home's provider entry, or why it cannot be used. */
export function routeOf(provider: JsonObject | undefined, config: PiClientConfig, path: string): PiRoute | string {
  const fix = 're-run agent-kit setup so it writes the LiteLLM providers into ~/.pi/agent/models.json'
  if (provider === undefined) return `pi: ${path} has no ${config.provider} provider; ${fix}`
  const { api, apiKey, baseUrl } = provider
  if (typeof baseUrl !== 'string' || typeof api !== 'string' || typeof apiKey !== 'string') {
    return `pi: the ${config.provider} provider in ${path} lacks a baseUrl, api or apiKey; ${fix}`
  }
  const port = urlPort(baseUrl)
  if (port === undefined) return `pi: the ${config.provider} provider's baseUrl is not an http or https URL with a port the confinement can allow`
  const models = Array.isArray(provider.models) ? provider.models.map(jsonObjectOrUndefined) : []
  const model = models.find((entry) => entry?.id === config.model) ?? { id: config.model }
  return { api, apiKey, baseUrl, model, port }
}

/** What `agent-launcher confine --check` says. */
export function confineReadiness(checked: ProcessResult | string): ClientReadiness {
  if (typeof checked !== 'string' && checked.timedOut !== true && checked.exitCode === 0) return { ready: true }
  return { detail: `pi: the operating system cannot confine a Pi critic to its packet here, so it does not run: ${unreadable(checked)}`, ready: false }
}

/**
 * The Pi launch run through the launcher, confined: the launcher's grants,
 * then `--` and Pi's own command line. Pi reads its agent directory from
 * `PI_CODING_AGENT_DIR`, keeps its temporary files there, and finds the
 * home's `fd` on `PATH`.
 */
export function confinedCommand(
  launcher: string,
  grants: readonly string[],
  pi: ReturnType<typeof piCommand>,
  path: string | undefined,
  agentDir: string,
  bin: string | undefined,
) {
  const searched = bin === undefined ? (path ?? '') : [bin, ...(path === undefined || path.length === 0 ? [] : [path])].join(':')
  return {
    ...pi,
    args: ['confine', ...grants, '--', pi.command, ...pi.args],
    command: launcher,
    env: { PATH: searched, PI_CODING_AGENT_DIR: agentDir, TMPDIR: agentDir },
  }
}

/** A Pi critic that never started because it could not be confined. */
function unconfinedReport(launch: CriticLaunch, detail: string): CriticReport {
  return {
    chargedUsd: 0,
    exitCode: CONFINE_REFUSED,
    exportPath: join(launch.criticDir, 'result.json'),
    outcome: { cost_usd: null, detail, failure: 'unconfined', status: 'failed' },
    stderr: '',
    stdout: '',
  }
}

/** One Pi critic's launch: print mode, the JSON stream, the read tools alone, and nothing of the home's or the packet's own. */
export function piCommand(config: PiClientConfig, launch: CriticLaunch, prompt: string) {
  return {
    args: [
      '-p',
      '--mode',
      'json',
      '--no-session',
      '--offline',
      '--provider',
      config.provider,
      '--model',
      config.model,
      '--tools',
      PI_TOOLS.join(','),
      '--no-extensions',
      '--no-skills',
      '--no-prompt-templates',
      '--no-themes',
      '--no-context-files',
      '--no-mcp',
      '--no-approve',
      '--system-prompt',
      PI_SYSTEM_PROMPT,
    ],
    command: config.binary,
    cwd: launch.packetDir,
    stdin: prompt,
    // Pi has no time limit of its own, so the outer timer is the limit.
    timeoutMs: launch.critic.time_limit_s * 1000,
  }
}

/** Why a status command could not say, as one clause. */
function unreadable(result: ProcessResult | string): string {
  if (typeof result === 'string') return result
  if (result.timedOut === true) return `no answer within ${String(STATUS_TIMEOUT_MS / 1000)}s`
  return firstLine(`${result.stdout}\n${result.stderr}`) ?? `exit ${String(result.exitCode)}`
}

function firstLine(text: string): string | undefined {
  return text
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line.length > 0)
}

/** What `pi auth check --json` says: `{"status": "ready" | "not_ready" | "invalid", "reason"?}`. */
export function authReadiness(checked: ProcessResult | string, config: PiClientConfig): ClientReadiness {
  const fix = 're-run agent-kit setup so it writes the LiteLLM providers into ~/.pi/agent/models.json'
  if (typeof checked === 'string' || checked.timedOut === true) {
    return { detail: `pi: auth check for ${config.provider} failed: ${unreadable(checked)}`, ready: false }
  }
  const answer = parseObject(checked.stdout.trim())
  if (answer?.status === 'ready') return { ready: true }
  if (typeof answer?.status !== 'string') return { detail: `pi: auth check for ${config.provider} failed: ${unreadable(checked)}`, ready: false }
  const reason = typeof answer.reason === 'string' ? ` (${answer.reason})` : ''
  return { detail: `pi: the ${config.provider} provider is ${answer.status}${reason}; ${fix}`, ready: false }
}

/** Whether `pi --list-models <model>` lists the routed model under the route's provider: its first two columns. */
export function modelsReadiness(listed: ProcessResult | string, config: PiClientConfig): ClientReadiness {
  if (typeof listed === 'string' || listed.timedOut === true || listed.exitCode !== 0) {
    return { detail: `pi: ${config.binary} --list-models failed: ${unreadable(listed)}`, ready: false }
  }
  const offered = listed.stdout.split('\n').some((line) => {
    const [provider, model] = line.trim().split(/\s+/)
    return provider === config.provider && model === config.model
  })
  return offered ? { ready: true } : { detail: `pi: the ${config.provider} provider does not offer ${config.model}`, ready: false }
}

/** One tool call of the stream: its tool and its arguments. */
interface PiToolCall {
  readonly tool: string
  readonly args: JsonObject | undefined
}

/** What Pi's JSON event stream held. */
export interface PiStream {
  /** False when a line of stdout is not a JSON object. */
  readonly parsed: boolean
  readonly calls: readonly PiToolCall[]
  /** The last assistant message, which ends the run. */
  readonly last: JsonObject | undefined
  /** The last assistant message's text. */
  readonly answer: string | undefined
  /** What Pi priced every assistant message at, summed; null when none carried a price. */
  readonly costUsd: number | null
  readonly tokens: CriticTokens | undefined
}

function parseObject(text: string): JsonObject | undefined {
  try {
    return jsonObjectOrUndefined(JSON.parse(text))
  } catch {
    return undefined
  }
}

function nonNegative(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : undefined
}

/**
 * Reads the stream line by line: each tool call's start, and each assistant
 * message's end with its usage. Pi's `input` leaves out the cached prompt,
 * which it counts as `cacheRead`, so the input recorded is both.
 */
export function readPiStream(stdout: string): PiStream {
  const calls: PiToolCall[] = []
  let last: JsonObject | undefined
  let cost: number | null = null
  let tokens: CriticTokens | undefined
  for (const line of stdout.split('\n').filter((text) => text.trim().length > 0)) {
    const event = parseObject(line)
    if (event === undefined) return { answer: undefined, calls, costUsd: cost, last: undefined, parsed: false, tokens }
    if (event.type === 'tool_execution_start') calls.push({ args: jsonObjectOrUndefined(event.args), tool: String(event.toolName) })
    const message = event.type === 'message_end' ? jsonObjectOrUndefined(event.message) : undefined
    if (message?.role !== 'assistant') continue
    last = message
    const usage = jsonObjectOrUndefined(message.usage)
    const price = nonNegative(jsonObjectOrUndefined(usage?.cost)?.total)
    if (price !== undefined) cost = (cost ?? 0) + price
    const input = nonNegative(usage?.input)
    const output = nonNegative(usage?.output)
    if (input === undefined || output === undefined) continue
    const cached = nonNegative(usage?.cacheRead) ?? 0
    tokens = { cached: (tokens?.cached ?? 0) + cached, input: (tokens?.input ?? 0) + input + cached, output: (tokens?.output ?? 0) + output }
  }
  return { answer: last === undefined ? undefined : messageText(last), calls, costUsd: cost, last, parsed: true, tokens }
}

function messageText(message: JsonObject): string {
  const content = Array.isArray(message.content) ? message.content.map(jsonObjectOrUndefined) : []
  return content.map((part) => (part?.type === 'text' && typeof part.text === 'string' ? part.text : '')).join('')
}

/**
 * The segments of `path` once `.` and `..` are resolved lexically, under the
 * segments it starts from; undefined when a `..` climbs above them.
 */
function resolvedSegments(start: readonly string[], path: string): readonly string[] | undefined {
  const segments = [...start]
  for (const segment of path.split('/')) {
    if (segment === '' || segment === '.') continue
    if (segment !== '..') {
      segments.push(segment)
      continue
    }
    if (segments.length === start.length) return undefined
    segments.pop()
  }
  return segments
}

/**
 * Whether a path a tool was given reaches outside the packet: Pi resolves a
 * relative path against its working directory, takes an absolute one as it
 * is, expands `~`, and drops a leading `@`.
 */
export function outsidePacket(raw: string, packetDir: string): boolean {
  const path = raw.startsWith('@') ? raw.slice(1) : raw
  if (path.startsWith('~')) return true
  if (!path.startsWith('/')) return resolvedSegments([], path) === undefined
  const root = packetDir.split('/').filter((segment) => segment.length > 0)
  const segments = resolvedSegments([], path) ?? []
  return segments.length < root.length || root.some((segment, index) => segments[index] !== segment)
}

/** Whether a glob a tool was given can match outside the packet: an absolute or home glob, or one that climbs with `..`. */
function globOutside(glob: string): boolean {
  return glob.startsWith('/') || glob.startsWith('~') || glob.split('/').includes('..')
}

/** Every call that reached outside the packet, as `<tool> <argument>`, in order and once each. */
export function outsideCalls(calls: readonly PiToolCall[], packetDir: string): readonly string[] {
  const outside: string[] = []
  for (const call of calls) {
    const path = call.args?.path
    const glob = call.args?.glob
    const pattern = call.tool === 'find' ? call.args?.pattern : undefined
    const reached = [
      ...(typeof path === 'string' && outsidePacket(path, packetDir) ? [path] : []),
      ...[glob, pattern].filter((value): value is string => typeof value === 'string' && globOutside(value)),
    ]
    for (const argument of reached) {
      const shown = `${call.tool} ${argument}`
      if (!outside.includes(shown)) outside.push(shown)
    }
  }
  return outside
}

/**
 * Pi exits 0 after a provider error too, so the stream decides: the run must
 * end in an assistant message that stopped normally, its tools must be the
 * read tools alone, every path they took must stay in the packet, and its
 * text's last ```json block must match the output contract.
 */
export function classifyPiRun(result: ProcessResult, stream: PiStream, packetDir: string, timeLimitS: number): CriticOutcome {
  const cost = stream.costUsd
  const failed = (failure: CriticFailure, detail: string): CriticOutcome => ({ cost_usd: cost, detail, failure, status: 'failed' })
  if (result.timedOut === true) return failed('timeout', `pi did not return within ${String(timeLimitS)}s, so the outer timer killed it`)
  if (result.exitCode === CONFINE_REFUSED) {
    return failed('unconfined', `the operating system could not confine pi to its packet, so it did not start: ${firstLine(result.stderr) ?? 'exit 125'}`)
  }
  if (result.exitCode !== 0) {
    const said = firstLine(result.stderr)
    return failed(`exit-${String(result.exitCode)}`, `pi exited ${String(result.exitCode)}${said === undefined ? '' : `: ${said}`}`)
  }
  if (result.stdout.trim().length === 0) return failed('empty-output', 'pi exited 0 with empty stdout')
  if (!stream.parsed) return failed('unparseable-output', 'stdout is not a Pi JSON event stream')
  const unsafe = [...new Set(stream.calls.map((call) => call.tool).filter((tool) => !(PI_TOOLS as readonly string[]).includes(tool)))]
  if (unsafe.length > 0) return failed('unsafe-tool', `the critic used tools beyond reading its packet: ${unsafe.join(', ')}`)
  const outside = outsideCalls(stream.calls, packetDir)
  if (outside.length > 0) return failed('unsafe-tool', `the critic reached outside its packet: ${outside.join(', ')}`)
  const { last } = stream
  if (last === undefined) return failed('unparseable-output', 'stdout is not a Pi event stream with an assistant message')
  const error = typeof last.errorMessage === 'string' ? `: ${last.errorMessage}` : ''
  if (last.stopReason === 'error') return failed('infrastructure-failure', `pi stopped on an error${error}`)
  if (last.stopReason === 'aborted') return failed('stopped-early', `pi was aborted${error}`)
  const parsed = parseFindings(stream.answer ?? '')
  if (!parsed.ok) return failed(parsed.failure, parsed.detail)
  return {
    cost_usd: cost,
    findings: parsed.findings,
    status: 'ok',
    ...(parsed.no_findings_reason === undefined ? {} : { no_findings_reason: parsed.no_findings_reason }),
  }
}
