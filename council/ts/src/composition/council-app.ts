import { spawn } from 'node:child_process'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { basename, dirname, join } from 'node:path'

import { SystemClockAdapter } from '../adapters/clock/index.js'
import { ProcessVerificationAdapter } from '../adapters/process/index.js'
import {
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
  ProcessCommand,
  ProcessPort,
  ProcessResult,
} from '../ports/index.js'
import type { JsonRecord, Task } from '../shared-kernel/index.js'
import {
  assignAgents,
  configWorkflow,
  evalWorkflow,
  executeDagExecutorState,
  fanoutWorkflow,
  fleetWorkflow,
  parseAgentsPool,
  planWorkflow,
  reviewPackWorkflow,
  resolveDagWorkerCommand,
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
  TriageWorkflowInput,
} from '../workflows/index.js'

export interface CouncilAppDeps {
  readonly clock?: ClockPort
  readonly createRunStore?: (root: string) => SuperviseRunStore
  readonly createWorkerSupervisor?: (dependencies: SuperviseWorkerSupervisorDependencies) => SuperviseWorkerSupervisor
  readonly executeDag?: ExecuteDagDependency
  readonly gh?: GhPort
  readonly integrationWorktreePath?: string
  readonly nowIso?: () => string
  readonly process?: ProcessPort
  readonly readText?: (path: string) => Promise<string>
  readonly repoRoot?: string
  readonly status?: (input: { readonly runDir: string }) => Promise<RunSummary>
  readonly worktreeDependencyProvisioner?: WorktreeDependencyProvisionerPort
  readonly worktreeRoot?: string
  readonly writeText?: (path: string, text: string) => Promise<void>
}

export interface CouncilAppExecuteDagInput extends Omit<ExecuteDagWorkflowInput, 'hooks'> {
  readonly hooks?: DagExecutorHooks
}

export type CouncilAppFanoutInput = FanoutBaseInput & (PlanOnlyWorkflowInput | CouncilAppExecuteDagInput)

export type CouncilAppFleetInput = FleetBaseInput & (PlanOnlyWorkflowInput | CouncilAppExecuteDagInput)

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

export class NodeProcessAdapter implements ProcessPort {
  async exec(command: ProcessCommand): Promise<ProcessResult> {
    const child = spawn(command.command, [...command.args], {
      ...(command.cwd === undefined ? {} : { cwd: command.cwd }),
      env: command.env === undefined ? process.env : { ...process.env, ...command.env },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk: Buffer) => {
      stdout += chunk.toString('utf8')
    })
    child.stderr.on('data', (chunk: Buffer) => {
      stderr += chunk.toString('utf8')
    })
    const timeout = setProcessTimeout(child, command.timeoutMs)
    const exitCode = await new Promise<number>((resolve, reject) => {
      child.once('error', reject)
      child.once('close', (code) => {
        resolve(processExitCode(code))
      })
    }).finally(() => {
      if (timeout !== undefined) clearTimeout(timeout)
    })

    return {
      exitCode,
      stderr,
      stdout,
    }
  }
}

export class CouncilApp {
  private readonly clock: ClockPort
  private readonly createRunStore: (root: string) => SuperviseRunStore
  private readonly createWorkerSupervisor: (
    dependencies: SuperviseWorkerSupervisorDependencies,
  ) => SuperviseWorkerSupervisor
  private readonly executeDagOverride: ExecuteDagDependency | undefined
  private readonly gh: GhPort | undefined
  private readonly integrationWorktreePath: string | undefined
  private readonly nowIso: () => string
  private readonly processPort: ProcessPort
  private readonly readText: (path: string) => Promise<string>
  private readonly repoRoot: string | undefined
  private readonly statusPort: (input: { readonly runDir: string }) => Promise<RunSummary>
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
    this.executeDagOverride = deps.executeDag
    this.gh = deps.gh
    this.integrationWorktreePath = deps.integrationWorktreePath
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

  async supervise(input: SuperviseInput): Promise<WorkerResult> {
    const target = runStoreTarget(input.runDir)
    const store = this.createRunStore(target.root)
    const workerId = `worker-${input.taskId}`
    let writes = Promise.resolve()
    const enqueue = (write: () => Promise<void>): void => {
      writes = writes.then(write)
    }
    const request = workerStartRequest(input)
    const snapshot = await readExistingSnapshot(store, target.runId, input.taskId)
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
  }

  roundTripTasksMarkdown(tasksPath: string): Promise<readonly import('../shared-kernel/index.js').JsonRecord[]> {
    return roundTripTasksMarkdownWorkflow(tasksPath, { readText: this.readText })
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
    const result = await this.supervise({
      args: resolved.command.args,
      command: resolved.command.command,
      runDir,
      taskId: request.task.id,
      worktree: request.worktree_path,
      ...optional('mcpProfile', request.task.attachment?.mcpProfile),
      ...optional('modelTier', request.assignment.model),
    })
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

function setProcessTimeout(
  child: ReturnType<typeof spawn>,
  timeoutMs: number | undefined,
): NodeJS.Timeout | undefined {
  if (timeoutMs === undefined) return undefined
  const timeout = setTimeout(() => {
    child.kill('SIGTERM')
  }, timeoutMs)
  timeout.unref()
  return timeout
}

function processExitCode(code: number | null): number {
  if (code !== null) return code
  return 124
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
