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
 * own, so the stream is checked afterwards: any other tool, or a path outside
 * the packet, voids the review, and the tripwire catches a write.
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

  /** Signed in, and the provider offers the routed model, as `pi --list-models` reads `models.json`. */
  async readiness(config: PiClientConfig): Promise<ClientReadiness> {
    const signIn = await this.signIn(config)
    if (!signIn.ready) return signIn
    return modelsReadiness(await this.status(config.binary, ['--offline', '--list-models', config.model]), config)
  }

  async version(config: PiClientConfig): Promise<string | null> {
    return cliVersion(this.process, config.binary)
  }

  async review(launch: CriticLaunch, config: PiClientConfig): Promise<CriticReport> {
    const result = await this.process.exec(piCommand(config, launch, await this.files.readText(launch.promptPath)))
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
