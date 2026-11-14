import { randomBytes } from 'node:crypto'
import { join, sep } from 'node:path'

import type { ProcessResult } from '../../../ports/index.js'
import { ModelGatewayError, type CostClass, type GatewayConfig, type ModelEntry } from '../domain/config.js'
import { CannotStartError, type EscalationReport, type UnavailableResult } from '../domain/escalation.js'
import { runRecords, type LedgerRecord } from '../domain/ledger.js'
import { familyOfModelId, modelKey } from '../domain/models.js'
import { decidePolicy, type ModelPermission } from '../domain/policy.js'
import {
  FORBIDDEN_PROVIDERS,
  namedReset,
  NO_TOKENS,
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
import { requireWorkflowOpen, startWorkflow } from '../domain/workflows.js'
import { CapGuard, MIN_GRANT_TIME_LEFT_S, runCostClass, type CapSource, type Reservation } from './caps.js'
import type { Estimate } from '../domain/caps.js'
import { ClientRun, type ClientRunOutcome } from './client-run.js'
import { loadConfig } from './config.js'
import { EscalatedRun, type AttemptOptions } from './escalation.js'
import { GrantGuard } from './grants.js'
import { messageOf } from './json-file.js'
import { overridePath, runsDir } from './locations.js'
import { DELEGATION_MARK, READ_ONLY_AGENT, type LaunchPaths } from './launchers.js'
import { requireUsable } from './litellm-catalogue.js'
import { resolveScope } from './policy.js'
import type { ClientConfig } from '../domain/config.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

export { DELEGATION_MARK, READ_ONLY_AGENT }

/** The brief a repository keeps for a Codex writing run, in the worktree the run works in. */
const CODEX_BRIEF_FILE = 'codex-brief.md'

/** How much of that brief a run carries: a longer one is cut here. */
const CODEX_BRIEF_MAX_CHARS = 20_000


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
  /** The other checkout of the repository whose changes beside the run made it fail. */
  readonly foreign_worktree?: string
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
   * `rate-limited`, `auth`, `model-unavailable`, `launch-failed`,
   * `denied-action`, `client-error`, `failed` (Hermes' own usage report counts
   * the run as failed), `check-failed`, `check-output-failed` or
   * `stage-failed`, and, for a writing run, `foreign-worktree-change` when a
   * checkout of the repository beside the run's own worktree changed while
   * it ran.
   */
  readonly reason?: string
  /** When the client model's own answer said it is usable again, so the gateway remembers the refusal until then; absent when it named no reset. */
  readonly unavailable_until?: string
  /** Set when the run's throwaway worktree could not be removed: what git said, and which directory is left behind. */
  readonly cleanup_warning?: string
  /** One line per plan window of this client at or above the warning ratio. */
  readonly warnings?: readonly string[]
  /** The worktree a writing run worked in, the branch it is on, its staged files and its check (writing runs only). */
  readonly worktree?: string
  readonly branch?: string
  readonly staged_files?: readonly string[] | null
  readonly check?: CheckReport
  /** This run's place in its delegation when it is a retry or a climb (2 and up); the first attempt carries none. */
  readonly attempt?: number
  /** The run id of the delegation's first attempt, on every later one. */
  readonly attempt_of?: string
}

/** What a delegation comes to: its last run with the attempts that led to it, or `unavailable` when nothing could start. */
export type RunOutcome = (RunResult & EscalationReport) | UnavailableResult

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
  /** When the attempt started, from which a client's own named reset is measured. */
  readonly at: Date
  /** The monotonic reading at that moment, from which how long the client took is measured. */
  readonly startedMs: number
  readonly binary: string
  readonly config: GatewayConfig
  readonly entry: ModelEntry | undefined
  readonly family: string
  readonly id: string
  readonly input: RunInput
  readonly runDir: string
  readonly workDir: string
  /** The repository root the run works in, when it names a repository. */
  readonly repo?: string
  readonly branch?: string
  /** The worktrees of the repository a writing run watches, with what they held before the delegate ran. */
  readonly watched?: readonly WatchedWorktree[]
  /** When the run's grant or workflow lapses; the check may not outlive it. */
  readonly endsAt?: Date
}

/** What one attempt is launched with: the escalation's own options, and the reservation a batch already took for it. */
export interface RunOptions extends AttemptOptions {
  /** A reservation taken up front for this attempt, which it uses and settles instead of reserving its own. */
  readonly reservation?: Reservation
  /** The batch plan and delegation this attempt belongs to, written on its ledger line. */
  readonly tag?: BatchTag
}

/** What a batch writes on the ledger line of each attempt of one of its delegations. */
export interface BatchTag {
  readonly batch_id: string
  readonly delegation: string
}

/** What a batch hands one delegation: its pre-taken reservation, its tag, and the slot it must hold on the client each attempt runs on. */
export interface DelegationOptions {
  readonly reservation?: Reservation
  readonly tag?: BatchTag
  /** Takes a slot of the client an attempt is about to run on; the attempt gives it back when it ends. */
  readonly slot?: (client: string) => Promise<() => void>
}

/** Where a client model stands for one delegation: the same facts a run settles before it launches anything. */
interface Placed {
  readonly allowed: RunPermission
  readonly client: ClientConfig
  readonly config: GatewayConfig
  readonly entry: ModelEntry | undefined
  readonly family: string
  readonly root: string | undefined
}

/** What a delegation needs before it can run, as a batch plan checks it: where it stands, what it costs and what holds that cost. */
export interface Checked extends Placed {
  readonly cap: CapSource
  readonly costClass: CostClass
  readonly estimate: Estimate | undefined
}

/** What the data policy allows this run, and the repository it is keyed by. */
interface RunPermission {
  readonly permission: ModelPermission
  readonly repository: string | null
}

/** One watched worktree: where it is, and what `git status --porcelain` said before the delegate ran. */
interface WatchedWorktree {
  readonly path: string
  readonly before: string
}

/**
 * Hands one read-only task to one client model and reports what came of it.
 *
 * Everything that can refuse the run happens before anything is launched: the
 * nesting guard, the data policy for the repository, and the read-only proof
 * of the client model. A run that is launched always leaves a run directory
 * and a ledger line, whether it succeeded or not.
 */
export class RunCommand implements ModelGatewayCommand<unknown, RunOutcome> {
  private readonly caps: CapGuard
  private readonly clientRun: ClientRun
  private readonly escalation: EscalatedRun
  private readonly grants: GrantGuard

  constructor(private readonly deps: ModelGatewayDependencies) {
    this.caps = new CapGuard(deps)
    this.clientRun = new ClientRun(deps)
    this.escalation = new EscalatedRun(deps)
    this.grants = new GrantGuard(deps)
  }

  /** One delegation: a run, retried, climbed and swapped as the escalation decides. */
  run(raw: unknown): Promise<RunOutcome> {
    return this.delegation(readRunInput(raw))
  }

  /**
   * One delegation whose input is already read, with the reservation a batch
   * took for it. Only its first attempt holds that reservation: a retry, a
   * climb or a swap reserves its own, as any attempt does.
   */
  delegation(input: RunInput, given: DelegationOptions = {}): Promise<RunOutcome> {
    let held = given.reservation
    return this.escalation.run(input, async (attempt, options) => {
      const taken = held
      held = undefined
      const give = await given.slot?.(attempt.client)
      try {
        return await this.runOnce(attempt, {
          ...options,
          ...(taken === undefined ? {} : { reservation: taken }),
          ...(given.tag === undefined ? {} : { tag: given.tag }),
        })
      } finally {
        give?.()
      }
    })
  }

  /**
   * What a delegation needs before it can run, as a batch plan checks it
   * without launching or reserving anything: its client model, the data
   * policy, its workflow, its estimate, and the grant that covers it. The
   * first thing wrong is the refusal it throws.
   */
  async check(input: RunInput): Promise<Checked> {
    const placed = await this.place(input)
    const costClass = runCostClass(placed.client, input)
    const estimate = this.caps.estimate(input, placed.entry, placed.config)
    await this.requireWorkflowOpen(input.workflow)
    return { ...placed, cap: await this.capSource(costClass, input), costClass, estimate }
  }

  /** A fresh run id, and the run directory that run keeps its prompt, output and result in. */
  async allot(root: string | undefined, at: Date): Promise<{ readonly runDir: string; readonly runId: string }> {
    const id = runId(at, randomBytes(3).toString('hex'))
    return { runDir: await this.runDir(root, id), runId: id }
  }

  /** One attempt: a single run of one client model, which the escalation wraps. */
  async runOnce(given: RunInput, options: RunOptions = {}): Promise<RunResult> {
    this.refuseNested()
    const { allowed, client, config, entry, family, root } = await this.place(given)
    const held = options.reservation
    const costClass = runCostClass(client, given)
    /**
     * The other checkouts of the repository, watched from before anything is
     * taken: the listing itself refuses the run when it cannot be read, so a
     * refusal leaves no reservation to settle, no workflow joined, and no
     * worktree kept behind it.
     */
    const watched = isWriting(given) ? await this.watchOthers(root ?? given.repo) : []
    const input = held !== undefined && costClass === 'metered-employer' ? await this.withinGrant(given) : given
    const blocked = await options.startCheck?.(input)
    if (blocked !== undefined) {
      await this.caps.release(held, this.deps.clock.now())
      throw new CannotStartError(blocked)
    }
    const throwaway = isWriting(input) ? false : await this.needsThrowaway(input, entry)
    const estimate = held?.estimate ?? this.caps.estimate(input, entry, config)
    await this.requireWorkflowOpen(input.workflow)
    const cap: CapSource = held === undefined ? await this.capSource(costClass, input) : { kind: 'metered-personal' }
    const startedAt = this.deps.clock.now()
    const startedMs = this.deps.clock.monotonicMs()
    const { runDir, runId: id } =
      held === undefined ? await this.allot(root, startedAt) : { runDir: held.record.run_dir, runId: held.record.run_id }
    const reservation =
      held ??
      (await this.caps.reserve({
        cap,
        config,
        costClass,
        estimate,
        input,
        now: this.deps.clock.now(),
        runDir,
        runId: id,
      }))
    await this.joinWorkflow(input.workflow, reservation)
    const workplace = isWriting(input)
      ? await this.writeWorkplace(input, root ?? input.repo, id, startedAt)
      : await this.workplace(root, throwaway)
    let result: RunResult
    try {
      result = await this.delegate({
        at: startedAt,
        startedMs,
        binary: client.binary,
        config,
        entry,
        family,
        id,
        input: reservation === undefined ? input : { ...input, max_price_usd: reservation.estimate.usd, time_limit_s: Math.min(input.time_limit_s, reservation.time_limit_s) },
        runDir,
        workDir: workplace.dir,
        ...(root === undefined ? {} : { repo: root }),
        ...(workplace.branch === undefined ? {} : { branch: workplace.branch }),
        ...(watched.length === 0 ? {} : { watched }),
        ...(reservation?.ends_at === undefined ? {} : { endsAt: reservation.ends_at }),
      })
    } catch (error) {
      await this.release(workplace)
      throw error
    }
    const warning = await this.release(workplace)
    const linked: RunResult = options.link === undefined ? result : { ...result, ...options.link }
    const released: RunResult = warning === undefined ? linked : { ...linked, cleanup_warning: warning }
    await this.deps.ledger.append(this.ledgerRecord(released, allowed.repository, startedMs, costClass, options.tag))
    await this.caps.settle(reservation, released.cost_usd, this.deps.clock.now())
    const warnings = warningsOf(
      input.client,
      planWindows(input.client, client.plan_limits ?? [], runRecords(await this.deps.ledger.read()), this.deps.clock.now(), config.settings.plan_warning_ratio),
    )
    const recorded: RunResult = warnings.length === 0 ? released : { ...released, warnings }
    await this.deps.files.writeText(join(runDir, 'result.json'), `${JSON.stringify(recorded, null, 2)}\n`)
    return recorded
  }

  /**
   * A metered-employer delegation that starts later than its plan was checked
   * (a batch's dependent) needs its grant to still stand: it is refused
   * `no-grant` when every grant lapsed, and `grant-expiring` when under a
   * minute is left on it, and a run with more time than that is held to what
   * is left.
   */
  private async withinGrant(input: RunInput): Promise<RunInput> {
    const now = this.deps.clock.now()
    const grant = await this.grants.require(input.workflow, input.client, now)
    const leftS = Math.floor((Date.parse(grant.expires_at) - now.getTime()) / 1000)
    if (leftS < MIN_GRANT_TIME_LEFT_S) {
      throw new ModelGatewayError('grant-expiring', `grant ${grant.id} for workflow ${input.workflow} has ${String(Math.max(leftS, 0))} seconds left, under the ${String(MIN_GRANT_TIME_LEFT_S)} a run needs`)
    }
    return leftS >= input.time_limit_s ? input : { ...input, time_limit_s: leftS }
  }

  /**
   * Where a client model stands for one delegation, or the refusal that says
   * why it cannot run: the client and its provider, the model the catalogue
   * places, the repository it works in and the data policy for it.
   */
  private async place(input: RunInput): Promise<Placed> {
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const client = config.clients[input.client]
    if (client === undefined) throw new ModelGatewayError('unknown-model', `the gateway knows no client ${input.client}`)
    if (input.client === 'opencode') requireAllowedProvider(input.model, client.providers)
    this.requireProvider(input, client)
    const entry = catalogueEntry(config, input.client, input.model)
    await requireUsable(this.deps, config, input.client, input.model)
    const family = entry?.family ?? familyOfModelId(input.model)
    if (family === undefined) {
      throw new ModelGatewayError(
        'unknown-model',
        `${input.client}:${input.model} is not a model the gateway can place: add it to the catalogue in defaults.json or the machine override`,
      )
    }
    const root = input.repo === undefined ? undefined : await this.repositoryRoot(input.repo)
    const allowed = await this.permission(input, config, family)
    return { allowed, client, config, entry, family, root }
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
    const codex = await this.codexWorker(run)
    const paths: LaunchPaths = {
      exportDir: join(runDir, 'vibe'),
      promptPath: join(runDir, 'prompt.md'),
      runDir,
      workDir: run.workDir,
      ...(codex.gitWritable === undefined ? {} : { gitWritable: codex.gitWritable }),
    }
    const prompt = wrappedPrompt(input.prompt, config.settings.summary_max_chars, input.mode, codex.worker)
    await this.deps.files.writeText(paths.promptPath, prompt)
    await this.deps.files.writeText(join(runDir, 'input.json'), `${JSON.stringify(input, null, 2)}\n`)
    const launched = await this.launch(run.binary, input, paths, prompt)
    const output = launched.output
    const delegateReason = launched.failure ?? runFailureReason(launched.result, output.answer, output)
    const finished = isWriting(input) ? await this.finishWrite(run, input, delegateReason, run.watched ?? []) : undefined
    const reason = finished === undefined ? delegateReason : finished.reason
    const unavailableUntil =
      delegateReason === 'rate-limited' || delegateReason === 'model-unavailable'
        ? namedReset(launched.result, output.error, new Date(run.at.getTime() + (this.deps.clock.monotonicMs() - run.startedMs)))
        : undefined
    const cost = priceRun(output.charged, output.tokens, entry?.prices)
    const outputPath = join(runDir, 'output.json')
    await this.deps.files.writeText(outputPath, launched.result.stdout)
    if (launched.result.stderr !== '') await this.deps.files.writeText(join(runDir, 'stderr.txt'), launched.result.stderr)
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
      ...(unavailableUntil === undefined ? {} : { unavailable_until: unavailableUntil.toISOString() }),
      ...finished?.report,
    }
    return result
  }

  /**
   * What a Codex writing run gets beyond the prompt: the git directories its
   * sandbox may write, so it can commit in the worktree, and the brief the
   * repository keeps for it. Every other run gets neither, and its prompt is
   * the one it always got.
   */
  private async codexWorker(run: Delegation): Promise<{ gitWritable?: readonly string[]; worker?: { brief?: string } }> {
    const input = run.input
    if (input.client !== 'codex' || !isWriting(input)) return {}
    const gitWritable = await this.codexGitWritable(run.workDir, run.repo ?? input.repo)
    if (gitWritable === undefined) return {}
    const brief = await this.codexBrief(run.workDir)
    return { gitWritable, worker: brief === undefined ? {} : { brief } }
  }

  /**
   * The git directories a Codex writing run may write: the worktree's own git
   * directory, which sits outside the worktree, and the repository's shared
   * object, ref and log directories. The common directory itself is never
   * named, so `.git/hooks` and `.git/config` stay read-only. Undefined when
   * git cannot say where they are: the run then goes on as it did before, and
   * the gateway stages whatever the delegate left uncommitted.
   */
  private async codexGitWritable(workDir: string, repo: string): Promise<readonly string[] | undefined> {
    try {
      const gitDir = await this.deps.git.gitDir(workDir)
      const common = await this.commonDir(repo)
      if (gitDir === common) return undefined
      return [gitDir, join(common, 'objects'), join(common, 'refs'), join(common, 'logs')]
    } catch {
      return undefined
    }
  }

  /** The brief the worktree keeps for a Codex writing run, cut at the limit; undefined when it keeps none. */
  private async codexBrief(workDir: string): Promise<string | undefined> {
    const path = join(workDir, CODEX_BRIEF_FILE)
    try {
      if (!(await this.deps.files.exists(path))) return undefined
      return (await this.deps.files.readText(path)).slice(0, CODEX_BRIEF_MAX_CHARS)
    } catch {
      return undefined
    }
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
   * A worktree of the repository the run watches is held to the state its
   * snapshot holds: one a delegate changed, or one that cannot be read when
   * the run ends, makes the run fail with
   * `foreign-worktree-change`, and the name of the worktree goes out beside
   * the result.
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
    watched: readonly WatchedWorktree[],
  ): Promise<{ reason: string | undefined; report: WriteReport }> {
    const base = { branch: String(run.branch), worktree: run.workDir }
    /**
     * A change of another checkout is checked before the delegate's own
     * failure: whatever went wrong inside the run, a checkout of the
     * repository beside it changed, and that is what the result says first.
     * The delegate's own work is staged as before, and only a delegate that
     * did not finish leaves nothing staged.
     */
    const foreign = await this.foreignChange(watched)
    const staged = delegateReason === undefined ? await this.stage(run.workDir, input.base) : undefined
    if (foreign !== undefined) {
      return {
        reason: 'foreign-worktree-change',
        report: {
          ...base,
          foreign_worktree: foreign.path,
          staged_files: staged === undefined ? [] : staged.files,
          ...(staged?.error === undefined ? {} : { worktree_error: staged.error }),
        },
      }
    }
    if (staged === undefined) return { reason: delegateReason, report: { ...base, staged_files: [] } }
    const check = await this.runCheck(run, input)
    if (check === undefined) return { reason: 'grant-expired', report: { ...base, staged_files: staged.files, ...(staged.error === undefined ? {} : { worktree_error: staged.error }) } }
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
   * delegate's diff, taken before the check can add to it. The listing is
   * taken against the run's base, so a change the delegate committed itself on
   * the worktree's branch is listed too. A git that cannot stage leaves
   * nothing staged; one that cannot list leaves the worktree staged but
   * unlisted, which the result says with `null` rather than an empty list that
   * would read as a delegate that changed nothing.
   */
  private async stage(workDir: string, base: string): Promise<{ files: readonly string[] | null; reason?: string; error?: string }> {
    try {
      await this.deps.git.stageAll(workDir)
    } catch (error) {
      return { error: messageOf(error), files: [], reason: 'stage-failed' }
    }
    try {
      return { files: await this.deps.git.stagedFiles(workDir, base) }
    } catch (error) {
      return { error: messageOf(error), files: null, reason: 'stage-list-failed' }
    }
  }

  /**
   * Every worktree of the repository but nothing of the run's own: where each
   * stands before the delegate runs, so the run can hold every other checkout
   * of the repository to the state the delegate found it in. Worktrees the
   * gateway itself keeps for its runs — everything under the repository's
   * common directory's `model-gateway/worktrees` — are not watched: they are
   * this gateway's own workplaces, whose changes are its own work and never
   * a change beside the run.
   */
  private async watchOthers(repo: string): Promise<readonly WatchedWorktree[]> {
    let worktrees: readonly string[]
    try {
      worktrees = await this.deps.git.listWorktrees(repo)
    } catch (error) {
      throw new ModelGatewayError('worktree-failed', `${repo}: cannot list the worktrees to watch: ${messageOf(error)}`)
    }
    const common = await this.commonDir(repo)
    const kept = join(common, 'model-gateway', 'worktrees')
    const watched: WatchedWorktree[] = []
    for (const path of worktrees) {
      if (path === kept || path.startsWith(`${kept}${sep}`)) continue
      try {
        watched.push({ before: await this.deps.git.status(path), path })
      } catch (error) {
        throw new ModelGatewayError('worktree-failed', `${repo}: cannot read the status of ${path}: ${messageOf(error)}`)
      }
    }
    return watched
  }

  /**
   * The watched worktree that is no longer as it was found: its listing
   * changed, or it cannot be read any more. The delegate has run by now, so
   * this never throws; a worktree that went away counts as changed.
   */
  private async foreignChange(watched: readonly WatchedWorktree[]): Promise<WatchedWorktree | undefined> {
    for (const one of watched) {
      try {
        if ((await this.deps.git.status(one.path)) !== one.before) return one
      } catch {
        return one
      }
    }
    return undefined
  }

  /**
   * The caller's check, `sh -c <command>` in the worktree, held to the run's
   * time limit and, for a run under a grant, to the time still left on its
   * grant and workflow when the check starts (at least a second): the delegate
   * has spent part of it. With none left the check is not started and this
   * gives undefined. The check carries the delegation mark, like the delegate,
   * so test code the delegate wrote cannot write a grant or delegate on. Its
   * output is kept in the run directory, and a check whose output cannot be
   * kept still reports its exit code; a check that cannot start has exit code
   * 127 and the reason as its output.
   */
  private async runCheck(run: Delegation, input: WritingInput): Promise<{ report: CheckReport; outputError?: string } | undefined> {
    let timeoutMs = input.time_limit_s * 1000
    if (run.endsAt !== undefined) {
      const leftMs = run.endsAt.getTime() - this.deps.clock.now().getTime()
      if (leftMs <= 0) return undefined
      timeoutMs = Math.min(timeoutMs, Math.max(leftMs, 1000))
    }
    let result: ProcessResult
    try {
      result = await this.deps.process.exec({
        args: ['-c', input.check],
        command: 'sh',
        cwd: run.workDir,
        env: { [DELEGATION_MARK]: '1' },
        timeoutMs,
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
   * What the run's estimate is held against: the grant that covers a
   * metered-employer run, which it is refused without, or the metered-personal
   * caps. It runs after the workflow is found open and before the estimate is
   * reserved, so a run without a grant reserves nothing and starts no
   * workflow.
   */
  private async capSource(costClass: CostClass, input: RunInput): Promise<CapSource> {
    if (costClass !== 'metered-employer') return { kind: 'metered-personal' }
    return { grant: await this.grants.require(input.workflow, input.client, this.deps.clock.now()), kind: 'grant' }
  }

  /**
   * Whether the run may join its workflow: one that ended is refused, and
   * nothing is started. It runs before the estimate is reserved, so a run the
   * caps refuse leaves no workflow started behind it.
   */
  private async requireWorkflowOpen(name: string): Promise<void> {
    requireWorkflowOpen(await this.deps.workflows.read(), name, this.deps.clock.now())
  }

  /**
   * A run names its workflow, and the gateway keeps that workflow's life: one
   * named for the first time starts now, and one that ended is refused, since
   * its caps and grants lapsed with it. It runs once the estimate is reserved,
   * so a run that cannot start its workflow releases the reservation rather
   * than holding it against the caps.
   */
  private async joinWorkflow(name: string, reservation: Reservation | undefined): Promise<void> {
    const now = this.deps.clock.now()
    try {
      await this.deps.workflows.update((workflows) => startWorkflow(workflows, name, now))
    } catch (error) {
      await this.caps.release(reservation, now)
      throw error
    }
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
  refuseNested(): void {
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
        mode: input.mode,
        model: input.model,
        paths,
        prompt,
        time_limit_s: input.time_limit_s,
        ...(input.max_price_usd === undefined ? {} : { max_price_usd: input.max_price_usd }),
        ...(input.provider === undefined ? {} : { provider: input.provider }),
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
  private ledgerRecord(result: RunResult, repository: string | null, startedMs: number, costClass: CostClass, tag?: BatchTag): LedgerRecord {
    return {
      at: this.deps.clock.now().toISOString(),
      client: result.client,
      cost_class: costClass,
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
      ...(result.unavailable_until === undefined ? {} : { unavailable_until: result.unavailable_until }),
      ...(result.attempt === undefined ? {} : { attempt: result.attempt }),
      ...(result.attempt_of === undefined ? {} : { attempt_of: result.attempt_of }),
      ...tag,
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
