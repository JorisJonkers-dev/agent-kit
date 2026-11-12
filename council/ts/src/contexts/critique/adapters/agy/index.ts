import { isAbsolute, join } from 'node:path'

import { ClientRun, type RunExtras } from '../../../model-gateway/index.js'
import type { EnvPort, ProcessPort, ProcessResult } from '../../../../ports/index.js'
import type { CriticClientPort, CriticLaunch, CriticReport, CritiqueFilesPort } from '../../application/ports.js'
import type { CriticFailure, CriticOutcome } from '../../domain/critic.js'
import { READ_ONLY_TOOLSET, toolNames } from '../../domain/critic-tools.js'
import { readFindings } from '../../domain/findings.js'
import { jsonObjectOrUndefined, type JsonObject } from '../../domain/json.js'
import type { AgyCriticClient as AgyClientConfig, ClientReadiness } from '../../domain/routing.js'
import { cliVersion } from '../cli-version.js'

/** Setup's own warning for a machine where `agy models` asks for a sign-in. */
const SIGN_IN = 'antigravity: needs sign-in: run agy and sign in with your Google account'

/**
 * The longest `agy models` may take. Signed in it answers in about a second;
 * stdin is closed, so it cannot wait on a prompt either.
 */
const MODELS_TIMEOUT_MS = 20_000

/** The progress line `agy models` prints first, which says nothing about the outcome. */
const MODELS_PROGRESS = 'Fetching available models...'

/** Seconds the outer timer allows beyond agy's own `--print-timeout`. */
const OUTER_TIMEOUT_GRACE_S = 60

/**
 * The tools a critic may use: reading, listing and searching its packet, and
 * `finish`, through which agy returns structured output. Any other tool in the
 * stream (MCP, web search, browser, workflows, subagents, commands, edits)
 * voids the review, even when agy refused it.
 */
const READ_ONLY_TOOLS: ReadonlySet<string> = new Set(['read_file', 'view_file', 'list_dir', 'grep_search', 'find_by_name', 'finish'])

/**
 * The Antigravity critic client: runs one Gemini critic through `agy` in
 * print mode and reads its review from the `stream-json` events. The launch
 * goes through the model gateway's launch path, with the extras a critic
 * needs: the read-only sandbox profile, the finding schema and the tools its
 * prompt names. Everything agy-specific about reading the run (its readiness
 * check, its event stream and its exit codes) stays here.
 */
export class AgyCriticClient implements CriticClientPort<AgyClientConfig> {
  private readonly gateway: ClientRun

  constructor(
    private readonly process: ProcessPort,
    private readonly files: CritiqueFilesPort,
    private readonly env: EnvPort,
  ) {
    this.gateway = new ClientRun({ env, files, process })
  }

  /** agy has no per-critic file, so being signed in is all a critic needs. */
  async signIn(config: AgyClientConfig): Promise<ClientReadiness> {
    return this.readiness(config)
  }

  async version(config: AgyClientConfig): Promise<string | null> {
    return cliVersion(this.process, config.binary)
  }

  /**
   * Setup's own check: `agy` (installed on macOS and Linux alike) must be on
   * `PATH`, and `agy models` must list the routed model. Listing the models
   * needs a signed-in account but calls no model, so it spends no quota and
   * answers even while the quota is spent.
   */
  async readiness(config: AgyClientConfig): Promise<ClientReadiness> {
    if (!(await this.onPath(config.binary))) return { detail: `antigravity: ${config.binary} is not on PATH`, ready: false }
    const listed = await this.process
      .exec({ args: ['models'], command: config.binary, timeoutMs: MODELS_TIMEOUT_MS })
      .catch((error: unknown) => (error instanceof Error ? error.message : String(error)))
    return modelsReadiness(listed, config)
  }

  async review(launch: CriticLaunch, config: AgyClientConfig): Promise<CriticReport> {
    const launched = await this.gateway.launch({
      binary: config.binary,
      client: config.gateway?.client ?? 'agy',
      extras: agyExtras(launch),
      model: config.gateway?.model ?? config.model,
      paths: { exportDir: join(launch.criticDir, 'export'), promptPath: launch.promptPath, workDir: launch.packetDir },
      prompt: await this.files.readText(launch.promptPath),
      time_limit_s: launch.critic.time_limit_s,
    })
    const result = launched.result
    const run = readStream(result.stdout)
    const exportPath = join(launch.criticDir, 'result.json')
    if (run.result !== undefined) await this.files.writeText(exportPath, `${JSON.stringify(run.result, null, 2)}\n`)
    const outcome =
      result.timedOut === true
        ? timedOut(launch.critic.time_limit_s + OUTER_TIMEOUT_GRACE_S)
        : classifyAgyRun(result.exitCode, result.stdout, run, result.stderr)
    // Antigravity is quota, not money: it counts against the daily cap, never the Vibe cap.
    return { ...result, chargedUsd: 0, exportPath, outcome }
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
 * What `agy models` says about this machine. It prints one model per line,
 * the id, a tab and a label, after a progress line; signed out, it exits 1
 * with `Error: Please sign in to view available models. ...`. A string is the
 * error that kept it from starting.
 */
function modelsReadiness(listed: ProcessResult | string, config: AgyClientConfig): ClientReadiness {
  const failed = (reason: string): ClientReadiness => ({ detail: `antigravity: ${config.binary} models failed: ${reason}`, ready: false })
  if (typeof listed === 'string') return failed(listed)
  if (listed.timedOut === true) return failed(`no answer within ${String(MODELS_TIMEOUT_MS / 1000)}s`)
  const lines = `${listed.stdout}\n${listed.stderr}`
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && line !== MODELS_PROGRESS)
  if (listed.exitCode !== 0) {
    if (lines.some((line) => /sign in/i.test(line))) return { detail: SIGN_IN, ready: false }
    return failed(lines[0] ?? `exit ${String(listed.exitCode)}`)
  }
  if (lines.some((line) => line.split('\t')[0] === config.model)) return { ready: true }
  return { detail: `antigravity: ${config.binary} does not offer ${config.model}`, ready: false }
}

/**
 * What an agy critic asks the gateway for: the read-only sandbox profile, the
 * finding schema agy enforces on its structured output, and the tools its
 * prompt names. The gateway denies every tool family the critic may not use,
 * so a critic that may not search (ADR-0006: agy's search is unproven until
 * #178) has agy's own listing and search tools denied too.
 */
function agyExtras(launch: CriticLaunch): RunExtras {
  return { sandbox: 'read-only', schema: launch.schemaPath, tools: toolNames(launch.toolset ?? READ_ONLY_TOOLSET) }
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

/**
 * Why agy exited non-zero: the `short_error` of its `AGY_ERROR: {...}` line
 * (a spent quota reads `RESOURCE_EXHAUSTED (code 429): Individual quota
 * reached. ... Resets in <time>.`), else the first line of stderr.
 */
function exitReason(stderr: string): string | undefined {
  const lines = stderr.split('\n').map((line) => line.trim()).filter((line) => line.length > 0)
  for (const line of lines.filter((text) => text.startsWith(AGY_ERROR_PREFIX))) {
    try {
      const shortError = jsonObjectOrUndefined(JSON.parse(line.slice(AGY_ERROR_PREFIX.length)))?.short_error
      if (typeof shortError === 'string' && shortError.length > 0) return shortError
    } catch {
      // Not JSON: fall back to the first line.
    }
  }
  return lines[0]
}

const AGY_ERROR_PREFIX = 'AGY_ERROR:'

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
function classifyAgyRun(exitCode: number, stdout: string, run: AgyStream, stderr: string): CriticOutcome {
  const failed = (failure: CriticFailure, detail: string): CriticOutcome => ({ cost_usd: null, detail, failure, status: 'failed' })
  if (exitCode !== 0) {
    const reason = exitReason(stderr)
    return failed(`exit-${String(exitCode)}`, `agy exited ${String(exitCode)}${reason === undefined ? '' : `: ${reason}`}`)
  }
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
