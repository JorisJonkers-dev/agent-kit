import { randomBytes } from 'node:crypto'
import { join } from 'node:path'

import type { ProcessCommand, ProcessResult } from '../../../ports/index.js'
import { ModelGatewayError, type GatewayConfig, type ModelEntry } from '../domain/config.js'
import type { LedgerRecord } from '../domain/ledger.js'
import { familyOfModelId, modelKey } from '../domain/models.js'
import { decidePolicy, type ModelPermission } from '../domain/policy.js'
import {
  clientProof,
  CRITIC_CLIENT,
  exportedCost,
  finalAnswerOf,
  FORBIDDEN_PROVIDERS,
  hermesUsage,
  oneShotAnswer,
  PROVIDER_CLIENTS,
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
import { startWorkflow } from '../domain/workflows.js'
import { loadConfig } from './config.js'
import { messageOf } from './json-file.js'
import { overridePath, runsDir } from './locations.js'
import { resolveScope } from './policy.js'
import type { ClientConfig } from '../domain/config.js'
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

/** Where one launch reads its prompt and writes what the client reports. */
interface LaunchPaths {
  readonly promptPath: string
  readonly runDir: string
  readonly workDir: string
}

/** What a client's own output and usage say about the run it just did. */
interface RunReport {
  readonly answer: string | undefined
  readonly tokens: RunTokens
  /** The dollars the client itself priced, when it priced the run. */
  readonly cost: number | undefined
  /** Why the client's own report says the run failed, when it says so. */
  readonly failure: string | undefined
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
    const provider = this.providerFor(input, client)
    const entry = catalogueEntry(config, input.client, input.model)
    const family = entry?.family ?? familyOfModelId(input.model)
    if (family === undefined) {
      throw new ModelGatewayError(
        'unknown-model',
        `${input.client}:${input.model} is not a model the gateway can place: add it to the catalogue in defaults.json or the machine override`,
      )
    }
    const root = input.repo === undefined ? undefined : await this.repositoryRoot(input.repo)
    const allowed = await this.permission(input, config, family)
    await this.proveReadOnly(input, entry)
    const startedAt = this.deps.clock.now()
    const startedMs = this.deps.clock.monotonicMs()
    const id = runId(startedAt, randomBytes(3).toString('hex'))
    const runDir = await this.runDir(root, id)
    await this.joinWorkflow(input.workflow)
    const workDir = root ?? (await this.deps.files.tempDir('model-gateway-run-'))
    const paths: LaunchPaths = { promptPath: join(runDir, 'prompt.md'), runDir, workDir }
    const prompt = wrappedPrompt(input.prompt, config.settings.summary_max_chars)
    await this.deps.files.writeText(paths.promptPath, prompt)
    await this.deps.files.writeText(join(runDir, 'input.json'), `${JSON.stringify(input, null, 2)}\n`)
    const launched = await this.launch(client.binary, input, paths, prompt, provider)
    const reported = await this.report(input.client, paths, launched.result.stdout)
    const answer = reported.answer
    const reason = launched.failure ?? reported.failure ?? runFailureReason(launched.result, answer)
    const tokens = reported.tokens
    const cost = reported.cost ?? runCost(undefined, tokens, entry?.prices)
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

  /**
   * A run names its workflow, and the gateway keeps that workflow's life: one
   * named for the first time starts now, and one that ended is refused, since
   * its caps and grants lapsed with it. It runs once every refusal has passed,
   * so a refused run leaves no workflow started behind it.
   */
  private async joinWorkflow(name: string): Promise<void> {
    const now = this.deps.clock.now()
    await this.deps.workflows.update((workflows) => startWorkflow(workflows, name, now))
  }

  /**
   * The provider a run may use. A client that runs on a provider the caller
   * names must name one this machine lists for it, and no configuration can
   * turn on a provider the owner forbade.
   */
  private providerFor(input: RunInput, client: ClientConfig): string | undefined {
    const named = (PROVIDER_CLIENTS as readonly string[]).includes(input.client)
    if (input.provider === undefined) {
      if (!named) return undefined
      throw new ModelGatewayError(
        'provider-not-allowed',
        `a ${input.client} run must name its provider: this machine allows ${allowed(client)}`,
      )
    }
    const forbidden = (FORBIDDEN_PROVIDERS as readonly string[]).find(
      (provider) => provider === input.provider || input.model.startsWith(`${provider}/`),
    )
    if (forbidden !== undefined) {
      throw new ModelGatewayError('provider-not-allowed', `${forbidden} is not allowed: the owner forbade OpenRouter`)
    }
    if (!(client.providers ?? []).includes(input.provider)) {
      throw new ModelGatewayError(
        'provider-not-allowed',
        `${input.client} may not run on ${input.provider}: this machine allows ${allowed(client)}`,
      )
    }
    return input.provider
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
   * A read-only run goes in place only on a client model whose read-only proof
   * passed on this machine. Vibe's proof is setup's recorded result for the
   * critic client whose model this is, matched on the model id critique's
   * routing names, which the catalogue keeps among the model's `names` beside
   * the alias the run names. Every other client is proven by setup's result for
   * the client itself, which setup records for none of them yet, so a Hermes
   * run is refused; the throwaway worktree for unproven clients comes in a
   * later ticket.
   */
  private async proveReadOnly(input: RunInput, entry: ModelEntry | undefined): Promise<void> {
    const files = await this.proofFiles()
    const proof =
      input.client === CRITIC_CLIENT
        ? readOnlyProof(files, input.model, entry?.names ?? [])
        : clientProof(files, input.client)
    if (proof.status === 'unproven') {
      throw new ModelGatewayError(
        'unproven-read-only',
        `${input.client}:${input.model} has no passing read-only proof: ${proof.reason}`,
      )
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
   * The client, non-interactively, with the mark that stops it delegating on.
   * A launch that cannot start at all is a failed run, not an error: the caller
   * still gets a result and a ledger line.
   */
  private async launch(
    binary: string,
    input: RunInput,
    paths: LaunchPaths,
    prompt: string,
    provider: string | undefined,
  ): Promise<{ result: ProcessResult; failure?: string }> {
    const command: ProcessCommand = {
      ...(input.client === 'hermes' ? hermesCommand(binary, input, paths, prompt, provider) : vibeCommand(binary, input, paths)),
      cwd: paths.workDir,
      timeoutMs: (input.time_limit_s + OUTER_TIMEOUT_GRACE_S) * 1000,
    }
    try {
      return { result: await this.deps.process.exec(command) }
    } catch (error) {
      return { failure: 'launch-failed', result: { exitCode: 1, stderr: messageOf(error), stdout: '', timedOut: false } }
    }
  }

  /**
   * What the client's own output and usage say: Vibe's journal and its
   * `export.json`, or Hermes' one-shot answer and the usage file it wrote.
   */
  private async report(client: string, paths: LaunchPaths, stdout: string): Promise<RunReport> {
    if (client === 'hermes') {
      const usage = hermesUsage(await this.readJson(join(paths.runDir, 'usage.json')))
      return {
        answer: oneShotAnswer(stdout),
        cost: usage.cost,
        failure: usage.failed ? 'failed' : undefined,
        tokens: usage.tokens,
      }
    }
    const exported = await this.readJson(join(paths.runDir, 'vibe', 'export.json'))
    return { answer: finalAnswerOf(stdout), cost: exportedCost(exported), failure: undefined, tokens: runTokens(exported) }
  }

  /** Parsed JSON a client wrote, or undefined when it wrote none or it does not parse. */
  private async readJson(path: string): Promise<unknown> {
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

/** The providers a machine lets a client run on, as an error names them. */
function allowed(client: ClientConfig): string {
  const providers = client.providers ?? []
  return providers.length === 0 ? 'no provider' : providers.join(', ')
}

/**
 * Vibe, non-interactively: the wrapped prompt from a file, the client model by
 * the alias `models` lists, the read-only agent, and Vibe's own time and price
 * limits.
 */
function vibeCommand(binary: string, input: RunInput, paths: LaunchPaths): ProcessCommand {
  return {
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
      join(paths.runDir, 'vibe'),
    ],
    command: binary,
    env: DELEGATE_ENV,
  }
}

/**
 * Hermes, one-shot: the wrapped prompt as its argument, the client model and
 * the provider this machine allows, and the usage file it writes its own
 * tokens and dollars to. Hermes loads the working directory's rules and
 * memory as it always does, so the run carries the mark that stops it
 * delegating on and nothing else.
 */
function hermesCommand(
  binary: string,
  input: RunInput,
  paths: LaunchPaths,
  prompt: string,
  provider: string | undefined,
): ProcessCommand {
  return {
    args: ['-z', prompt, '--usage-file', join(paths.runDir, 'usage.json'), '-m', input.model, '--provider', String(provider)],
    command: binary,
    env: { [DELEGATION_MARK]: '1' },
  }
}

/** The catalogue entry for a client model, or undefined when the catalogue does not know it; a disabled model is refused. */
function catalogueEntry(config: GatewayConfig, client: string, model: string): ModelEntry | undefined {
  if (config.disabled.includes(modelKey(client, model))) {
    throw new ModelGatewayError('unknown-model', `${client}:${model} is disabled in this machine's configuration`)
  }
  return config.models.find((candidate) => candidate.client === client && candidate.model === model)
}
