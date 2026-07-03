import {
  applyPreFanoutGate,
  createTaskGraph,
  type PreFanoutGateViolation,
} from '../contexts/graph/index.js'
import type {
  DagAgentPool,
  DagConcurrency,
  DagEvalConfig,
  DagExecutorHooks,
  DagExecutorInput,
  DagExecutorResult,
} from '../ports/index.js'
import type { EngineDef, JsonRecord, Task } from '../shared-kernel/index.js'

import type { RunSummary } from './status.js'

export interface PlanOnlyWorkflowInput {
  readonly execute?: false
}

export interface ExecuteDagWorkflowInput {
  readonly baseRef: string
  readonly concurrency: DagConcurrency
  readonly eval?: DagEvalConfig
  readonly execute: true
  readonly hooks: DagExecutorHooks
  readonly integrationBranch: string
}

export type FanoutInput = FanoutBaseInput & (PlanOnlyWorkflowInput | ExecuteDagWorkflowInput)

export interface FanoutBaseInput {
  readonly dryRun: boolean
  readonly github: boolean
  readonly repoFiles?: ReadonlySet<string>
  readonly runDir: string
}

export interface ExecutionPlan {
  readonly agents?: Readonly<Record<string, string>>
  readonly execution?: DagExecutorResult
  readonly github: 'disabled' | 'dry-run' | 'created'
  readonly prUrl?: string
  readonly run: string
  readonly tasks: RunSummary['tasks']
  readonly waves: RunSummary['waves']
}

export type ExecuteDagDependency = (input: DagExecutorInput) => Promise<DagExecutorResult>

export interface FanoutWorkflowDeps {
  readonly createPullRequest: (run: string) => Promise<string>
  readonly executeDag?: ExecuteDagDependency
  readonly status: (input: { readonly runDir: string }) => Promise<RunSummary>
}

interface ExecuteDagWorkflowRequest {
  readonly agentPool: DagAgentPool
  readonly runId: string
  readonly tasks: readonly Task[]
}

export class PreFanoutGateError extends Error {
  readonly gateName = 'pre-fanout-static'
  readonly violations: readonly PreFanoutGateViolation[]

  constructor(violations: readonly PreFanoutGateViolation[]) {
    super(`pre-fanout static gate failed: ${violations.map(({ message }) => message).join('; ')}`)
    this.name = 'PreFanoutGateError'
    this.violations = violations
  }
}

export async function fanoutWorkflow(input: FanoutInput, deps: FanoutWorkflowDeps): Promise<ExecutionPlan> {
  const summary = await deps.status({ runDir: input.runDir })
  const gate = applyPreFanoutGate({
    graph: createTaskGraph(summary.tasks),
    repoFiles: repoFilesForGate(summary.tasks, input.repoFiles),
  })
  assertPreFanoutGatePassed(gate.violations)
  const github = await resolveGithub(input.github, input.dryRun, summary.run, deps.createPullRequest)
  const plan: ExecutionPlan = {
    github: github.kind,
    ...(github.url ? { prUrl: github.url } : {}),
    run: summary.run,
    tasks: summary.tasks,
    waves: gate.waves,
  }
  return executeDagIfRequested(input, deps.executeDag, plan, () => ({
    agentPool: plannedAgentPool(summary.tasks),
    runId: summary.run,
    tasks: summary.tasks,
  }))
}

export function assertPreFanoutGatePassed(
  violations: readonly PreFanoutGateViolation[],
): void {
  if (violations.length === 0) return
  throw new PreFanoutGateError(violations)
}

export function repoFilesForGate(
  tasks: RunSummary['tasks'],
  repoFiles: ReadonlySet<string> | undefined,
): ReadonlySet<string> {
  return repoFiles ?? new Set(tasks.flatMap((task) => task.paths))
}

export async function executeDagIfRequested(
  input: (PlanOnlyWorkflowInput | ExecuteDagWorkflowInput) & { readonly dryRun: boolean },
  executeDag: ExecuteDagDependency | undefined,
  plan: ExecutionPlan,
  request: () => ExecuteDagWorkflowRequest,
): Promise<ExecutionPlan> {
  if (input.execute !== true) return plan
  if (executeDag === undefined) throw new Error('executeDag dependency is required when execute=true')
  const executeRequest = request()
  const execution = await executeDag({
    agent_pool: executeRequest.agentPool,
    base_ref: input.baseRef,
    concurrency: input.concurrency,
    dry_run: input.dryRun,
    ...(input.eval !== undefined ? { eval: input.eval } : {}),
    hooks: input.hooks,
    integration_branch: input.integrationBranch,
    run_id: executeRequest.runId,
    tasks: executeRequest.tasks,
  })
  return { ...plan, execution }
}

export function engineMetadata(engine: EngineDef): JsonRecord {
  return {
    cli: engine.cli,
    label: engine.label ?? `${engine.cli}:${engine.model}`,
    model: engine.model,
  }
}

async function resolveGithub(
  github: boolean,
  dryRun: boolean,
  run: string,
  createPullRequest: (run: string) => Promise<string>,
): Promise<{ readonly kind: ExecutionPlan['github']; readonly url?: string }> {
  if (!github) return { kind: 'disabled' }
  if (dryRun) return { kind: 'dry-run' }
  return { kind: 'created', url: await createPullRequest(run) }
}

function plannedAgentPool(tasks: readonly Task[]): DagAgentPool {
  return {
    assignments: tasks.map((task) => ({
      agent_id: plannedAgentId(task.id),
      metadata: task.engine === undefined ? { model: task.model, source: 'task-model' } : engineMetadata(task.engine),
      model: task.model,
      task_id: task.id,
    })),
    available: tasks.map((task) => ({
      id: plannedAgentId(task.id),
      kind: task.engine?.cli ?? 'planned',
      metadata: task.engine === undefined ? { model: task.model, source: 'task-model' } : engineMetadata(task.engine),
      model: task.model,
    })),
  }
}

function plannedAgentId(taskId: string): string {
  return `task:${taskId}`
}
