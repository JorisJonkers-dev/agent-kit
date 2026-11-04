import { randomBytes } from 'node:crypto'
import { join } from 'node:path'

import type { ProcessResult } from '../../../ports/index.js'
import { ModelGatewayError, type GatewayConfig, type ModelEntry } from '../domain/config.js'
import type { LedgerRecord } from '../domain/ledger.js'
import { familyOfModelId, modelKey } from '../domain/models.js'
import { decidePolicy, type ModelPermission } from '../domain/policy.js'
import {
  FORBIDDEN_PROVIDERS,
  PROVIDER_CLIENTS,
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
import { startWorkflow } from '../domain/workflows.js'
import { loadConfig } from './config.js'
import { messageOf } from './json-file.js'
import { overridePath, runsDir } from './locations.js'
import { DELEGATION_MARK, LAUNCHERS, READ_ONLY_AGENT, type ClientLauncher, type LaunchPaths } from './launchers.js'
import { resolveScope } from './policy.js'
import type { ClientConfig } from '../domain/config.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

export { DELEGATION_MARK, READ_ONLY_AGENT }

/** A writing run's input, narrowed: its repository, base and check are all named. */
type WritingInput = RunInput & { readonly mode: 'write'; readonly repo: string; readonly base: string; readonly check: string }

function isWriting(input: RunInput): input is WritingInput {
  return input.mode === 'write' && input.repo !== undefined && input.base !== undefined && input.check !== undefined
}

/** What a writing run adds to its result: the worktree it worked in, what is staged there, and how the check went. */
export interface WriteReport {
  readonly worktree: string
  readonly branch: string
  /**
   * The files the delegate's work staged, before the check ran; none for a run
   * whose delegate did not finish, and `null` when git could not list them.
   */
  readonly staged_files: readonly string[] | null
  /** The check the gateway ran after the delegate; absent when the delegate did not finish, so none was run. */
  readonly check?: CheckReport
  /** What git or the file system said when the gateway could not stage the worktree or keep the check's output. */
  readonly worktree_error?: string
}

/** One check command's outcome. */
export interface CheckReport {
  readonly command: string
  readonly exit_code: number
  /** The file the check's output (stdout, then stderr) was written to. */
  readonly output_path: string
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
  /**
   * Why a failed run failed: `timeout`, `parse-error`, `exit-<n>`,
   * `rate-limited`, `auth`, `launch-failed`, `denied-action`, `client-error`,
   * `failed` (Hermes' own usage report counts the run as failed) or, for a
   * writing run, `check-failed`, `check-output-failed` or `stage-failed`.
   */
  readonly reason?: string
  /** Set when the run's throwaway worktree could not be removed: what git said, and which directory is left behind. */
  readonly cleanup_warning?: string
  /** One line per plan window of this client at or above the warning ratio. */
  readonly warnings?: readonly string[]
  /** The worktree a writing run worked in, the branch it is on, its staged files and its check (writing runs only). */
  readonly worktree?: string
  readonly branch?: string
  readonly staged_files?: readonly string[] | null
  readonly check?: CheckReport
}

/**
 * Where a run works, and how it leaves: in the repository itself when the
 * client model's read-only proof passed, else in a throwaway directory that
 * `release` deletes.
 */
interface Workplace {
  readonly dir: string
  /** The branch a writing run's worktree is on. */
  readonly branch?: string
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
  readonly branch?: string
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
    if (input.client === 'opencode') requireAllowedProvider(input.model, client.providers)
    this.requireProvider(input, client)
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
    const throwaway = isWriting(input) ? false : await this.needsThrowaway(input, entry)
    await this.joinWorkflow(input.workflow)
    const startedAt = this.deps.clock.now()
    const startedMs = this.deps.clock.monotonicMs()
    const id = runId(startedAt, randomBytes(3).toString('hex'))
    const runDir = await this.runDir(root, id)
    const workplace = isWriting(input)
      ? await this.writeWorkplace(input, root ?? input.repo, id, startedAt)
      : await this.workplace(root, throwaway)
    let result: RunResult
    try {
      result = await this.delegate({
        binary: client.binary,
        config,
        entry,
        family,
        id,
        input,
        runDir,
        workDir: workplace.dir,
        ...(workplace.branch === undefined ? {} : { branch: workplace.branch }),
      })
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
    const launcher = LAUNCHERS[input.client as RunClient]
    const paths: LaunchPaths = {
      exportDir: join(runDir, 'vibe'),
      promptPath: join(runDir, 'prompt.md'),
      runDir,
      workDir: run.workDir,
    }
    const prompt = wrappedPrompt(input.prompt, config.settings.summary_max_chars, input.mode)
    await this.deps.files.writeText(paths.promptPath, prompt)
    await this.deps.files.writeText(join(runDir, 'input.json'), `${JSON.stringify(input, null, 2)}\n`)
    const launched = await this.launch(launcher, run.binary, input, paths, prompt)
    const reportPath = launcher.reportPath?.(paths)
    const written = reportPath === undefined ? undefined : await this.readJson(reportPath)
    const output = launcher.read(launched.result.stdout, written)
    const delegateReason = launched.failure ?? runFailureReason(launched.result, output.answer, output)
    const finished = isWriting(input) ? await this.finishWrite(run, input, delegateReason) : undefined
    const reason = finished === undefined ? delegateReason : finished.reason
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
      ...finished?.report,
    }
    return result
  }

  /**
   * What happens to a writing run's worktree once the delegate has ended. A
   * delegate that finished has its work staged first, and listed, and only
   * then gets the caller's check run in the worktree: whatever the check
   * writes (coverage, a build, a rewritten lockfile) is not the delegate's
   * diff, and stays an unstaged change in the worktree. A delegate that did
   * not finish (a timeout, an exit code, output that cannot be read) leaves
   * its worktree unstaged, with no check run. The gateway never commits,
   * merges or pushes: the worktree and its branch are the caller's to take.
   *
   * The delegate has run by now, so a failure of the gateway's own (git cannot
   * stage or list, the check's output cannot be kept) is a failed run with its
   * reason, never an error that would lose the result, the output and the
   * ledger line.
   */
  private async finishWrite(
    run: Delegation,
    input: WritingInput,
    delegateReason: string | undefined,
  ): Promise<{ reason: string | undefined; report: WriteReport }> {
    const base = { branch: String(run.branch), worktree: run.workDir }
    if (delegateReason !== undefined) return { reason: delegateReason, report: { ...base, staged_files: [] } }
    const staged = await this.stage(run.workDir)
    const check = await this.runCheck(run, input)
    const failure = staged.error ?? check.outputError
    const reason =
      staged.reason ??
      (check.outputError !== undefined ? 'check-output-failed' : check.report.exit_code === 0 ? undefined : 'check-failed')
    return {
      reason,
      report: { ...base, check: check.report, staged_files: staged.files, ...(failure === undefined ? {} : { worktree_error: failure }) },
    }
  }

  /**
   * Stages everything in the worktree and lists what that staged: the
   * delegate's diff, taken before the check can add to it. A git that cannot
   * stage leaves nothing staged; one that cannot list leaves the worktree
   * staged but unlisted, which the result says with `null` rather than an
   * empty list that would read as a delegate that changed nothing.
   */
  private async stage(workDir: string): Promise<{ files: readonly string[] | null; reason?: string; error?: string }> {
    try {
      await this.deps.git.stageAll(workDir)
    } catch (error) {
      return { error: messageOf(error), files: [], reason: 'stage-failed' }
    }
    try {
      return { files: await this.deps.git.stagedFiles(workDir) }
    } catch (error) {
      return { error: messageOf(error), files: null, reason: 'stage-list-failed' }
    }
  }

  /**
   * The caller's check, `sh -c <command>` in the worktree, held to the run's
   * time limit. Its output is kept in the run directory, and a check whose
   * output cannot be kept still reports its exit code; a check that cannot
   * start has exit code 127 and the reason as its output.
   */
  private async runCheck(run: Delegation, input: WritingInput): Promise<{ report: CheckReport; outputError?: string }> {
    let result: ProcessResult
    try {
      result = await this.deps.process.exec({
        args: ['-c', input.check],
        command: 'sh',
        cwd: run.workDir,
        timeoutMs: input.time_limit_s * 1000,
      })
    } catch (error) {
      result = { exitCode: 127, stderr: messageOf(error), stdout: '' }
    }
    const outputPath = join(run.runDir, 'check-output.txt')
    const report: CheckReport = { command: input.check, exit_code: result.exitCode, output_path: outputPath }
    try {
      await this.deps.files.writeText(outputPath, `${result.stdout}${result.stderr}`)
      return { report }
    } catch (error) {
      return { outputError: messageOf(error), report }
    }
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
  private requireProvider(input: RunInput, client: ClientConfig): void {
    const named = (PROVIDER_CLIENTS as readonly string[]).includes(input.client)
    if (input.provider === undefined) {
      if (!named) return
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
   * alias the run names. Every other model runs in a throwaway worktree, which
   * is every Hermes, Claude Code and OpenCode model: only Vibe and Antigravity
   * have a critic client that proves one.
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
    return join(await this.commonDir(root), 'model-gateway', 'runs', id)
  }

  /** The repository's common directory, which every gateway directory of the repository sits in. */
  private async commonDir(root: string): Promise<string> {
    try {
      return await this.deps.git.commonDir(root)
    } catch (error) {
      throw new ModelGatewayError('repository-unreadable', `${root}: ${messageOf(error)}`)
    }
  }

  /**
   * A writing run's worktree: created off the caller's base, in the
   * repository's gateway directory, on a branch of its own named after the
   * run. It is entered in the kept list at once, so whatever the run comes
   * to, the workflow's end can remove it; the run itself never does. A list
   * that cannot take it leaves the worktree nobody would remove, so it and its
   * branch are removed again, and the list's error is the run's.
   */
  private async writeWorkplace(input: WritingInput, root: string, id: string, at: Date): Promise<Workplace> {
    const dir = join(await this.commonDir(root), 'model-gateway', 'worktrees', id)
    const branch = `gateway/${id}`
    try {
      await this.deps.git.createWorktree(root, dir, branch, input.base)
    } catch (error) {
      await this.deps.files.remove(dir)
      throw new ModelGatewayError('worktree-failed', `${root}: cannot create a worktree off ${input.base}: ${messageOf(error)}`)
    }
    try {
      await this.deps.kept.add({ at: at.toISOString(), branch, path: dir, repo: root, run_id: id, workflow: input.workflow })
    } catch (error) {
      await this.deps.git.removeWorktree(root, dir).catch(() => undefined)
      await this.deps.git.deleteBranch(root, branch).catch(() => undefined)
      throw error
    }
    return { branch, dir, release: () => Promise.resolve() }
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
   * The client, non-interactively, through its launcher. A launch that cannot
   * start at all is a failed run, not an error: the caller still gets a
   * result and a ledger line.
   */
  private async launch(
    launcher: ClientLauncher,
    binary: string,
    input: RunInput,
    paths: LaunchPaths,
    prompt: string,
  ): Promise<{ result: ProcessResult; failure?: string }> {
    try {
      return { result: await this.deps.process.exec(launcher.command(binary, input, paths, prompt)) }
    } catch (error) {
      return { failure: 'launch-failed', result: { exitCode: 1, stderr: messageOf(error), stdout: '', timedOut: false } }
    }
  }

  /** Parsed JSON a client wrote beside its output, or undefined when it wrote none or it does not parse. */
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

/** The catalogue entry for a client model, or undefined when the catalogue does not know it; a disabled model is refused. */
function catalogueEntry(config: GatewayConfig, client: string, model: string): ModelEntry | undefined {
  if (config.disabled.includes(modelKey(client, model))) {
    throw new ModelGatewayError('unknown-model', `${client}:${model} is disabled in this machine's configuration`)
  }
  return config.models.find((candidate) => candidate.client === client && candidate.model === model)
}
