import { isAbsolute, join } from 'node:path'

import type { EnvPort, ProcessCommand, ProcessPort } from '../../../../ports/index.js'
import type {
  CriticClientPort,
  CriticLaunch,
  CriticReport,
  CritiqueFilesPort,
  ModelAnswer,
  ModelAnswerReport,
  ModelQuestion,
  QuestionClientPort,
} from '../../application/ports.js'
import type { CriticFailure, CriticOutcome } from '../../domain/critic.js'
import { parseFindings } from '../../domain/findings.js'
import { cliVersion } from '../cli-version.js'
import { jsonObjectOrUndefined } from '../../domain/json.js'
import type { ClientReadiness, RosterSlot, VibeCriticClient as VibeClientConfig } from '../../domain/routing.js'

/**
 * The Vibe critic client: runs one Vibe critic in its packet and reads its
 * review from stdout and the export file. Everything Vibe-specific (the
 * launch, the isolation it needs, its exit codes, its journal and export
 * formats) stays here.
 */
export class VibeCriticClient implements CriticClientPort<VibeClientConfig>, QuestionClientPort<VibeClientConfig> {
  constructor(
    private readonly process: ProcessPort,
    private readonly files: CritiqueFilesPort,
    private readonly env: EnvPort,
  ) {}

  /**
   * Whether the critic can run on the personal home: `vibe` on `PATH`, the
   * home signed in the way setup checks it (Vibe's Keychain item, else a key
   * in `~/.vibe/.env`; `MISTRAL_API_KEY` does not count, since a critic runs
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

  /** `vibe` on `PATH` and the personal home signed in, whatever critic agents are installed. */
  async signIn(config: VibeClientConfig): Promise<ClientReadiness> {
    if (!(await this.onPath(config.binary))) return { detail: `vibe: ${config.binary} is not on PATH`, ready: false }
    return (await this.signedIn(join(this.env.require('HOME'), '.vibe'))) ? { ready: true } : { detail: SIGN_IN, ready: false }
  }

  async version(config: VibeClientConfig): Promise<string | null> {
    return cliVersion(this.process, config.binary)
  }

  private async signedIn(home: string): Promise<boolean> {
    const keychain = await this.process
      .exec({ args: ['find-generic-password', '-s', 'ai.mistral.vibe', '-a', 'MISTRAL_API_KEY'], command: 'security' })
      .catch(() => ({ exitCode: 1 }))
    if (keychain.exitCode === 0) return true
    const dotenv = join(home, '.env')
    if (!(await this.files.exists(dotenv))) return false
    return (await this.files.readText(dotenv))
      .split('\n')
      .some((line) => /^\s*MISTRAL_API_KEY\s*=\s*['"]?[^'"\s]/.test(line))
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
    const { result, run } = await this.launch(launch.critic, config, { exportDir, promptPath: launch.promptPath, workDir: launch.packetDir })
    return { ...result, chargedUsd: vibeCharge(run, launch.critic.max_price_usd), exportPath, outcome: classifyVibeRun(run) }
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
    return { ...result, answer, chargedUsd: vibeCharge(run, question.max_price_usd) }
  }

  private async launch(
    limits: VibeLimits,
    config: VibeClientConfig,
    paths: VibeLaunchPaths,
  ): Promise<{ readonly result: { exitCode: number; stdout: string; stderr: string }; readonly run: VibeRun }> {
    const result = await this.process.exec(vibeLaunch(limits, config, paths))
    const run: VibeRun = {
      exitCode: result.exitCode,
      exported: await this.readExport(join(paths.exportDir, 'export.json')),
      stderr: result.stderr,
      stdout: result.stdout,
      timedOut: result.timedOut === true,
      timeoutS: limits.time_limit_s + OUTER_TIMEOUT_GRACE_S,
    }
    return { result: { exitCode: result.exitCode, stderr: result.stderr, stdout: result.stdout }, run }
  }

  /** Parsed `export.json`, or undefined when Vibe wrote none or it does not parse. */
  private async readExport(path: string): Promise<unknown> {
    if (!(await this.files.exists(path))) return undefined
    try {
      return JSON.parse(await this.files.readText(path)) as unknown
    } catch {
      return undefined
    }
  }
}

/** Setup's own warning for a personal Vibe home that is not signed in. */
const SIGN_IN = 'vibe: not signed in; run vibe and sign in with your Mistral account, then re-run setup'

/** Seconds the outer timer allows beyond Vibe's own `--time-limit`. */
const OUTER_TIMEOUT_GRACE_S = 60

/**
 * Settings Vibe reads from `VIBE_*` variables. The environment layer sits
 * above the personal home's `config.toml` and below the critic agent file, so
 * these switch off the home's MCP fleet and its `AGENTS.md` pointer even if
 * the agent file forgot to.
 */
const ISOLATION_ENV: Readonly<Record<string, string>> = {
  VIBE_ENABLE_MCP: 'false',
  VIBE_ENABLE_SUBAGENTS: 'false',
  VIBE_INCLUDE_PROJECT_CONTEXT: 'false',
  VIBE_OUTSIDE_WORKDIR: 'deny',
}

/**
 * The plain `vibe` binary on the personal home: `MISTRAL_API_KEY` and
 * `VIBE_HOME` are removed, so a critique started from a `vibe-study` or
 * `vibe-private` session neither spends on the Study account nor runs in the
 * private home. The packet is the working directory and `read_file` the only
 * tool; with no `skill` tool the critic cannot load skills.
 */
interface VibeLaunchPaths {
  /** The working directory: a critic's packet copy, or a question's own directory. */
  readonly workDir: string
  readonly promptPath: string
  readonly exportDir: string
}

/** The price cap and time limit one launch runs under. */
type VibeLimits = Pick<RosterSlot, 'max_price_usd' | 'time_limit_s'>

function vibeLaunch(critic: VibeLimits, config: VibeClientConfig, paths: VibeLaunchPaths): ProcessCommand {
  return {
    args: [
      '-u',
      'MISTRAL_API_KEY',
      '-u',
      'VIBE_HOME',
      config.binary,
      '--prompt-file',
      paths.promptPath,
      '--output',
      'json',
      '--trust',
      '--agent',
      config.agent,
      '--enabled-tools',
      'read_file',
      '--max-price',
      String(critic.max_price_usd),
      '--time-limit',
      String(critic.time_limit_s),
      '--output-dir',
      paths.exportDir,
    ],
    command: 'env',
    cwd: paths.workDir,
    env: ISOLATION_ENV,
    timeoutMs: (critic.time_limit_s + OUTER_TIMEOUT_GRACE_S) * 1000,
  }
}

interface VibeRun {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
  /** Parsed `export.json`, or undefined when Vibe wrote none or it does not parse. */
  readonly exported: unknown
  /** The outer timer killed Vibe after `timeoutS` seconds. */
  readonly timedOut: boolean
  readonly timeoutS: number
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
  const cost = typeof exported?.cost_usd === 'number' ? exported.cost_usd : null
  const failed = (failure: CriticFailure, detail: string): VibeMessage => ({ cost_usd: cost, detail, failure, status: 'failed' })

  if (run.timedOut) return failed('timeout', `vibe did not return within ${String(run.timeoutS)}s, so the outer timer killed it`)
  if (run.exitCode !== 0) {
    const outcome = typeof exported?.outcome === 'string' ? `outcome ${exported.outcome}` : run.stderr.trim()
    return failed(EXIT_FAILURES[run.exitCode] ?? `exit-${String(run.exitCode)}`, `vibe exited ${String(run.exitCode)}: ${outcome}`)
  }
  if (run.stdout.trim().length === 0) return failed('empty-output', 'vibe exited 0 with empty stdout')
  if (exported === undefined) return failed('missing-export', 'vibe wrote no readable export.json')
  const violations = isolationViolations(jsonObjectOrUndefined(exported.config))
  if (violations.length > 0) return failed('unsafe-config', `effective config: ${violations.join('; ')}`)
  const message = finalAssistantText(run.stdout)
  if (message === undefined) return failed('unparseable-output', 'stdout is not a Vibe journal with an assistant message')
  return { cost_usd: cost, status: 'ok', text: message }
}

/**
 * What a Vibe run counts against the flow's cap. The export's `cost_usd` is
 * the base, and it may exceed the critic's cap: `--max-price` is checked only
 * between turns. Where the export cannot be trusted the cap is charged
 * instead, as a conservative estimate: no export or no cost (a killed run
 * included), a deadline stop (Vibe drops the interrupted turn's tokens and
 * reports $0), and a zero cost for tokens used (a model with no prices).
 */
function vibeCharge(run: VibeRun, cap: number): number {
  const exported = jsonObjectOrUndefined(run.exported)
  const cost = exported?.cost_usd
  if (run.timedOut || typeof cost !== 'number') return cap
  if (exported?.outcome === 'deadline') return Math.max(cost, cap)
  return cost === 0 && usedTokens(jsonObjectOrUndefined(exported?.usage)) ? cap : cost
}

function usedTokens(usage: Readonly<Record<string, unknown>> | undefined): boolean {
  return [usage?.input_tokens, usage?.output_tokens].some((tokens) => typeof tokens === 'number' && tokens > 0)
}

function isolationViolations(config: Readonly<Record<string, unknown>> | undefined): readonly string[] {
  const violations: string[] = []
  if (config?.enable_mcp !== false) violations.push('enable_mcp is not false')
  const tools = config?.enabled_tools
  if (!Array.isArray(tools) || tools.length !== 1 || tools[0] !== 'read_file') {
    violations.push('enabled_tools is not exactly read_file')
  }
  if (config?.include_project_context !== false) {
    violations.push('include_project_context is not false, so the AGENTS.md instructions pointer reaches the critic')
  }
  if (config?.outside_workdir !== 'deny') violations.push('outside_workdir is not deny')
  return violations
}

function finalAssistantText(stdout: string): string | undefined {
  let journal: unknown
  try {
    journal = JSON.parse(stdout)
  } catch {
    return undefined
  }
  const entries = Array.isArray(journal) ? journal.map(jsonObjectOrUndefined) : []
  const last = entries.findLast((entry) => entry?.type === 'message' && entry.role === 'assistant')
  const content = Array.isArray(last?.content) ? last.content.map(jsonObjectOrUndefined) : undefined
  return content?.map((part) => (typeof part?.text === 'string' ? part.text : '')).join('')
}
