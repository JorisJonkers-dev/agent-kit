import { join } from 'node:path'

import type { ProcessCommand, ProcessPort } from '../../../../ports/index.js'
import type { CriticClientPort, CriticLaunch, CriticReport, CritiqueFilesPort } from '../../application/ports.js'
import type { CriticFailure, CriticOutcome } from '../../domain/critic.js'
import { parseFindings } from '../../domain/findings.js'
import { jsonObjectOrUndefined } from '../../domain/json.js'
import type { RosterCritic } from '../../domain/routing.js'

/**
 * The Vibe critic client: runs one Vibe critic in its packet and reads its
 * review from stdout and the export file. Everything Vibe-specific (the
 * launch, the isolation it needs, its exit codes, its journal and export
 * formats) stays here.
 */
export class VibeCriticClient implements CriticClientPort {
  constructor(
    private readonly process: ProcessPort,
    private readonly files: CritiqueFilesPort,
  ) {}

  async review(launch: CriticLaunch): Promise<CriticReport> {
    const exportDir = join(launch.criticDir, 'export')
    const exportPath = join(exportDir, 'export.json')
    const result = await this.process.exec(
      vibeLaunch(launch.critic, { exportDir, packetDir: launch.packetDir, promptPath: launch.promptPath }),
    )
    const outcome = classifyVibeRun({
      exitCode: result.exitCode,
      exported: await this.readExport(exportPath),
      stderr: result.stderr,
      stdout: result.stdout,
    })
    return { ...result, exportPath, outcome }
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
  readonly packetDir: string
  readonly promptPath: string
  readonly exportDir: string
}

function vibeLaunch(critic: RosterCritic, paths: VibeLaunchPaths): ProcessCommand {
  return {
    args: [
      '-u',
      'MISTRAL_API_KEY',
      '-u',
      'VIBE_HOME',
      critic.client_config.binary,
      '--prompt-file',
      paths.promptPath,
      '--output',
      'json',
      '--trust',
      '--agent',
      critic.client_config.agent,
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
    cwd: paths.packetDir,
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
}

const EXIT_FAILURES: Readonly<Record<number, CriticFailure>> = {
  1: 'config-error',
  2: 'infrastructure-failure',
  3: 'stopped-early',
}

/**
 * Exit code first: anything but 0 is a failed critic, whatever stdout holds.
 * On 0, the export must show an isolated config, and stdout must end in an
 * assistant message whose last ```json block matches the output contract.
 */
function classifyVibeRun(run: VibeRun): CriticOutcome {
  const exported = jsonObjectOrUndefined(run.exported)
  const cost = typeof exported?.cost_usd === 'number' ? exported.cost_usd : null
  const failed = (failure: CriticFailure, detail: string): CriticOutcome => ({ cost_usd: cost, detail, failure, status: 'failed' })

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
  const parsed = parseFindings(message)
  if (!parsed.ok) return failed(parsed.failure, parsed.detail)
  return {
    cost_usd: cost,
    findings: parsed.findings,
    status: 'ok',
    ...(parsed.no_findings_reason === undefined ? {} : { no_findings_reason: parsed.no_findings_reason }),
  }
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
