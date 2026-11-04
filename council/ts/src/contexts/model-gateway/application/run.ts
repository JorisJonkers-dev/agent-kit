import { randomBytes } from 'node:crypto'
import { join } from 'node:path'

import type { ProcessCommand, ProcessResult } from '../../../ports/index.js'
import { ModelGatewayError, type GatewayConfig, type ModelEntry } from '../domain/config.js'
import type { LedgerRecord } from '../domain/ledger.js'
import { familyOf, modelKey } from '../domain/models.js'
import { decidePolicy, type ModelPermission } from '../domain/policy.js'
import {
  finalAnswerOf,
  readOnlyProof,
  readRunInput,
  runCost,
  runFailureReason,
  runId,
  runTokens,
  summaryOf,
  wrappedPrompt,
  type ProofFiles,
  type RunInput,
  type RunMode,
  type RunTokens,
} from '../domain/run.js'
import { loadConfig } from './config.js'
import { messageOf } from './json-file.js'
import { overridePath, runsDir } from './locations.js'
import { resolveScope } from './policy.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

/** The mark the gateway puts in every delegate's environment, and refuses to run under. */
export const DELEGATION_MARK = 'AGENT_KIT_DELEGATION'

/** The Vibe agent a read-only delegation runs through: read tools only, no subagents, nothing outside its workdir. */
export const READ_ONLY_AGENT = 'delegate-readonly'

/** Seconds the outer timer allows beyond Vibe's own `--time-limit`, so Vibe's own stop is the one that reports. */
const OUTER_TIMEOUT_GRACE_S = 60

/**
 * What a delegate's environment holds: the mark that stops it delegating on,
 * and the isolation Vibe reads from `VIBE_*` variables, which sits above the
 * home's `config.toml` and below the agent file.
 */
const DELEGATE_ENV: Readonly<Record<string, string>> = {
  [DELEGATION_MARK]: '1',
  VIBE_ENABLE_MCP: 'false',
  VIBE_ENABLE_SUBAGENTS: 'false',
  VIBE_INCLUDE_PROJECT_CONTEXT: 'false',
  VIBE_OUTSIDE_WORKDIR: 'deny',
}

/** What one delegation came to: a short result, with the full output left in its run directory. */
export interface RunResult {
  readonly status: 'ok' | 'failed'
  readonly run_id: string
  readonly workflow: string
  readonly client: string
  readonly model: string
  readonly family: string
  readonly mode: RunMode
  readonly tokens: RunTokens
  readonly cost_usd: number
  /** The file the delegate's full output was written to. */
  readonly output_path: string
  /** The delegate's own summary, cut at `summary_max_chars`. */
  readonly summary: string
  /** Why a failed run failed: `timeout`, `parse-error`, `exit-<n>` or `launch-failed`. */
  readonly reason?: string
}

/** Where one launch reads its prompt and writes what Vibe exports. */
interface LaunchPaths {
  readonly exportDir: string
  readonly promptPath: string
  readonly workDir: string
}

/** What the data policy allows this run, and the repository it is keyed by. */
interface RunPermission {
  readonly permission: ModelPermission
  readonly repository: string | null
}

/**
 * Hands one read-only task to one client model and reports what came of it.
 *
 * Everything that can refuse the run happens before anything is launched: the
 * nesting guard, the data policy for the repository, and the read-only proof
 * of the client model. A run that is launched always leaves a run directory
 * and a ledger line, whether it succeeded or not.
 */
export class RunCommand implements ModelGatewayCommand<unknown, RunResult> {
  constructor(private readonly deps: ModelGatewayDependencies) {}

  async run(raw: unknown): Promise<RunResult> {
    const input = readRunInput(raw)
    this.refuseNested()
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const client = config.clients[input.client]
    if (client === undefined) throw new ModelGatewayError('unknown-model', `the gateway knows no client ${input.client}`)
    const entry = catalogueEntry(config, input.client, input.model)
    const family = entry?.family ?? familyOf(input.model)
    if (family === undefined) {
      throw new ModelGatewayError(
        'unknown-model',
        `${input.client}:${input.model} is not a model the gateway can place: add it to the catalogue in defaults.json or the machine override`,
      )
    }
    const root = input.repo === undefined ? undefined : await this.repositoryRoot(input.repo)
    const allowed = await this.permission(input, config, family)
    await this.proveReadOnly(input)
    const startedAt = this.deps.clock.now()
    const startedMs = this.deps.clock.monotonicMs()
    const id = runId(startedAt, randomBytes(3).toString('hex'))
    const workDir = root ?? (await this.deps.files.tempDir('model-gateway-run-'))
    const runDir = await this.runDir(input, workDir, id)
    const paths: LaunchPaths = { exportDir: join(runDir, 'vibe'), promptPath: join(runDir, 'prompt.md'), workDir }
    await this.deps.files.writeText(paths.promptPath, wrappedPrompt(input.prompt, config.settings.summary_max_chars))
    await this.deps.files.writeText(join(runDir, 'input.json'), `${JSON.stringify(input, null, 2)}\n`)
    const launched = await this.launch(client.binary, input, paths)
    const exported = await this.readExport(join(paths.exportDir, 'export.json'))
    const answer = finalAnswerOf(launched.result.stdout)
    const reason = launched.failure ?? runFailureReason(launched.result, answer)
    const tokens = runTokens(exported)
    const cost = runCost(exported, tokens, entry?.prices)
    const outputPath = join(runDir, 'output.json')
    await this.deps.files.writeText(outputPath, launched.result.stdout)
    const result: RunResult = {
      client: input.client,
      cost_usd: cost,
      family,
      mode: input.mode,
      model: input.model,
      output_path: outputPath,
      run_id: id,
      status: reason === undefined ? 'ok' : 'failed',
      summary: answer === undefined ? '' : summaryOf(answer, config.settings.summary_max_chars),
      tokens,
      workflow: input.workflow,
      ...(reason === undefined ? {} : { reason }),
    }
    await this.deps.files.writeText(join(runDir, 'result.json'), `${JSON.stringify(result, null, 2)}\n`)
    await this.deps.ledger.append(this.ledgerRecord(result, allowed.repository, startedMs))
    return result
  }

  /** A delegate may not delegate on: the mark is in its environment, so this call came from inside one. */
  private refuseNested(): void {
    if (this.deps.env.get(DELEGATION_MARK) !== undefined) {
      throw new ModelGatewayError('nested-delegation', `a delegate may not delegate on: ${DELEGATION_MARK} is set in this environment`)
    }
  }

  /**
   * The data policy's permission for this client model, or a refusal naming
   * the rule. A run with a repository reads it, so it carries code whatever
   * the caller marks; an inline run carries only what the caller declares.
   */
  private async permission(input: RunInput, config: GatewayConfig, family: string): Promise<RunPermission> {
    const scope = await resolveScope(this.deps, input.repo)
    const decision = decidePolicy(config, scope, [{ client: input.client, family, model: input.model }])
    const permission = decision.permissions[0]
    if (!permission?.permitted) {
      throw new ModelGatewayError('policy-refused', `${input.client}:${input.model} is not permitted: ${decision.reason}`)
    }
    const carries = input.carries ?? { code: false, history: false }
    const carriesCode = input.repo === undefined ? carries.code : true
    if (carriesCode && !permission.may_send.code) {
      throw new ModelGatewayError('policy-refused', `${input.client}:${input.model} may not carry code: ${decision.reason}`)
    }
    if (carries.history && !permission.may_send.history) {
      throw new ModelGatewayError('policy-refused', `${input.client}:${input.model} may not carry history: ${decision.reason}`)
    }
    return { permission, repository: scope.kind === 'repository' ? scope.key : null }
  }

  /** A read-only run goes in place only on a client model whose read-only proof passed on this machine. */
  private async proveReadOnly(input: RunInput): Promise<void> {
    const proof = readOnlyProof(await this.proofFiles(), input.model)
    if (proof.status === 'unproven') {
      throw new ModelGatewayError('unproven-read-only', `${input.client}:${input.model} has no passing read-only proof: ${proof.reason}`)
    }
  }

  private async proofFiles(): Promise<ProofFiles> {
    return this.deps.proof.read()
  }

  /**
   * The repository's top level, which is where the delegate works. A path git
   * cannot read, or one in no repository at all, is an error: a run that names
   * a repository never falls back to an inline one.
   */
  private async repositoryRoot(repo: string): Promise<string> {
    try {
      const root = await this.deps.git.root(repo)
      if (root === undefined) throw new Error('not a git repository')
      return root
    } catch (error) {
      throw new ModelGatewayError('repository-unreadable', `${repo}: ${messageOf(error)}`)
    }
  }

  /**
   * Where the run keeps its prompt, its output and its result: in the
   * repository's common directory when it has one, so a worktree's run is
   * found with the repository's other runs, and in this machine's state
   * directory otherwise.
   */
  private async runDir(input: RunInput, workDir: string, id: string): Promise<string> {
    if (input.repo === undefined) return join(runsDir(this.deps.env), id)
    let commonDir: string
    try {
      commonDir = await this.deps.git.commonDir(workDir)
    } catch (error) {
      throw new ModelGatewayError('repository-unreadable', `${workDir}: ${messageOf(error)}`)
    }
    return join(commonDir, 'model-gateway', 'runs', id)
  }

  /**
   * Vibe, non-interactively: the wrapped prompt from a file, the client model
   * by the alias `models` lists, the read-only agent, and Vibe's own time and
   * price limits. A launch that cannot start at all is a failed run, not an
   * error: the caller still gets a result and a ledger line.
   */
  private async launch(binary: string, input: RunInput, paths: LaunchPaths): Promise<{ result: ProcessResult; failure?: string }> {
    const command: ProcessCommand = {
      args: [
        '--prompt-file',
        paths.promptPath,
        '--model',
        input.model,
        '--agent',
        READ_ONLY_AGENT,
        '--trust',
        '--workdir',
        paths.workDir,
        '--output',
        'json',
        '--time-limit',
        String(input.time_limit_s),
        ...(input.max_price_usd === undefined ? [] : ['--max-price', String(input.max_price_usd)]),
        '--output-dir',
        paths.exportDir,
      ],
      command: binary,
      cwd: paths.workDir,
      env: DELEGATE_ENV,
      timeoutMs: (input.time_limit_s + OUTER_TIMEOUT_GRACE_S) * 1000,
    }
    try {
      return { result: await this.deps.process.exec(command) }
    } catch (error) {
      return { failure: 'launch-failed', result: { exitCode: 1, stderr: messageOf(error), stdout: '', timedOut: false } }
    }
  }

  /** Parsed `export.json`, or undefined when Vibe wrote none or it does not parse. */
  private async readExport(path: string): Promise<unknown> {
    if (!(await this.deps.files.exists(path))) return undefined
    try {
      return JSON.parse(await this.deps.files.readText(path)) as unknown
    } catch {
      return undefined
    }
  }

  /** The run's one ledger line: what it was, what it cost and how it ended. */
  private ledgerRecord(result: RunResult, repository: string | null, startedMs: number): LedgerRecord {
    return {
      at: this.deps.clock.now().toISOString(),
      client: result.client,
      cost_usd: result.cost_usd,
      duration_s: Math.round(this.deps.clock.monotonicMs() - startedMs) / 1000,
      family: result.family,
      mode: result.mode,
      model: result.model,
      outcome: result.status,
      repository,
      run_id: result.run_id,
      tokens: result.tokens,
      workflow: result.workflow,
      ...(result.reason === undefined ? {} : { reason: result.reason }),
    }
  }
}

/** The catalogue entry for a client model, or undefined when the catalogue does not know it; a disabled model is refused. */
function catalogueEntry(config: GatewayConfig, client: string, model: string): ModelEntry | undefined {
  if (config.disabled.includes(modelKey(client, model))) {
    throw new ModelGatewayError('unknown-model', `${client}:${model} is disabled in this machine's configuration`)
  }
  return config.models.find((candidate) => candidate.client === client && candidate.model === model)
}
