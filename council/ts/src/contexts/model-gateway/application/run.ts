import { randomBytes } from 'node:crypto'
import { join } from 'node:path'

import { ModelGatewayError, type GatewayConfig, type ModelEntry } from '../domain/config.js'
import type { LedgerRecord } from '../domain/ledger.js'
import { familyOf, modelKey, modelPart } from '../domain/models.js'
import { decidePolicy, type ModelPermission } from '../domain/policy.js'
import {
  NO_TOKENS,
  readOnlyProof,
  readRunInput,
  requireAllowedProvider,
  priceRun,
  runFailureReason,
  runId,
  summaryOf,
  wrappedPrompt,
  type ProofFiles,
  type RunClient,
  type RunInput,
  type RunMode,
  type RunTokens,
} from '../domain/run.js'
import { planWindows, warningsOf } from '../domain/usage.js'
import { ClientRun, type ClientRunOutcome } from './client-run.js'
import { loadConfig } from './config.js'
import { messageOf } from './json-file.js'
import { overridePath, runsDir } from './locations.js'
import { DELEGATION_MARK, READ_ONLY_AGENT, type LaunchPaths } from './launchers.js'
import { resolveScope } from './policy.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

export { DELEGATION_MARK, READ_ONLY_AGENT }

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
  /**
   * Why a failed run failed: `timeout`, `parse-error`, `exit-<n>`,
   * `rate-limited`, `auth`, `launch-failed`, `denied-action` or `client-error`.
   */
  readonly reason?: string
  /** Set when the run's throwaway worktree could not be removed: what git said, and which directory is left behind. */
  readonly cleanup_warning?: string
  /** One line per plan window of this client at or above the warning ratio. */
  readonly warnings?: readonly string[]
}

/**
 * Where a run works, and how it leaves: in the repository itself when the
 * client model's read-only proof passed, else in a throwaway directory that
 * `release` deletes.
 */
interface Workplace {
  readonly dir: string
  release(): Promise<void>
}

/** Everything one launched run needs beyond its input, settled before anything starts. */
interface Delegation {
  readonly binary: string
  readonly config: GatewayConfig
  readonly entry: ModelEntry | undefined
  readonly family: string
  readonly id: string
  readonly input: RunInput
  readonly runDir: string
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
  private readonly clientRun: ClientRun

  constructor(private readonly deps: ModelGatewayDependencies) {
    this.clientRun = new ClientRun(deps)
  }

  async run(raw: unknown): Promise<RunResult> {
    const input = readRunInput(raw)
    this.refuseNested()
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const client = config.clients[input.client]
    if (client === undefined) throw new ModelGatewayError('unknown-model', `the gateway knows no client ${input.client}`)
    if (input.client === 'opencode') requireAllowedProvider(input.model, client.providers)
    const entry = catalogueEntry(config, input.client, input.model)
    const family = entry?.family ?? familyOf(modelPart(input.model))
    if (family === undefined) {
      throw new ModelGatewayError(
        'unknown-model',
        `${input.client}:${input.model} is not a model the gateway can place: add it to the catalogue in defaults.json or the machine override`,
      )
    }
    const root = input.repo === undefined ? undefined : await this.repositoryRoot(input.repo)
    const allowed = await this.permission(input, config, family)
    const throwaway = await this.needsThrowaway(input, entry)
    const startedAt = this.deps.clock.now()
    const startedMs = this.deps.clock.monotonicMs()
    const id = runId(startedAt, randomBytes(3).toString('hex'))
    const runDir = await this.runDir(root, id)
    const workplace = await this.workplace(root, throwaway)
    let result: RunResult
    try {
      result = await this.delegate({ binary: client.binary, config, entry, family, id, input, runDir, workDir: workplace.dir })
    } catch (error) {
      await this.release(workplace)
      throw error
    }
    const warning = await this.release(workplace)
    const released: RunResult = warning === undefined ? result : { ...result, cleanup_warning: warning }
    await this.deps.ledger.append(this.ledgerRecord(released, allowed.repository, startedMs))
    const warnings = warningsOf(
      input.client,
      planWindows(input.client, client.plan_limits ?? [], await this.deps.ledger.read(), this.deps.clock.now(), config.settings.plan_warning_ratio),
    )
    const recorded: RunResult = warnings.length === 0 ? released : { ...released, warnings }
    await this.deps.files.writeText(join(runDir, 'result.json'), `${JSON.stringify(recorded, null, 2)}\n`)
    return recorded
  }

  /**
   * Deletes the run's throwaway directory. A cleanup that fails never fails
   * the run: it is reported as a warning the result carries, and a run that
   * threw keeps its own error.
   */
  private async release(workplace: Workplace): Promise<string | undefined> {
    try {
      await workplace.release()
      return undefined
    } catch (error) {
      return `cannot remove the throwaway worktree ${workplace.dir}: ${messageOf(error)}`
    }
  }

  /** One launched run, up to the point where its result is known: its prompt on disk, the launch and the client's full output. */
  private async delegate(run: Delegation): Promise<RunResult> {
    const { config, entry, input, runDir } = run
    const paths: LaunchPaths = { exportDir: join(runDir, 'vibe'), promptPath: join(runDir, 'prompt.md'), workDir: run.workDir }
    const prompt = wrappedPrompt(input.prompt, config.settings.summary_max_chars)
    await this.deps.files.writeText(paths.promptPath, prompt)
    await this.deps.files.writeText(join(runDir, 'input.json'), `${JSON.stringify(input, null, 2)}\n`)
    const launched = await this.launch(run.binary, input, paths, prompt)
    const output = launched.output
    const reason = launched.failure ?? runFailureReason(launched.result, output.answer, output)
    const cost = priceRun(output.charged, output.tokens, entry?.prices)
    const outputPath = join(runDir, 'output.json')
    await this.deps.files.writeText(outputPath, launched.result.stdout)
    const result: RunResult = {
      client: input.client,
      cost_usd: cost,
      family: run.family,
      mode: input.mode,
      model: input.model,
      output_path: outputPath,
      run_id: run.id,
      status: reason === undefined ? 'ok' : 'failed',
      summary: output.answer === undefined ? '' : summaryOf(output.answer, config.settings.summary_max_chars),
      tokens: output.tokens,
      workflow: input.workflow,
      ...(reason === undefined ? {} : { reason }),
    }
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

  /**
   * Whether the run must work in a throwaway directory: a client model goes in
   * place only when its read-only proof passed on this machine, whatever the
   * client. The proof is recorded against the model id critique's routing
   * names, which the catalogue keeps among the model's `names` beside the
   * alias the run names. Every other model runs in a throwaway worktree.
   */
  private async needsThrowaway(input: RunInput, entry: ModelEntry | undefined): Promise<boolean> {
    const proof = readOnlyProof(await this.proofFiles(), input.model, entry?.names ?? [], input.client)
    return proof.status !== 'proven'
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
  private async runDir(root: string | undefined, id: string): Promise<string> {
    if (root === undefined) return join(runsDir(this.deps.env), id)
    let commonDir: string
    try {
      commonDir = await this.deps.git.commonDir(root)
    } catch (error) {
      throw new ModelGatewayError('repository-unreadable', `${root}: ${messageOf(error)}`)
    }
    return join(commonDir, 'model-gateway', 'runs', id)
  }

  /**
   * The directory the client works in. A run with a passing proof works in the
   * repository itself, or in a fresh empty directory when it has none (which
   * stays, as before). A throwaway run works in a worktree of the repository's
   * HEAD, or in a fresh empty directory when it has none, and `release`
   * deletes it whatever the run came to.
   */
  private async workplace(root: string | undefined, throwaway: boolean): Promise<Workplace> {
    if (!throwaway) {
      const dir = root ?? (await this.deps.files.tempDir('model-gateway-run-'))
      return { dir, release: () => Promise.resolve() }
    }
    const dir = await this.deps.files.tempDir('model-gateway-worktree-')
    if (root === undefined) return { dir, release: () => this.deps.files.remove(dir) }
    try {
      await this.deps.git.addWorktree(root, dir)
    } catch (error) {
      await this.deps.files.remove(dir)
      throw new ModelGatewayError('worktree-failed', `${root}: cannot add a throwaway worktree: ${messageOf(error)}`)
    }
    return {
      dir,
      release: () => this.deps.git.removeWorktree(root, dir),
    }
  }

  /**
   * The client, non-interactively, through the gateway's launch path, with the
   * mark that stops a delegate delegating on. A launch that cannot start at
   * all is a failed run, not an error: the caller still gets a result and a
   * ledger line.
   */
  private async launch(binary: string, input: RunInput, paths: LaunchPaths, prompt: string): Promise<ClientRunOutcome & { failure?: string }> {
    try {
      return await this.clientRun.launch({
        binary,
        client: input.client as RunClient,
        extras: { env: { [DELEGATION_MARK]: '1' } },
        model: input.model,
        paths,
        prompt,
        time_limit_s: input.time_limit_s,
        ...(input.max_price_usd === undefined ? {} : { max_price_usd: input.max_price_usd }),
      })
    } catch (error) {
      return {
        exported: undefined,
        failure: 'launch-failed',
        output: { answer: undefined, charged: undefined, tokens: NO_TOKENS },
        result: { exitCode: 1, stderr: messageOf(error), stdout: '', timedOut: false },
      }
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
