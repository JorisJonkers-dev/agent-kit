import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { basename, dirname } from 'node:path'

import {
  FsRunStoreAdapter,
  normalizeLegacyRunDir,
  type WorkerResult,
} from '../adapters/runstore/index.js'
import { resolveCouncilConfig } from '../contexts/config/index.js'
import {
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
import type { GhPort } from '../ports/index.js'
import type { Task } from '../shared-kernel/index.js'
import {
  assignAgents,
  configWorkflow,
  evalWorkflow,
  fanoutWorkflow,
  fleetWorkflow,
  parseAgentsPool,
  planWorkflow,
  reviewPackWorkflow,
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
  ExecutionPlan,
  FanoutInput,
  FleetInput,
  PlanInput,
  PlanResult,
  ReviewPackInput,
  RunSummary,
  TriageWorkflowInput,
} from '../workflows/index.js'

export interface CouncilAppDeps {
  readonly createRunStore?: (root: string) => SuperviseRunStore
  readonly createWorkerSupervisor?: (dependencies: SuperviseWorkerSupervisorDependencies) => SuperviseWorkerSupervisor
  readonly gh?: GhPort
  readonly nowIso?: () => string
  readonly readText?: (path: string) => Promise<string>
  readonly status?: (input: { readonly runDir: string }) => Promise<RunSummary>
  readonly writeText?: (path: string, text: string) => Promise<void>
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

export class CouncilApp {
  private readonly createRunStore: (root: string) => SuperviseRunStore
  private readonly createWorkerSupervisor: (
    dependencies: SuperviseWorkerSupervisorDependencies,
  ) => SuperviseWorkerSupervisor
  private readonly gh: GhPort | undefined
  private readonly nowIso: () => string
  private readonly readText: (path: string) => Promise<string>
  private readonly statusPort: (input: { readonly runDir: string }) => Promise<RunSummary>
  private readonly writeText: (path: string, text: string) => Promise<void>

  constructor(deps: CouncilAppDeps = {}) {
    this.createRunStore =
      deps.createRunStore ?? ((root) => new FsRunStoreAdapter(root) as unknown as SuperviseRunStore)
    this.createWorkerSupervisor =
      deps.createWorkerSupervisor ?? ((dependencies) => new WorkerSupervisorAdapter(dependencies))
    this.gh = deps.gh
    this.nowIso = deps.nowIso ?? (() => new Date().toISOString())
    this.readText = deps.readText ?? ((path) => readFile(path, 'utf8'))
    this.statusPort =
      deps.status ??
      ((input) =>
        runStatusWorkflow(input, {
          normalizeRunDir: normalizeLegacyRunDir,
        }))
    this.writeText = deps.writeText ?? writeTextFile
  }

  plan(input: PlanInput = {}): Promise<PlanResult> {
    return Promise.resolve(planWorkflow(input))
  }

  fanout(input: FanoutInput): Promise<ExecutionPlan> {
    return fanoutWorkflow(input, {
      createPullRequest: (run) => this.createPullRequest(run),
      status: (statusInput) => this.status(statusInput),
    })
  }

  fleet(input: FleetInput): Promise<ExecutionPlan> {
    return fleetWorkflow(input, {
      createPullRequest: (run) => this.createPullRequest(run),
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
