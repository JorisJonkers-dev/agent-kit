import { isAbsolute, join } from 'node:path'

import type { EnvPort, ProcessCommand, ProcessPort } from '../../../../ports/index.js'
import type { CriticClientPort, CriticLaunch, CriticReport, CritiqueFilesPort } from '../../application/ports.js'
import type { CriticFailure, CriticOutcome } from '../../domain/critic.js'
import { readFindings } from '../../domain/findings.js'
import { jsonObjectOrUndefined, type JsonObject } from '../../domain/json.js'
import type { AgyCriticClient as AgyClientConfig, ClientReadiness } from '../../domain/routing.js'

/** Setup's own warning for a machine whose Keychain has no `Antigravity CLI` item. */
const SIGN_IN = 'antigravity: needs sign-in: run agy and sign in with your Google account'

/** Seconds the outer timer allows beyond agy's own `--print-timeout`. */
const OUTER_TIMEOUT_GRACE_S = 60

/**
 * Settings for this session only; agy never saves them. Deny beats any allow
 * rule in the owner's `settings.json`, so the critic cannot call an MCP tool,
 * fetch or drive a URL, run a command or write a file, whatever that file
 * allows. agy has no session switch for loading the MCP servers themselves or
 * the global `AGENTS.md`; the integration document records that gap.
 */
const SESSION_SETTINGS = {
  permissions: {
    deny: ['mcp(*)', 'read_url(*)', 'execute_url(*)', 'command(*)', 'write_file(*)'],
  },
}

/**
 * The tools a critic may use: reading, listing and searching its packet, and
 * `finish`, through which agy returns structured output. Any other tool in the
 * stream (MCP, web search, browser, workflows, subagents, commands, edits)
 * voids the review, even when agy refused it.
 */
const READ_ONLY_TOOLS: ReadonlySet<string> = new Set(['read_file', 'view_file', 'list_dir', 'grep_search', 'find_by_name', 'finish'])

/**
 * The Antigravity critic client: runs one Gemini critic through `agy` in
 * print mode and reads its review from the `stream-json` events. Everything
 * agy-specific (the launch, the isolation it needs, its readiness check and
 * its output format) stays here.
 */
export class AgyCriticClient implements CriticClientPort<AgyClientConfig> {
  constructor(
    private readonly process: ProcessPort,
    private readonly files: CritiqueFilesPort,
    private readonly env: EnvPort,
    private readonly platform: NodeJS.Platform,
  ) {}

  /**
   * Setup's own check: Antigravity is set up on macOS only, `agy` must be on
   * `PATH`, and the Keychain must hold an `Antigravity CLI` item.
   */
  async readiness(config: AgyClientConfig): Promise<ClientReadiness> {
    if (this.platform !== 'darwin') return { detail: `antigravity: macOS only; not set up on ${this.platform}`, ready: false }
    if (!(await this.onPath(config.binary))) return { detail: `antigravity: ${config.binary} is not on PATH`, ready: false }
    const keychain = await this.process
      .exec({ args: ['find-generic-password', '-s', 'Antigravity CLI'], command: 'security' })
      .catch(() => ({ exitCode: 1 }))
    return keychain.exitCode === 0 ? { ready: true } : { detail: SIGN_IN, ready: false }
  }

  async review(launch: CriticLaunch, config: AgyClientConfig): Promise<CriticReport> {
    const prompt = await this.files.readText(launch.promptPath)
    const result = await this.process.exec(agyLaunch(launch, config, prompt))
    const run = readStream(result.stdout)
    const exportPath = join(launch.criticDir, 'result.json')
    if (run.result !== undefined) await this.files.writeText(exportPath, `${JSON.stringify(run.result, null, 2)}\n`)
    const outcome =
      result.timedOut === true
        ? timedOut(launch.critic.time_limit_s + OUTER_TIMEOUT_GRACE_S)
        : classifyAgyRun(result.exitCode, result.stdout, run)
    return { ...result, exportPath, outcome }
  }

  private async onPath(binary: string): Promise<boolean> {
    if (isAbsolute(binary)) return this.files.exists(binary)
    for (const dir of (this.env.get('PATH') ?? '').split(':').filter((entry) => entry.length > 0)) {
      if (await this.files.exists(join(dir, binary))) return true
    }
    return false
  }
}

/**
 * A fresh print-mode session in the packet: no `--continue`, `--conversation`
 * or project, so a critic never resumes an earlier one. Slash commands and
 * skill expansion are off, the sandbox is on, and plan mode is left out
 * because agy ignores it once slash commands are disabled. Stdin is closed.
 */
function agyLaunch(launch: CriticLaunch, config: AgyClientConfig, prompt: string): ProcessCommand {
  const timeLimit = launch.critic.time_limit_s
  return {
    args: [
      '-p',
      prompt,
      '--output-format',
      'stream-json',
      '--model',
      config.model,
      '--print-timeout',
      `${String(timeLimit)}s`,
      '--disable-slash-commands',
      '--sandbox',
      '--json-schema',
      launch.schemaPath,
      '--settings',
      JSON.stringify(SESSION_SETTINGS),
    ],
    command: config.binary,
    cwd: launch.packetDir,
    timeoutMs: (timeLimit + OUTER_TIMEOUT_GRACE_S) * 1000,
  }
}

interface AgyStream {
  /** False when a line of stdout is not a JSON object. */
  readonly parsed: boolean
  readonly tools: readonly string[]
  readonly result: JsonObject | undefined
}

function readStream(stdout: string): AgyStream {
  const tools: string[] = []
  let result: JsonObject | undefined
  for (const line of stdout.split('\n').filter((text) => text.trim().length > 0)) {
    let event: JsonObject | undefined
    try {
      event = jsonObjectOrUndefined(JSON.parse(line))
    } catch {
      event = undefined
    }
    if (event === undefined) return { parsed: false, result: undefined, tools }
    const step = jsonObjectOrUndefined(event.step_update)
    if (step?.step_type === 'tool' && typeof step.tool_name === 'string') tools.push(step.tool_name)
    if (event.event === 'result') result = jsonObjectOrUndefined(event.result)
  }
  return { parsed: true, result, tools }
}

function timedOut(timeoutS: number): CriticOutcome {
  return { cost_usd: null, detail: `agy did not return within ${String(timeoutS)}s, so the outer timer killed it`, failure: 'timeout', status: 'failed' }
}

/**
 * agy reports `SUCCESS` and exits 0 for runs that produced nothing, so the
 * fields decide: an empty response, a denied action, a print timeout or a
 * missing `structured_output` is a failed critic whatever the exit code, and
 * so is any tool beyond reading the packet. Findings come from
 * `structured_output` only, checked against the output contract again.
 */
function classifyAgyRun(exitCode: number, stdout: string, run: AgyStream): CriticOutcome {
  const failed = (failure: CriticFailure, detail: string): CriticOutcome => ({ cost_usd: null, detail, failure, status: 'failed' })
  if (exitCode !== 0) return failed(`exit-${String(exitCode)}`, `agy exited ${String(exitCode)}`)
  if (stdout.trim().length === 0) return failed('empty-output', 'agy exited 0 with empty stdout')
  const { result } = run
  if (result === undefined) return failed('unparseable-output', 'stdout is not an agy event stream with a result event')
  const unsafe = [...new Set(run.tools.filter((tool) => !READ_ONLY_TOOLS.has(tool)))]
  if (unsafe.length > 0) return failed('unsafe-tool', `the critic used tools beyond reading its packet: ${unsafe.join(', ')}`)
  if (result.finished_reason === 'print_timeout') return failed('print-timeout', 'agy stopped at its print timeout')
  const denied = Array.isArray(result.denied_actions) ? result.denied_actions.map(jsonObjectOrUndefined) : []
  if (denied.length > 0) {
    return failed('denied-action', `headless agy denied: ${denied.map((action) => String(action?.action)).join(', ')}`)
  }
  if (typeof result.response !== 'string' || result.response.trim().length === 0) {
    return failed('empty-output', 'agy returned an empty response')
  }
  if (result.structured_output === undefined) {
    return failed('missing-structured-output', 'the reply did not match the finding schema, so agy returned no structured_output')
  }
  const parsed = readFindings(result.structured_output)
  if (!parsed.ok) return failed(parsed.failure, parsed.detail)
  return {
    cost_usd: null,
    findings: parsed.findings,
    status: 'ok',
    ...(parsed.no_findings_reason === undefined ? {} : { no_findings_reason: parsed.no_findings_reason }),
  }
}
