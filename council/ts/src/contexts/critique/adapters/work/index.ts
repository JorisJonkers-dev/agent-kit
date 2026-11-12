import { isAbsolute, join } from 'node:path'

import { ClientRun } from '../../../model-gateway/index.js'
import type { ClientRunOutcome } from '../../../model-gateway/index.js'
import type { EnvPort, ProcessPort, ProcessResult } from '../../../../ports/index.js'
import type { CriticClientPort, CriticLaunch, CriticReport, CritiqueFilesPort } from '../../application/ports.js'
import type { CriticFailure, CriticOutcome } from '../../domain/critic.js'
import { parseFindings } from '../../domain/findings.js'
import type { ClaudeCriticClient as ClaudeClientConfig, ClientReadiness, CodexCriticClient as CodexClientConfig } from '../../domain/routing.js'
import { cliVersion } from '../cli-version.js'

/**
 * The work critic clients: Codex on the nedap account and the work Claude
 * agent, which only a `nedap` repository's work roster names (ADR-0008,
 * ADR-0011). Both launch through the model gateway's launch path, read-only,
 * in the critic's own copy of the packet. Each reads its sign-in from the
 * CLI's own status command, which calls no model and so spends nothing.
 */

/** The longest a status command may take; signed in it answers at once. */
const STATUS_TIMEOUT_MS = 20_000

/** What a status command's answer says: signed in, or why the client cannot run. */
type Status = { readonly signedIn: true } | { readonly signedIn: false; readonly reason: string }

/** Setup's wording for a client that is not on `PATH`. */
function missing(client: string, binary: string): ClientReadiness {
  return { detail: `${client}: ${binary} is not on PATH`, ready: false }
}

/** Whether a binary is installed: an absolute path that exists, or a file of `PATH`. */
async function onPath(files: Pick<CritiqueFilesPort, 'exists'>, env: EnvPort, binary: string): Promise<boolean> {
  if (isAbsolute(binary)) return files.exists(binary)
  for (const dir of (env.get('PATH') ?? '').split(':').filter((entry) => entry.length > 0)) {
    if (await files.exists(join(dir, binary))) return true
  }
  return false
}

/** Runs a status command; the error that kept it from running, else its result. */
async function runStatus(process: ProcessPort, binary: string, args: readonly string[], env?: Readonly<Record<string, string>>): Promise<ProcessResult | string> {
  return process
    .exec({ args, command: binary, ...(env === undefined ? {} : { env }), timeoutMs: STATUS_TIMEOUT_MS })
    .catch((error: unknown) => (error instanceof Error ? error.message : String(error)))
}

/** The first non-blank line of what a command printed. */
function firstLine(result: ProcessResult): string | undefined {
  return `${result.stdout}\n${result.stderr}`
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line.length > 0)
}

/** Why a status command could not say, as one clause. */
function unreadable(result: ProcessResult | string): string {
  if (typeof result === 'string') return result
  if (result.timedOut === true) return `no answer within ${String(STATUS_TIMEOUT_MS / 1000)}s`
  return firstLine(result) ?? `exit ${String(result.exitCode)}`
}

/** Seconds are the client's own limit; none of these clients has one besides the outer timer. */
function limitOf(launch: CriticLaunch): number {
  return launch.critic.time_limit_s
}

/**
 * What a work critic's run amounts to. The exit code and the client's own
 * failure come first, then the final message must hold the findings block.
 * The work critics are paid by the work accounts, so a run charges nothing
 * against the flow's Vibe cap.
 */
function classify(client: string, launch: CriticLaunch, run: ClientRunOutcome): CriticOutcome {
  const { output, result } = run
  const cost = output.charged ?? null
  const failed = (failure: CriticFailure, detail: string): CriticOutcome => ({ cost_usd: cost, detail, failure, status: 'failed' })
  if (result.timedOut === true) return failed('timeout', `${client} did not return within ${String(limitOf(launch))}s, so the outer timer killed it`)
  if (result.exitCode !== 0) {
    const said = firstLine(result)
    return failed(`exit-${String(result.exitCode)}`, `${client} exited ${String(result.exitCode)}${said === undefined ? '' : `: ${said}`}`)
  }
  if (result.stdout.trim().length === 0) return failed('empty-output', `${client} exited 0 with empty stdout`)
  if (output.failure !== undefined) {
    return failed('infrastructure-failure', `${client} reported a failed run${output.error === undefined ? '' : `: ${output.error.text}`}`)
  }
  if (output.answer === undefined) return failed('unparseable-output', `stdout is not ${client}'s output with a final message`)
  const parsed = parseFindings(output.answer)
  if (!parsed.ok) return failed(parsed.failure, parsed.detail)
  return {
    cost_usd: cost,
    findings: parsed.findings,
    status: 'ok',
    ...(parsed.no_findings_reason === undefined ? {} : { no_findings_reason: parsed.no_findings_reason }),
  }
}

/** The report of one launch: the process's own output, what the run used, and the outcome. */
async function report(files: CritiqueFilesPort, client: string, launch: CriticLaunch, run: ClientRunOutcome): Promise<CriticReport> {
  const exportPath = join(launch.criticDir, 'result.json')
  const { output, result } = run
  await files.writeText(
    exportPath,
    `${JSON.stringify({ answer: output.answer ?? null, charged_usd: output.charged ?? null, tokens: output.tokens }, null, 2)}\n`,
  )
  return {
    ...result,
    chargedUsd: 0,
    exportPath,
    outcome: classify(client, launch, run),
    tokens: output.tokens,
  }
}

/**
 * The Codex critic client: `codex exec` in its read-only sandbox, in the
 * critic's own copy of the packet. Signed in is `codex login status`, which
 * exits 0 only while it holds the work account's login.
 */
export class CodexCriticClient implements CriticClientPort<CodexClientConfig> {
  private readonly gateway: ClientRun

  constructor(
    private readonly process: ProcessPort,
    private readonly files: CritiqueFilesPort,
    private readonly env: EnvPort,
  ) {
    this.gateway = new ClientRun({ env, files, process })
  }

  /** Readiness is the CLI being installed and signed in: a Codex critic needs no file of its own. */
  async readiness(config: CodexClientConfig): Promise<ClientReadiness> {
    return this.signIn(config)
  }

  async signIn(config: CodexClientConfig): Promise<ClientReadiness> {
    if (!(await onPath(this.files, this.env, config.binary))) return missing('codex', config.binary)
    const status = await this.status(config)
    return status.signedIn ? { ready: true } : { detail: status.reason, ready: false }
  }

  async version(config: CodexClientConfig): Promise<string | null> {
    return cliVersion(this.process, config.binary)
  }

  async review(launch: CriticLaunch, config: CodexClientConfig): Promise<CriticReport> {
    const run = await this.gateway.launch({
      binary: config.binary,
      client: 'codex',
      extras: {},
      mode: 'read-only',
      model: config.gateway?.model ?? config.model,
      paths: { exportDir: join(launch.criticDir, 'export'), promptPath: launch.promptPath, workDir: launch.packetDir },
      prompt: await this.files.readText(launch.promptPath),
      time_limit_s: limitOf(launch),
    })
    return report(this.files, 'codex', launch, run)
  }

  private async status(config: CodexClientConfig): Promise<Status> {
    const result = await runStatus(this.process, config.binary, ['login', 'status'])
    if (typeof result !== 'string' && result.timedOut !== true && result.exitCode === 0) return { signedIn: true }
    return { reason: `codex: not signed in; run codex login with the work account, then re-run setup (${unreadable(result)})`, signedIn: false }
  }
}

/**
 * The work Claude agent critic client: Claude Code in plan mode with the read
 * tools alone, in the critic's own copy of the packet. It always runs in the
 * work configuration directory, `~/.claude`, whatever `CLAUDE_CONFIG_DIR` the
 * environment holds, so the identity cannot switch by accident. Signed in is
 * `claude auth status` for that directory.
 */
export class ClaudeCriticClient implements CriticClientPort<ClaudeClientConfig> {
  private readonly gateway: ClientRun

  constructor(
    private readonly process: ProcessPort,
    private readonly files: CritiqueFilesPort,
    private readonly env: EnvPort,
  ) {
    this.gateway = new ClientRun({ env, files, process })
  }

  async readiness(config: ClaudeClientConfig): Promise<ClientReadiness> {
    return this.signIn(config)
  }

  async signIn(config: ClaudeClientConfig): Promise<ClientReadiness> {
    if (!(await onPath(this.files, this.env, config.binary))) return missing('claude', config.binary)
    const result = await runStatus(this.process, config.binary, ['auth', 'status'], this.profile())
    if (typeof result !== 'string' && result.timedOut !== true && result.exitCode === 0) return { ready: true }
    return {
      detail: `claude: not signed in to the work profile; run claude and sign in with the work account, then re-run setup (${unreadable(result)})`,
      ready: false,
    }
  }

  async version(config: ClaudeClientConfig): Promise<string | null> {
    return cliVersion(this.process, config.binary)
  }

  async review(launch: CriticLaunch, config: ClaudeClientConfig): Promise<CriticReport> {
    const run = await this.gateway.launch({
      binary: config.binary,
      client: 'claude',
      extras: { env: this.profile() },
      mode: 'read-only',
      model: config.gateway?.model ?? config.model,
      paths: { exportDir: join(launch.criticDir, 'export'), promptPath: launch.promptPath, workDir: launch.packetDir },
      prompt: await this.files.readText(launch.promptPath),
      time_limit_s: limitOf(launch),
    })
    return report(this.files, 'claude', launch, run)
  }

  /** The work profile's directory, set explicitly so an ambient `CLAUDE_CONFIG_DIR` cannot pick another. */
  private profile(): Readonly<Record<string, string>> {
    return { CLAUDE_CONFIG_DIR: join(this.env.require('HOME'), '.claude') }
  }
}
