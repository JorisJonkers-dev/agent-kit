import { mkdir, open as openFile, readFile, stat as statFile, writeFile } from 'node:fs/promises'
import { basename, dirname, isAbsolute, join } from 'node:path'

import { ProcessEnvAdapter, SystemClockAdapter } from '../adapters/clock/index.js'
import { NodeProcessAdapter, ProcessVerificationAdapter } from '../adapters/process/index.js'
import {
  FsLiveRunDirReader,
  FsRunStoreAdapter,
  normalizeLegacyRunDir,
  type WorkerResult,
} from '../adapters/runstore/index.js'
import {
  createWorktreeDependencyProvisioner,
  type WorktreeDependencyProvisionerPort,
} from '../adapters/worktree-provisioning/index.js'
import { resolveCouncilConfig } from '../contexts/config/index.js'
import {
  commandEngine,
  EngineGate,
  EngineGuardError,
  engineLeaseFileText,
  isGuardedEngine,
  modelPicks,
  parseEngineLeaseFile,
  parseEngineSpec,
  wrappedEngine,
  type EngineEnd,
  type EngineLease,
  type EngineRunGuard,
} from '../contexts/engines/index.js'
import { createMonitorFsAdapter } from '../contexts/monitor/index.js'
import {
  GitCliAdapter,
  planWaves,
  WorkerSupervisorAdapter,
  type WorkerSupervisorDependencies,
  type WorkerSupervisorEvent,
  type WorkerSupervisorResult,
  type WorkerSupervisorSession,
  type WorkerSupervisorSnapshot,
  type WorkerSupervisorStartRequest,
  type WorkerSupervisorWatchdogConfig,
} from '../contexts/graph/index.js'
import {
  workerDetectedEvent,
  workerExitedEvent,
  workerFinishedEvent,
  workerOutputEvent,
  workerRestartedEvent,
  workerStartedEvent,
  type RunStoreEvent,
  type WorkerLifecycleEvent,
} from '../contexts/runstore/index.js'
import { nodeEngineGuard, type NodeEngineGuardOptions } from '../contexts/model-gateway/index.js'
import {
  recommendLenses,
  type LensProblemProfile,
  type LensRecommendation,
  type TriageGatePayload,
} from '../contexts/triage/index.js'
import type {
  ClockPort,
  DagExecutorHooks,
  DagExecutorInput,
  DagExecutorResult,
  DagEvalConfig,
  DagEvalResult,
  DagSuperviseInput,
  DagSuperviseResult,
  GhPort,
  LiveRunDirReaderPort,
  ProcessPort,
} from '../ports/index.js'
import type { JsonRecord, Task } from '../shared-kernel/index.js'
import {
  assignAgents,
  configWorkflow,
  monitorList,
  monitorStatus,
  startMonitor,
  type MonitorListInput,
  type MonitorListResult,
  type MonitorStartInput,
  type MonitorStartResult,
  type MonitorStatusInput,
  type MonitorStatusResult,
  evalWorkflow,
  executeDagExecutorState,
  fanoutWorkflow,
  fleetWorkflow,
  parseAgentsPool,
  planWorkflow,
  reviewPackWorkflow,
  resolveDagWorkerCommand,
  statusWatchWorkflow,
  tailWorkflow,
  roundTripTasksMarkdownWorkflow,
  statusWorkflow as runStatusWorkflow,
  stringifyAssignments,
  triageWorkflow,
} from '../workflows/index.js'
import type {
  ConfigCommandInput,
  ConfigCommandResult,
  EvalWorkflowInput,
  EvalWorkflowResult,
  ExecuteDagDependency,
  ExecuteDagWorkflowInput,
  ExecutionPlan,
  FanoutBaseInput,
  FanoutInput,
  FleetBaseInput,
  FleetInput,
  PlanInput,
  PlanOnlyWorkflowInput,
  PlanResult,
  ReviewPackInput,
  RunSummary,
  StatusWatchInput,
  StatusWatchTickerInput,
  StatusWatchTickerPort,
  TailWorkflowFrame,
  TailWorkflowInput,
  TailWorkflowLogReaderPort,
  TriageWorkflowInput,
} from '../workflows/index.js'

export interface CouncilAppDeps {
  readonly clock?: ClockPort
  /**
   * The model gateway's guard for the `claude` and `codex` engine processes
   * council launches: it authorises a worker before it starts and records it
   * when it ends. The CLI wires the real one; an app built without one (as
   * every test builds it) launches workers unguarded.
   */
  readonly engineGuard?: EngineRunGuard
  readonly createRunStore?: (root: string) => SuperviseRunStore
  readonly createWorkerSupervisor?: (dependencies: SuperviseWorkerSupervisorDependencies) => SuperviseWorkerSupervisor
  readonly executeDag?: ExecuteDagDependency
  readonly gh?: GhPort
  readonly integrationWorktreePath?: string
  readonly liveRunDirReader?: LiveRunDirReaderPort
  readonly nowIso?: () => string
  readonly process?: ProcessPort
  readonly readText?: (path: string) => Promise<string>
  readonly repoRoot?: string
  readonly status?: (input: { readonly runDir: string }) => Promise<RunSummary>
  readonly statusTicker?: StatusWatchTickerPort
  readonly statusWriter?: CouncilStatusOutputWriter
  readonly tailLogReader?: TailWorkflowLogReaderPort
  readonly tailTicker?: CouncilTailTickerPort
  readonly worktreeDependencyProvisioner?: WorktreeDependencyProvisionerPort
  readonly worktreeRoot?: string
  readonly writeText?: (path: string, text: string) => Promise<void>
}

export interface CouncilAppExecuteDagInput extends Omit<ExecuteDagWorkflowInput, 'hooks'> {
  readonly hooks?: DagExecutorHooks
}

export type CouncilAppFanoutInput = FanoutBaseInput & (PlanOnlyWorkflowInput | CouncilAppExecuteDagInput)

export type CouncilAppFleetInput = FleetBaseInput & (PlanOnlyWorkflowInput | CouncilAppExecuteDagInput)

export interface CouncilStatusOutputWriter {
  write(output: string): Promise<void> | void
}

export interface CouncilAppLiveStatusInput extends StatusWatchInput {
  readonly writer?: CouncilStatusOutputWriter
}

export interface CouncilAppTailInput extends TailWorkflowInput {
  readonly intervalMs?: number
}

export type CouncilAppTailFrame = TailWorkflowFrame

export interface CouncilTailTickerInput {
  readonly intervalMs: number
}

export interface CouncilTailTickerPort {
  ticks(input: CouncilTailTickerInput): AsyncIterable<void>
}

export interface RecommendInput {
  readonly profile?: LensProblemProfile
}

export interface SuperviseInput {
  readonly runDir: string
  readonly taskId: string
  readonly worktree: string
  readonly command: string
  readonly args?: readonly string[]
  readonly stdin?: string
  /**
   * The engine and model the worker runs, as `<engine>:<model>` (for example
   * `codex:gpt-5.5`). A `claude` or `codex` worker is authorised by the model
   * gateway before it starts, under this run's workflow, and recorded in the
   * gateway's ledger when it ends.
   */
  readonly engine?: string
  /** How long the worker's prompt is, in characters, where it is not the stdin or arguments (a prompt file); the gateway's estimate counts it. */
  readonly promptChars?: number
  /** The worker's prompt carries no ticket or issue text; without this the gateway holds it to the repository's history permission. */
  readonly noHistory?: boolean
  readonly restartPreamble?: string
  readonly checkpointPreamble?: string
  readonly supportsStreamingStdin?: boolean
  readonly mcpProfile?: string
  readonly modelTier?: string
  readonly escalationModelTier?: string
  readonly pollIntervalMs?: number
  readonly killGraceMs?: number
  readonly watchdog?: WorkerSupervisorWatchdogConfig
}

export type SuperviseWorkerSupervisorDependencies = WorkerSupervisorDependencies
export type SuperviseWorkerSupervisorEvent = WorkerSupervisorEvent
export type SuperviseWorkerSupervisorResult = WorkerSupervisorResult
export type SuperviseWorkerSupervisorSession = WorkerSupervisorSession
export type SuperviseWorkerSupervisorSnapshot = WorkerSupervisorSnapshot
export type SuperviseWorkerSupervisorStartRequest = WorkerSupervisorStartRequest

export interface SuperviseWorkerSupervisor {
  reattach(
    request: SuperviseWorkerSupervisorStartRequest,
    snapshot: SuperviseWorkerSupervisorSnapshot,
  ): SuperviseWorkerSupervisorSession
  start(request: SuperviseWorkerSupervisorStartRequest): SuperviseWorkerSupervisorSession
}

export interface SuperviseRunStore {
  appendWorkerEvent(runId: string, event: WorkerLifecycleEvent): Promise<void>
  readEvents(runId: string): Promise<readonly RunStoreEvent[]>
  readWorkerSupervisorSnapshot(runId: string, taskId: string): Promise<SuperviseWorkerSupervisorSnapshot>
  writeWorkerResult(runId: string, taskId: string, result: WorkerResult): Promise<void>
  writeWorkerSupervisorSnapshot(
    runId: string,
    taskId: string,
    snapshot: SuperviseWorkerSupervisorSnapshot,
  ): Promise<void>
}

export interface SelfTestGolden {
  readonly agents: Readonly<Record<string, string>>
  readonly config: {
    readonly defaultIntensity: string
    readonly quickRoundOverride: number
    readonly thoroughWorker: string
  }
  readonly splitDestUrl: string
  readonly verify: {
    readonly localized: string
    readonly relative: string
  }
  readonly waves: readonly (readonly string[])[]
}

const NOOP_STATUS_WRITER: CouncilStatusOutputWriter = {
  write: () => undefined,
}

class ClockStatusTicker implements StatusWatchTickerPort {
  private readonly clock: ClockPort

  constructor(clock: ClockPort) {
    this.clock = clock
  }

  async *ticks(input: StatusWatchTickerInput): AsyncIterable<unknown> {
    for (;;) {
      await this.clock.sleep(input.intervalMs)
      yield undefined
    }
  }
}

class ClockTailTicker implements CouncilTailTickerPort {
  private readonly clock: ClockPort

  constructor(clock: ClockPort) {
    this.clock = clock
  }

  async *ticks(input: CouncilTailTickerInput): AsyncIterable<void> {
    for (;;) {
      await this.clock.sleep(input.intervalMs)
      yield
    }
  }
}

class FsTailWorkflowLogReader implements TailWorkflowLogReaderPort {
  async stat(input: { readonly path: string; readonly runDir: string }): Promise<{ readonly sizeBytes: number } | undefined> {
    try {
      const stats = await statFile(tailLogFilePath(input))
      return { sizeBytes: stats.size }
    } catch (error) {
      if (isErrno(error, 'ENOENT')) return undefined
      throw error
    }
  }

  async read(input: {
    readonly end: number
    readonly path: string
    readonly runDir: string
    readonly start: number
  }): Promise<Uint8Array> {
    const length = Math.max(0, input.end - input.start)
    const file = await openFile(tailLogFilePath(input), 'r')
    try {
      const buffer = Buffer.alloc(length)
      const result = await file.read(buffer, 0, length, input.start)
      return buffer.subarray(0, result.bytesRead)
    } finally {
      await file.close()
    }
  }
}

const DEFAULT_TAIL_INTERVAL_MS = 1000

export class CouncilApp {
  private readonly clock: ClockPort
  private readonly createRunStore: (root: string) => SuperviseRunStore
  private readonly engineGate: EngineGate | undefined
  private readonly createWorkerSupervisor: (
    dependencies: SuperviseWorkerSupervisorDependencies,
  ) => SuperviseWorkerSupervisor
  private readonly executeDagOverride: ExecuteDagDependency | undefined
  private readonly gh: GhPort | undefined
  private readonly integrationWorktreePath: string | undefined
  private readonly liveRunDirReader: LiveRunDirReaderPort
  private readonly nowIso: () => string
  private readonly processPort: ProcessPort
  private readonly readText: (path: string) => Promise<string>
  private readonly repoRoot: string | undefined
  private readonly statusPort: (input: { readonly runDir: string }) => Promise<RunSummary>
  private readonly statusTicker: StatusWatchTickerPort
  private readonly statusWriter: CouncilStatusOutputWriter
  private readonly tailLogReader: TailWorkflowLogReaderPort
  private readonly tailTicker: CouncilTailTickerPort
  private readonly worktreeDependencyProvisioner: WorktreeDependencyProvisionerPort
  private readonly worktreeRoot: string | undefined
  private readonly writeText: (path: string, text: string) => Promise<void>

  constructor(deps: CouncilAppDeps = {}) {
    this.clock = deps.clock ?? new SystemClockAdapter()
    this.createRunStore =
      deps.createRunStore ?? ((root) => new FsRunStoreAdapter(root) as unknown as SuperviseRunStore)
    this.createWorkerSupervisor =
      deps.createWorkerSupervisor ??
      ((dependencies) =>
        new WorkerSupervisorAdapter({
          ...dependencies,
          nowMs: () => Math.trunc(this.clock.monotonicMs()),
          sleep: (ms) => this.clock.sleep(ms),
        }))
    this.engineGate = deps.engineGuard === undefined ? undefined : new EngineGate(deps.engineGuard, this.clock)
    this.executeDagOverride = deps.executeDag
    this.gh = deps.gh
    this.integrationWorktreePath = deps.integrationWorktreePath
    this.liveRunDirReader = deps.liveRunDirReader ?? new FsLiveRunDirReader()
    this.nowIso = deps.nowIso ?? (() => this.clock.now().toISOString())
    this.processPort = deps.process ?? new NodeProcessAdapter()
    this.readText = deps.readText ?? ((path) => readFile(path, 'utf8'))
    this.repoRoot = deps.repoRoot
    this.statusPort =
      deps.status ??
      ((input) =>
        runStatusWorkflow(input, {
          normalizeRunDir: normalizeLegacyRunDir,
        }))
    this.statusTicker = deps.statusTicker ?? new ClockStatusTicker(this.clock)
    this.statusWriter = deps.statusWriter ?? NOOP_STATUS_WRITER
    this.tailLogReader = deps.tailLogReader ?? new FsTailWorkflowLogReader()
    this.tailTicker = deps.tailTicker ?? new ClockTailTicker(this.clock)
    this.worktreeDependencyProvisioner =
      deps.worktreeDependencyProvisioner ?? createWorktreeDependencyProvisioner()
    this.worktreeRoot = deps.worktreeRoot
    this.writeText = deps.writeText ?? writeTextFile
  }

  plan(input: PlanInput = {}): Promise<PlanResult> {
    return Promise.resolve(planWorkflow(input))
  }

  fanout(input: CouncilAppFanoutInput): Promise<ExecutionPlan> {
    return fanoutWorkflow(fanoutInputForWorkflow(input), {
      createPullRequest: (run) => this.createPullRequest(run),
      executeDag: this.executeDagForRun(input.runDir),
      status: (statusInput) => this.status(statusInput),
    })
  }

  fleet(input: CouncilAppFleetInput): Promise<ExecutionPlan> {
    return fleetWorkflow(fleetInputForWorkflow(input), {
      createPullRequest: (run) => this.createPullRequest(run),
      executeDag: this.executeDagForRun(fleetRunDir(input.tasksPath)),
      readText: this.readText,
    })
  }

  eval(input: EvalWorkflowInput): Promise<EvalWorkflowResult> {
    const target = runStoreTarget(input.runDir)
    const store = this.createRunStore(target.root)
    return evalWorkflow(input, {
      readEvents: (runId) => readRunEvents(store, runId),
      status: (statusInput) => this.status(statusInput),
    })
  }

  status(input: { readonly runDir: string }): Promise<RunSummary> {
    return this.statusPort(input)
  }

  async liveStatus(input: CouncilAppLiveStatusInput): Promise<void> {
    const writer = input.writer ?? this.statusWriter
    for await (const frame of statusWatchWorkflow(input, {
      clock: this.clock,
      readRunDir: (runDir) => this.liveRunDirReader.readRunDir(runDir),
      ticker: this.statusTicker,
    })) {
      await writer.write(frame.output)
    }
  }

  async tail(input: CouncilAppTailInput): Promise<readonly TailWorkflowFrame[]> {
    const frames: TailWorkflowFrame[] = []
    for await (const frame of tailWorkflow(input, {
      artifacts: this.liveRunDirReader,
      logs: this.tailLogReader,
      ticker: {
        ticks: () =>
          this.tailTicker.ticks({
            intervalMs: input.intervalMs ?? DEFAULT_TAIL_INTERVAL_MS,
          }),
      },
    })) {
      frames.push(frame)
    }
    return frames
  }

  readReviewPack(input: ReviewPackInput): Promise<import('../shared-kernel/index.js').JsonRecord> {
    return reviewPackWorkflow(input, {
      status: (statusInput) => this.status(statusInput),
    })
  }

  config(input: ConfigCommandInput): Promise<ConfigCommandResult> {
    return configWorkflow(input, {
      readText: this.readText,
      writeText: this.writeText,
    })
  }

  recommend(input: RecommendInput = {}): Promise<LensRecommendation> {
    return Promise.resolve(recommendLenses(input.profile))
  }

  triage(input: TriageWorkflowInput): Promise<TriageGatePayload> {
    return triageWorkflow(input, {
      writeText: this.writeText,
    })
  }

  supervise(input: SuperviseInput): Promise<WorkerResult> {
    return this.superviseWorker(input, commandEngine(input.command))
  }

  /**
   * Supervises one worker. `boundEngine` is the guarded engine the worker's
   * command really is: the command's own name for a direct call, or the
   * registry's engine for a native DAG worker, whose command is that engine's
   * shell line. The `--engine` the caller names must be that engine, so the
   * gateway always authorises the process that is actually spawned.
   */
  private async superviseWorker(input: SuperviseInput, boundEngine: string | undefined, fromRegistry = false): Promise<WorkerResult> {
    const target = runStoreTarget(input.runDir)
    const store = this.createRunStore(target.root)
    const workerId = `worker-${input.taskId}`
    let writes = Promise.resolve()
    const enqueue = (write: () => Promise<void>): void => {
      writes = writes.then(write)
    }
    const request = workerStartRequest(input)
    const snapshot = await readExistingSnapshot(store, target.runId, input.taskId)
    const leaseFile = join(input.runDir, 'workers', input.taskId, 'engine-lease.json')
    const lease = await this.leaseFor(input, boundEngine, fromRegistry, target.runId, leaseFile, snapshot !== undefined)
    let end: EngineEnd = { outcome: 'failed', reason: 'supervise-error' }
    let failed = false
    try {
      const supervisor = this.createWorkerSupervisor({
        onEvent: (event) => {
          const lifecycleEvent = workerLifecycleEvent(input, event, workerId, this.nowIso())
          if (lifecycleEvent !== undefined) {
            enqueue(() => store.appendWorkerEvent(target.runId, lifecycleEvent))
          }
        },
        onSnapshot: (snapshot) => {
          enqueue(() => store.writeWorkerSupervisorSnapshot(target.runId, input.taskId, snapshot))
        },
      })
      const session = snapshot === undefined ? supervisor.start(request) : supervisor.reattach(request, snapshot)
      const supervisorResult = await session.result
      end = superviseEnd(supervisorResult)
      try {
        await writes
        const result = workerResultFromSupervisor(input, supervisorResult)
        await store.writeWorkerResult(target.runId, input.taskId, result)
        await store.appendWorkerEvent(
          target.runId,
          workerFinishedEvent({
            finished_at: this.nowIso(),
            result_path: `workers/${input.taskId}/result.json`,
            status: result.status,
            task_id: input.taskId,
            worker_id: workerId,
          }),
        )
        return result
      } catch (error) {
        // The worker ran, but what it did could not be kept: the gateway is told it failed.
        end = { outcome: 'failed', reason: 'store-error' }
        throw error
      }
    } catch (error) {
      failed = true
      throw error
    } finally {
      await this.endLease(lease, end, leaseFile, failed)
    }
  }

  /**
   * Tells the gateway how the worker ended, whatever happened to it, and marks
   * the lease file so no later re-attach records it again. A ledger that cannot
   * take the line fails the call loudly, unless the call already failed: the
   * run's own error is the one that matters then.
   */
  private async endLease(lease: EngineLease | undefined, end: EngineEnd, leaseFile: string, failed: boolean): Promise<void> {
    if (lease === undefined) return
    try {
      await lease.end(end)
      await this.writeText(leaseFile, engineLeaseFileText({ state: 'ended' }))
    } catch (error) {
      if (!failed) throw error
    }
  }

  monitor(input: MonitorStartInput): Promise<MonitorStartResult> {
    return startMonitor(input, {
      process: this.processPort,
      fs: createMonitorFsAdapter(),
      nowIso: this.nowIso,
      nowMs: () => Math.trunc(this.clock.monotonicMs()),
      sleep: (ms) => this.clock.sleep(ms),
    })
  }

  monitorStatus(input: MonitorStatusInput): Promise<MonitorStatusResult> {
    return monitorStatus(input, {
      process: this.processPort,
      fs: createMonitorFsAdapter(),
      nowIso: this.nowIso,
      /* c8 ignore next -- monitorStatus does not invoke nowMs */
      nowMs: () => Math.trunc(this.clock.monotonicMs()),
      /* c8 ignore next -- monitorStatus does not invoke sleep */
      sleep: (ms) => this.clock.sleep(ms),
    })
  }

  monitorList(input: MonitorListInput): Promise<MonitorListResult> {
    return monitorList(input, {
      process: this.processPort,
      fs: createMonitorFsAdapter(),
      nowIso: this.nowIso,
      nowMs: () => Math.trunc(this.clock.monotonicMs()),
      /* c8 ignore next -- monitorList does not invoke sleep */
      sleep: (ms) => this.clock.sleep(ms),
    })
  }

  roundTripTasksMarkdown(tasksPath: string): Promise<readonly import('../shared-kernel/index.js').JsonRecord[]> {
    return roundTripTasksMarkdownWorkflow(tasksPath, { readText: this.readText })
  }

  /**
   * The gateway's lease for a worker, or undefined for a worker the gateway
   * does not guard. It fails closed:
   * - a `claude` or `codex` command needs a guard, `--engine <same engine>:<model>`
   *   and, if its arguments pick a model, that same model;
   * - a command that is neither names no guarded `--engine`.
   * A worker that is started gets a new lease, written to its lease file at
   * once. One that is re-attached to gets the lease its first supervisor left
   * (or none, if that run was already recorded), or, when no lease was ever
   * written, a new one marked as re-attached.
   */
  private async leaseFor(
    input: SuperviseInput,
    boundEngine: string | undefined,
    fromRegistry: boolean,
    runId: string,
    leaseFile: string,
    reattaching: boolean,
  ): Promise<EngineLease | undefined> {
    const named = input.engine?.split(':')[0]
    if (boundEngine === undefined) {
      // A command the caller picked may start a guarded engine from inside
      // itself (`env codex`, `sh -c 'claude ...'`); only a command that came
      // from the engine registry is trusted to be what its engine says.
      const wrapped = fromRegistry ? undefined : wrappedEngine(input.command, input.args ?? [])
      if (wrapped !== undefined) {
        throw new EngineGuardError('engine-wrapper', `${basename(input.command)} would start ${wrapped} past the model gateway: start ${wrapped} as the command itself, with --engine ${wrapped}:<model>`)
      }
      if (named !== undefined && isGuardedEngine(named)) {
        throw new EngineGuardError('engine-mismatch', `engine ${named} does not match the command ${basename(input.command)}: a guarded engine is named only for its own command`)
      }
      return undefined
    }
    const gate = this.engineGate
    if (gate === undefined) {
      throw new EngineGuardError('no-engine-guard', `${boundEngine} runs through the model gateway, and this council has no model gateway guard, so it is not started`)
    }
    if (input.engine === undefined) {
      throw new EngineGuardError('engine-mismatch', `${boundEngine} runs through the model gateway: name it with --engine ${boundEngine}:<model>`)
    }
    if (named !== boundEngine) {
      throw new EngineGuardError('engine-mismatch', `engine ${named ?? ''} does not match the command ${basename(input.command)}: the gateway authorises the engine that is spawned`)
    }
    const spec = parseEngineSpec(input.engine)
    const picks = modelPicks(spec.name, input.args ?? [])
    if (picks.overrides.length > 0) {
      throw new EngineGuardError('engine-model-override', `${basename(input.command)} sets ${picks.overrides.join(', ')}, which can change the model or provider the gateway authorised`)
    }
    const other = picks.models.find((model) => model !== spec.model)
    if (other !== undefined) {
      throw new EngineGuardError('engine-model-override', `${basename(input.command)} picks ${other} but --engine names ${spec.model}: the gateway holds the run to the model it is told`)
    }
    if (reattaching) {
      const standing = await this.readLeaseFile(leaseFile)
      if (standing?.state === 'ended') return undefined
      if (standing !== undefined) {
        const stored = standing.lease
        if (stored.council_run !== runId || stored.task_id !== input.taskId || stored.engine !== spec.name || stored.model !== spec.model) {
          throw new EngineGuardError('engine-lease-mismatch', `the lease in ${leaseFile} was opened for ${stored.engine}:${stored.model} of task ${stored.task_id} in run ${stored.council_run}, not for this worker: it is not resumed`)
        }
        try {
          return await gate.resume(stored, { carriesHistory: input.noHistory !== true, repo: input.worktree })
        } catch (error) {
          // The gateway ended the lease as failed; the file must say so, or a later re-attach would end it again.
          await this.writeText(leaseFile, engineLeaseFileText({ state: 'ended' })).catch(() => undefined)
          throw error
        }
      }
    }
    const wallClockCapMs = input.watchdog?.wallClockCapMs
    const lease = await gate.begin(
      {
        carriesHistory: input.noHistory !== true,
        engine: spec.name,
        model: spec.model,
        promptChars: await this.promptChars(input),
        repo: input.worktree,
        taskId: input.taskId,
        workflow: runId,
        ...(wallClockCapMs === undefined ? {} : { timeLimitS: Math.ceil(wallClockCapMs / 1000) }),
      },
      { reattached: reattaching },
    )
    try {
      await this.writeText(leaseFile, engineLeaseFileText({ lease: lease.persisted(), state: 'open' }))
    } catch (error) {
      // A lease nobody could find again would hold its reservation for ever.
      await lease.end({ outcome: 'failed', reason: 'lease-not-persisted' }).catch(() => undefined)
      throw error
    }
    return lease
  }

  /** The worker's lease file, or undefined when its first supervisor never wrote one; a file that is not a lease is an error. */
  private async readLeaseFile(path: string): Promise<ReturnType<typeof parseEngineLeaseFile> | undefined> {
    let text: string
    try {
      text = await this.readText(path)
    } catch (error) {
      if (isErrno(error, 'ENOENT')) return undefined
      throw error
    }
    return parseEngineLeaseFile(text)
  }

  /**
   * How long the worker's prompt is, never less than what the caller names: its
   * stdin, its arguments, and the prompt file the command is told to read. A
   * file that cannot be read is an error, since the estimate would not be.
   */
  private async promptChars(input: SuperviseInput): Promise<number> {
    const args = input.args ?? []
    const flag = args.indexOf('--prompt-file')
    const file = flag === -1 ? undefined : args[flag + 1]
    const fileChars = file === undefined ? 0 : (await this.readText(isAbsolute(file) ? file : join(input.worktree, file))).length
    const counted = (input.stdin?.length ?? 0) + args.reduce((total, arg) => total + arg.length, 0) + fileChars
    return Math.max(input.promptChars ?? 0, counted)
  }

  private async createPullRequest(run: string): Promise<string> {
    if (!this.gh) throw new Error('--github requires a gh adapter')
    const pr = await this.gh.createPullRequest({
      body: `Council run ${run}`,
      cwd: '.',
      draft: true,
      title: `Council ${run}`,
    })
    return pr.url
  }

  private executeDagForRun(runDir: string): ExecuteDagDependency {
    return async (input) => {
      const executeDag = this.executeDagOverride ?? ((request) => this.executeNativeDag(runDir, request))
      const result = await executeDag(input)
      return this.attachEvalResult(runDir, input.eval, result)
    }
  }

  private async executeNativeDag(
    runDir: string,
    input: DagExecutorInput,
  ): Promise<DagExecutorResult> {
    const git = new GitCliAdapter(this.processPort)
    const repoRoot = this.repoRoot ?? (await git.root(process.cwd()))
    const verifier = new ProcessVerificationAdapter(this.processPort)
    const target = runStoreTarget(runDir)
    return executeDagExecutorState({
      ...input,
      execution: {
        dependency_provisioner: this.worktreeDependencyProvisioner,
        git,
        integration_worktree_path: this.integrationWorktreePath ?? repoRoot,
        repo_root: repoRoot,
        worktree_root: this.worktreeRoot ?? join(repoRoot, '.worktrees', 'workers', input.run_id),
      },
      hooks: {
        provision: input.hooks.provision,
        supervise: (request) => this.superviseDagWorker(runDir, input, repoRoot, request),
        verify: (request) => verifier.verify(request),
      },
      repoFiles: new Set(input.tasks.flatMap((task) => task.paths)),
      run_store: this.createRunStore(target.root),
    })
  }

  private async superviseDagWorker(
    runDir: string,
    input: DagExecutorInput,
    repoRoot: string,
    request: DagSuperviseInput,
  ): Promise<DagSuperviseResult> {
    const agent = assignedDagAgent(input, request)
    const resolved = resolveDagWorkerCommand({
      agent,
      assignment: request.assignment,
      cwd: request.worktree_path,
      repoRoot,
      task: request.task,
    })
    await this.writeText(join(request.worktree_path, resolved.promptFile), resolved.prompt)
    const result = await this.superviseWorker({
      args: resolved.command.args,
      command: resolved.command.command,
      runDir,
      taskId: request.task.id,
      worktree: request.worktree_path,
      ...optional('engine', isGuardedEngine(resolved.engine.name) ? `${resolved.engine.name}:${resolved.engine.model}` : undefined),
      ...optional('mcpProfile', request.task.attachment?.mcpProfile),
      ...optional('modelTier', request.assignment.model),
      promptChars: resolved.prompt.length,
    }, isGuardedEngine(resolved.engine.name) ? resolved.engine.name : undefined, true)
    return {
      result,
      status: dagSuperviseStatus(result),
    }
  }

  private async attachEvalResult(
    runDir: string,
    evalConfig: DagEvalConfig | undefined,
    result: DagExecutorResult,
  ): Promise<DagExecutorResult> {
    if (evalConfig?.enabled !== true) return result
    const evaluation = await this.eval({ runDir })
    return {
      ...result,
      eval: dagEvalResult(evalConfig, evaluation),
    }
  }
}

/**
 * The gateway's guard on this machine, for the CLI to hand the app: the
 * machine-wide ledger, workflows and grants the gateway keeps, and the shipped
 * defaults in the model-gateway skill beside the council skill
 * (`<skills>/model-gateway/defaults.json`; council's own files sit in
 * `<skills>/council/ts-dist/composition/`).
 */
export function defaultEngineGuard(options: Partial<NodeEngineGuardOptions> = {}): EngineRunGuard {
  return nodeEngineGuard({
    clock: options.clock ?? new SystemClockAdapter(),
    defaultsPath: options.defaultsPath ?? join(import.meta.dirname, '..', '..', '..', 'model-gateway', 'defaults.json'),
    env: options.env ?? new ProcessEnvAdapter(),
  })
}

export function extractJson(text: string): unknown {
  const fenced = /```json\s*([\s\S]*?)\s*```/u.exec(text)
  if (fenced?.[1]) return JSON.parse(fenced[1])

  const start = text.indexOf('{')
  if (start < 0) throw new Error('no JSON object found')
  for (let end = text.length; end > start; end -= 1) {
    const candidate = text.slice(start, end).trim()
    if (!candidate.endsWith('}')) continue
    try {
      return JSON.parse(candidate)
    } catch {
      continue
    }
  }
  throw new Error('no JSON object found')
}

export function renderTemplate(template: string, values: Readonly<Record<string, string>>): string {
  return Object.entries(values).reduce(
    (rendered, [key, value]) => rendered.replaceAll(`{{${key}}}`, value),
    template,
  )
}

export function splitDestUrl(owner: string, name: string): string {
  return `git@github.com:${owner}/${name}.git`
}

export function localizeVerify(command: string, repoRoot: string, worktree: string): string {
  return command.replaceAll(repoRoot, worktree)
}

export function pythonSelfTestGolden(): SelfTestGolden {
  const tasks = [
    taskForSelfTest('T1', []),
    taskForSelfTest('T2', ['T1']),
    taskForSelfTest('T3', ['T1']),
    taskForSelfTest('T4', ['T2', 'T3']),
  ]
  const config = resolveCouncilConfig({ flags: { intensity: 'quick', rounds: 5 } })
  const agents = assignAgents(['t1', 't2', 't3'], parseAgentsPool('claude:haiku,codex:gpt-5.5'))
  return {
    agents: stringifyAssignments(agents),
    config: {
      defaultIntensity: resolveCouncilConfig().intensity,
      quickRoundOverride: config.rounds,
      thoroughWorker: resolveCouncilConfig({ flags: { intensity: 'thorough' } }).worker,
    },
    splitDestUrl: splitDestUrl('o', 'n'),
    verify: {
      localized: localizeVerify('cd /workspace/services/foo && npm test', '/workspace', '/tmp/wt/T1'),
      relative: localizeVerify('npm test', '/workspace', '/tmp/wt/T1'),
    },
    waves: planWaves(tasks),
  }
}

function taskForSelfTest(id: 'T1' | 'T2' | 'T3' | 'T4', dependsOn: readonly ('T1' | 'T2' | 'T3')[]): Task {
  return {
    boundaries: 'Stay in scope',
    depends_on: dependsOn,
    difficulty: 'moderate',
    id,
    model: 'haiku',
    objective: `Task ${id}`,
    output_format: 'Code edits',
    paths: [`${id}.txt`],
    title: id,
    verify: 'npm test',
  }
}

async function writeTextFile(path: string, text: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, text, 'utf8')
}

function fanoutInputForWorkflow(input: CouncilAppFanoutInput): FanoutInput {
  if (input.execute !== true) return input
  return {
    ...input,
    hooks: input.hooks ?? DEFAULT_DAG_EXECUTOR_HOOKS,
  }
}

function fleetInputForWorkflow(input: CouncilAppFleetInput): FleetInput {
  if (input.execute !== true) return input
  return {
    ...input,
    hooks: input.hooks ?? DEFAULT_DAG_EXECUTOR_HOOKS,
  }
}

const DEFAULT_DAG_EXECUTOR_HOOKS: DagExecutorHooks = {
  provision: (request) =>
    Promise.resolve({
      assignment: request.assignment,
      branch: `worker/${request.task.id}`,
      status: 'dry-run',
      worktree_path: '',
    }),
  supervise: (request) =>
    Promise.resolve({
      result: {
        status: 'skipped',
        task_id: request.task.id,
      },
      status: 'skipped',
    }),
  verify: (request) =>
    Promise.resolve({
      command: request.command,
      exit_code: null,
      status: 'skipped',
    }),
}

function fleetRunDir(tasksPath: string): string {
  return join(dirname(tasksPath), basename(tasksPath, '.json'))
}

function runStoreTarget(runDir: string): { readonly root: string; readonly runId: string } {
  const runId = basename(runDir)
  if (runId.length === 0) throw new Error('--run must point to a run directory')
  return {
    root: dirname(runDir),
    runId,
  }
}

function tailLogFilePath(input: { readonly path: string; readonly runDir: string }): string {
  return join(input.runDir, input.path)
}

async function readExistingSnapshot(
  store: SuperviseRunStore,
  runId: string,
  taskId: string,
): Promise<SuperviseWorkerSupervisorSnapshot | undefined> {
  try {
    return await store.readWorkerSupervisorSnapshot(runId, taskId)
  } catch (error) {
    if (isErrno(error, 'ENOENT')) return undefined
    throw error
  }
}

async function readRunEvents(store: SuperviseRunStore, runId: string): Promise<readonly RunStoreEvent[]> {
  try {
    return await store.readEvents(runId)
  } catch (error) {
    if (isErrno(error, 'ENOENT')) return []
    throw error
  }
}

function workerStartRequest(input: SuperviseInput): SuperviseWorkerSupervisorStartRequest {
  return {
    command: input.command,
    id: input.taskId,
    worktree: input.worktree,
    ...optional('args', input.args),
    ...optional('stdin', input.stdin),
    ...optional('restartPreamble', input.restartPreamble),
    ...optional('checkpointPreamble', input.checkpointPreamble),
    ...optional('supportsStreamingStdin', input.supportsStreamingStdin),
    ...optional('mcpProfile', input.mcpProfile),
    ...optional('modelTier', input.modelTier),
    ...optional('escalationModelTier', input.escalationModelTier),
    ...optional('pollIntervalMs', input.pollIntervalMs),
    ...optional('killGraceMs', input.killGraceMs),
    ...optional('watchdog', input.watchdog),
  }
}

function workerLifecycleEvent(
  input: SuperviseInput,
  event: SuperviseWorkerSupervisorEvent,
  workerId: string,
  nowIso: string,
): WorkerLifecycleEvent | undefined {
  if (event.type === 'started') {
    return workerStartedEvent({
      attempt: event.attemptId,
      command: [input.command, ...(input.args ?? [])],
      cwd: input.worktree,
      ...optional('model_tier', event.modelTier),
      ...optional('pid', event.pid),
      started_at: nowIso,
      task_id: event.taskId,
      worker_id: workerId,
    })
  }

  if (event.type === 'stdout' || event.type === 'stderr') {
    return workerOutputEvent({
      byte_count: event.byteCount,
      log_path: event.logPath,
      observed_at: nowIso,
      offset: event.offset,
      stream: event.type,
      tail: event.tail,
      tail_bytes: event.tailBytes,
      task_id: event.taskId,
      worker_id: workerId,
    })
  }

  if (event.type === 'detected') {
    return workerDetectedEvent({
      detected_at: nowIso,
      ...optional('pid', event.pid),
      status: event.detection.kind,
      task_id: event.taskId,
      worker_id: workerId,
    })
  }

  if (event.type === 'restarted') {
    return workerRestartedEvent({
      attempt: event.attemptId,
      ...optional('pid', event.pid),
      ...optional('previous_pid', event.previousPid),
      ...optional('reason', event.detection?.kind),
      restarted_at: nowIso,
      task_id: event.taskId,
      worker_id: workerId,
    })
  }

  if (event.type === 'exited') {
    return workerExitedEvent({
      exit_code: event.exitCode,
      exited_at: nowIso,
      ...optional('pid', event.pid),
      signal: event.signal,
      task_id: event.taskId,
      worker_id: workerId,
    })
  }

  return undefined
}

/** What a supervised worker's end tells the gateway: a clean finish is ok, a watchdog limit is a timeout, anything else failed with its exit code or its status. */
function superviseEnd(result: SuperviseWorkerSupervisorResult): EngineEnd {
  if (result.status === 'completed') return { outcome: 'ok' }
  if (result.status === 'stalled' || result.status === 'budget-cap') return { outcome: 'timeout', reason: result.status }
  const failed = result.exitCode !== null && result.exitCode !== 0
  return { outcome: 'failed', reason: failed ? `exit-${String(result.exitCode)}` : result.status }
}

function workerResultFromSupervisor(
  input: SuperviseInput,
  result: SuperviseWorkerSupervisorResult,
): WorkerResult {
  return {
    ...optional('model_tier', result.modelTier),
    status: result.status === 'completed' ? 'ok' : result.status,
    stderr_bytes: result.stderrBytes,
    stderr_log_path: result.stderrLogPath,
    stderr_tail: result.stderr,
    stdout_bytes: result.stdoutBytes,
    stdout_log_path: result.stdoutLogPath,
    stdout_tail: result.stdout,
    task_id: input.taskId,
    worktree: input.worktree,
  }
}

function assignedDagAgent(input: DagExecutorInput, request: DagSuperviseInput): DagExecutorInput['agent_pool']['available'][number] {
  return (
    input.agent_pool.available.find((candidate) => candidate.id === request.assignment.agent_id) ??
    input.agent_pool.available[0] ?? {
      id: request.assignment.agent_id,
      kind: request.task.engine?.cli ?? 'codex',
      model: request.assignment.model,
    }
  )
}

function dagSuperviseStatus(result: WorkerResult): DagSuperviseResult['status'] {
  return result.status === 'ok' || result.status === 'succeeded' ? 'succeeded' : 'failed'
}

function dagEvalResult(config: DagEvalConfig, evaluation: EvalWorkflowResult): DagEvalResult {
  const status = dagEvalStatus(config, evaluation.status)
  return {
    ...optional('command', config.command),
    exit_code: status === 'passed' ? 0 : 1,
    metadata: dagEvalMetadata(evaluation),
    output: `${evaluation.status} score=${String(evaluation.score)} findings=${String(evaluation.summary.finding_count)}`,
    status,
  }
}

function dagEvalStatus(
  config: DagEvalConfig,
  status: EvalWorkflowResult['status'],
): DagEvalResult['status'] {
  if (status === 'fail') return 'failed'
  if (config.require_clean_boundaries === true && status !== 'pass') return 'failed'
  return 'passed'
}

function dagEvalMetadata(evaluation: EvalWorkflowResult): JsonRecord {
  return {
    critical_finding_count: evaluation.summary.critical_finding_count,
    finding_count: evaluation.summary.finding_count,
    score: evaluation.score,
    status: evaluation.status,
    warning_finding_count: evaluation.summary.warning_finding_count,
  }
}

function isErrno(error: unknown, code: string): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    (error as { readonly code?: unknown }).code === code
  )
}

function optional<Key extends string, Value>(
  key: Key,
  value: Value | undefined,
): Partial<Record<Key, NonNullable<Value>>> {
  return value === undefined ? {} : ({ [key]: value } as Partial<Record<Key, NonNullable<Value>>>)
}
