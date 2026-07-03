import {
  applyBoundsGate,
  applyPreFanoutGate,
  createTaskGraph,
  dispatchReadySet,
  markTaskState,
  propagateStalled,
  type GraphTaskState,
  type PreFanoutGateResult,
  type TaskGraph,
} from '../contexts/graph/index.js'
import type {
  DagAgentAssignment,
  DagExecutorInput,
  DagExecutorResult,
  DagExecutorStatus,
  DagFailedTask,
  DagSkipReason,
  DagSkippedTask,
  DagTaskResult,
  DagTaskStatus,
  DagVerifyResult,
  GitDagExecutorPort,
  WorkerResult,
} from '../ports/index.js'
import type { Task, TaskId } from '../shared-kernel/index.js'

export interface DagExecutorDependencyProvisioningRequest {
  readonly repoRoot: string
  readonly worktreePath: string
}

export interface DagExecutorDependencyProvisionerPort {
  provision(request: DagExecutorDependencyProvisioningRequest): Promise<unknown>
}

export type DagExecutorExecutionGitPort = Pick<
  GitDagExecutorPort,
  'changedFiles' | 'commitAll' | 'createWorktree' | 'reconcileIntegrationBranch' | 'removeWorktree'
>

export interface DagExecutorExecutionPorts {
  readonly dependency_provisioner: DagExecutorDependencyProvisionerPort
  readonly git: DagExecutorExecutionGitPort
  readonly integration_worktree_path: string
  readonly repo_root: string
  readonly worktree_root: string
}

export interface DagExecutorStateInput extends DagExecutorInput {
  readonly execution?: DagExecutorExecutionPorts
  readonly repoFiles: ReadonlySet<string>
}

export interface DagExecutorTaskState {
  readonly blocked_by: readonly TaskId[]
  readonly graph_state: GraphTaskState
  readonly status: DagTaskStatus
  readonly task_id: TaskId
}

export interface DagExecutorStateSnapshot {
  readonly dispatched: readonly TaskId[]
  readonly graph: TaskGraph
  readonly task_states: readonly DagExecutorTaskState[]
}

export interface DagExecutorStateResult extends DagExecutorResult {
  readonly pre_fanout_gate: PreFanoutGateResult
  readonly state: DagExecutorStateSnapshot
}

interface TaskExecutionOutcome {
  readonly failed?: DagFailedTask
  readonly graphState: Extract<GraphTaskState, 'closed' | 'stalled'>
  readonly result: DagTaskResult
}

interface BlockedResults {
  readonly skippedTasks: readonly DagSkippedTask[]
  readonly taskResults: readonly DagTaskResult[]
}

interface FailureDetails {
  readonly assignment?: DagAgentAssignment
  readonly branch?: string
  readonly verify?: DagVerifyResult
  readonly worker_result?: WorkerResult
  readonly worktree_path?: string
}

interface PreparedTaskWorkspace {
  readonly assignment: DagAgentAssignment
  readonly branch: string
  readonly cleanup?: () => Promise<void>
  readonly worktreePath: string
}

interface FailedTaskWorkspace {
  readonly outcome: TaskExecutionOutcome
}

type TaskWorkspacePreparation = FailedTaskWorkspace | PreparedTaskWorkspace

export async function executeDagExecutorState(
  input: DagExecutorStateInput,
): Promise<DagExecutorStateResult> {
  let graph = createTaskGraph(input.tasks)
  const preFanoutGate = applyPreFanoutGate({ graph, repoFiles: input.repoFiles })
  const dispatched: TaskId[] = []
  const failedTasks: DagFailedTask[] = []
  const skippedTasks: DagSkippedTask[] = []
  const taskResults: DagTaskResult[] = []

  if (!preFanoutGate.ok) {
    return resultFor(input, 'failed', preFanoutGate, graph, dispatched, taskResults, skippedTasks, failedTasks)
  }

  if (input.dry_run) {
    const dryRun = dryRunResults(graph)
    return resultFor(
      input,
      executorStatus(input.dry_run, dryRun.taskResults, dryRun.skippedTasks, []),
      preFanoutGate,
      graph,
      dispatched,
      dryRun.taskResults,
      dryRun.skippedTasks,
      [],
    )
  }

  const assignmentByTaskId = new Map(input.agent_pool.assignments.map((assignment) => [assignment.task_id, assignment]))
  const limit = Math.max(1, Math.trunc(input.concurrency.max_parallel_tasks))
  let ready = dispatchReadySet(graph, limit)

  while (ready.length > 0) {
    ready.forEach((taskId) => {
      graph = markTaskState(graph, taskId, 'running')
    })
    dispatched.push(...ready)

    const readyTasks = ready.flatMap((taskId) =>
      [...graph.nodes.values()].filter((node) => node.task.id === taskId).map((node) => node.task),
    )
    const batch = await Promise.all(
      readyTasks.map((task) => executeTask(input, task, assignmentFor(task, assignmentByTaskId))),
    )

    batch.forEach((outcome) => {
      taskResults.push(outcome.result)
      if (outcome.failed !== undefined) {
        failedTasks.push(outcome.failed)
        graph = propagateStalled(graph, outcome.result.task_id)
        return
      }
      graph = markTaskState(graph, outcome.result.task_id, outcome.graphState)
    })

    ready = dispatchReadySet(graph, limit)
  }

  const blocked = blockedResultsFor(graph, taskResults)
  taskResults.push(...blocked.taskResults)
  skippedTasks.push(...blocked.skippedTasks)

  return resultFor(
    input,
    executorStatus(input.dry_run, taskResults, skippedTasks, failedTasks),
    preFanoutGate,
    graph,
    dispatched,
    taskResults,
    skippedTasks,
    failedTasks,
  )
}

async function executeTask(
  input: DagExecutorStateInput,
  task: Task,
  assignment: DagAgentAssignment,
): Promise<TaskExecutionOutcome> {
  const workspace = await prepareTaskWorkspace(input, task, assignment)
  if ('outcome' in workspace) {
    return workspace.outcome
  }

  try {
    return await executePreparedTask(input, task, workspace)
  } finally {
    if (workspace.cleanup !== undefined) {
      await workspace.cleanup()
    }
  }
}

async function prepareTaskWorkspace(
  input: DagExecutorStateInput,
  task: Task,
  assignment: DagAgentAssignment,
): Promise<TaskWorkspacePreparation> {
  if (input.execution !== undefined) {
    const execution = input.execution
    const worktree = await execution.git.createWorktree(
      execution.integration_worktree_path,
      `worker/${task.id}`,
      worktreePathFor(execution.worktree_root, task.id),
    )
    await execution.dependency_provisioner.provision({
      repoRoot: execution.repo_root,
      worktreePath: worktree.path,
    })
    return {
      assignment,
      branch: worktree.branch,
      cleanup: () => execution.git.removeWorktree(execution.integration_worktree_path, worktree.path),
      worktreePath: worktree.path,
    }
  }

  const provision = await input.hooks.provision({
    assignment,
    base_ref: input.base_ref,
    integration_branch: input.integration_branch,
    run_id: input.run_id,
    task,
  })

  if (provision.status !== 'provisioned') {
    return {
      outcome: failedOutcome(task, assignment, provision.error ?? `provision reported ${provision.status}`),
    }
  }

  const provisionedAssignment = provision.assignment ?? assignment
  return {
    assignment: provisionedAssignment,
    branch: provision.branch ?? `worker/${task.id}`,
    worktreePath: provision.worktree_path ?? '',
  }
}

async function executePreparedTask(
  input: DagExecutorStateInput,
  task: Task,
  workspace: PreparedTaskWorkspace,
): Promise<TaskExecutionOutcome> {
  const supervised = await input.hooks.supervise({
    assignment: workspace.assignment,
    branch: workspace.branch,
    dry_run: input.dry_run,
    run_id: input.run_id,
    task,
    worktree_path: workspace.worktreePath,
  })

  if (supervised.status !== 'succeeded') {
    return failedOutcome(task, workspace.assignment, `worker reported ${supervised.status}`, {
      branch: workspace.branch,
      worker_result: supervised.result,
      worktree_path: workspace.worktreePath,
    })
  }

  const verified = await input.hooks.verify({
    assignment: workspace.assignment,
    command: task.verify,
    run_id: input.run_id,
    task,
    worktree_path: workspace.worktreePath,
  })

  if (verified.status !== 'passed') {
    return failedOutcome(task, workspace.assignment, verifyFailureMessage(verified), {
      branch: workspace.branch,
      verify: verified,
      worker_result: supervised.result,
      worktree_path: workspace.worktreePath,
    })
  }

  if (input.execution !== undefined) {
    return finalizeExecutedTask(input, input.execution, task, workspace, supervised.result, verified)
  }

  return {
    graphState: 'closed',
    result: {
      assignment: workspace.assignment,
      branch: workspace.branch,
      status: 'succeeded',
      task_id: task.id,
      verify: verified,
      worker_result: supervised.result,
      worktree_path: workspace.worktreePath,
    },
  }
}

async function finalizeExecutedTask(
  input: DagExecutorStateInput,
  execution: DagExecutorExecutionPorts,
  task: Task,
  workspace: PreparedTaskWorkspace,
  workerResult: WorkerResult,
  verified: DagVerifyResult,
): Promise<TaskExecutionOutcome> {
  const filesChanged = await execution.git.changedFiles(workspace.worktreePath)
  const bounds = applyBoundsGate({
    allowedPaths: task.paths,
    filesChanged,
    taskId: task.id,
  })
  const boundedWorkerResult = workerResultWithExecutionDetails({
    bounds,
    branch: workspace.branch,
    committed: false,
    verified,
    workerResult,
    worktreePath: workspace.worktreePath,
  })

  if (bounds.status !== 'ok') {
    return failedOutcome(task, workspace.assignment, `bounds gate reported ${bounds.status}`, {
      branch: workspace.branch,
      verify: verified,
      worker_result: boundedWorkerResult,
      worktree_path: workspace.worktreePath,
    })
  }

  const committed = await execution.git.commitAll(workspace.worktreePath, {
    message: commitMessageFor(task),
  })
  await execution.git.reconcileIntegrationBranch(execution.integration_worktree_path, {
    baseBranch: input.base_ref,
    integrationBranch: input.integration_branch,
    sourceBranch: committed.branch,
  })

  return {
    graphState: 'closed',
    result: {
      assignment: workspace.assignment,
      branch: committed.branch,
      commit: committed.commit,
      files_changed: bounds.files_changed,
      status: 'succeeded',
      task_id: task.id,
      verify: verified,
      worker_result: workerResultWithExecutionDetails({
        bounds,
        branch: committed.branch,
        committed: true,
        verified,
        workerResult,
        worktreePath: workspace.worktreePath,
      }),
      worktree_path: workspace.worktreePath,
    },
  }
}

function workerResultWithExecutionDetails(input: {
  readonly bounds: ReturnType<typeof applyBoundsGate>
  readonly branch: string
  readonly committed: boolean
  readonly verified: DagVerifyResult
  readonly workerResult: WorkerResult
  readonly worktreePath: string
}): WorkerResult {
  return {
    ...input.workerResult,
    branch: input.branch,
    committed: input.committed,
    files_changed: input.bounds.files_changed,
    out_of_bounds: input.bounds.out_of_bounds,
    status: input.bounds.status === 'ok' ? input.workerResult.status : input.bounds.status,
    verify_rc: input.verified.exit_code,
    worktree: input.worktreePath,
    ...(input.verified.output === undefined ? {} : { verify_output: input.verified.output }),
  }
}

function commitMessageFor(task: Task): string {
  return `${task.id} ${task.title}`
}

function worktreePathFor(worktreeRoot: string, taskId: TaskId): string {
  return `${worktreeRoot.replace(/\/$/u, '')}/${taskId}`
}

function assignmentFor(
  task: Task,
  assignmentByTaskId: ReadonlyMap<TaskId, DagAgentAssignment>,
): DagAgentAssignment {
  return (
    assignmentByTaskId.get(task.id) ?? {
      agent_id: `task:${task.id}`,
      model: task.model,
      reason: 'Default assignment from task model.',
      task_id: task.id,
    }
  )
}

function failedOutcome(
  task: Task,
  assignment: DagAgentAssignment,
  error: string,
  details: FailureDetails = {},
): TaskExecutionOutcome {
  return {
    failed: {
      error,
      status: 'failed',
      task_id: task.id,
    },
    graphState: 'stalled',
    result: {
      assignment,
      error,
      status: 'failed',
      task_id: task.id,
      ...details,
    },
  }
}

function verifyFailureMessage(verify: DagVerifyResult): string {
  const exitCode = verify.exit_code === null ? 'unknown exit code' : `exit code ${String(verify.exit_code)}`
  return `verify failed with ${exitCode}`
}

function dryRunResults(graph: TaskGraph): BlockedResults {
  const taskResults = [...graph.nodes.keys()].map((taskId): DagTaskResult => ({
    skipped_reason: 'dry-run',
    status: 'skipped',
    task_id: taskId,
  }))
  const skippedTasks = [...graph.nodes.keys()].map((taskId): DagSkippedTask => ({
    reason: 'dry-run',
    status: 'skipped',
    task_id: taskId,
  }))
  return { skippedTasks, taskResults }
}

function blockedResultsFor(graph: TaskGraph, existingResults: readonly DagTaskResult[]): BlockedResults {
  const handled = new Set(existingResults.map(({ task_id }) => task_id))
  const statusByTaskId = new Map(existingResults.map(({ status, task_id }) => [task_id, status]))
  const taskResults: DagTaskResult[] = []
  const skippedTasks: DagSkippedTask[] = []

  for (const node of graph.nodes.values()) {
    if (node.state !== 'blocked' || handled.has(node.task.id)) {
      continue
    }

    const dependencyTaskId = node.blocked_by[0]
    const reason: DagSkipReason =
      dependencyTaskId !== undefined && statusByTaskId.get(dependencyTaskId) === 'skipped'
        ? 'dependency-skipped'
        : 'dependency-failed'
    const taskResult: DagTaskResult = {
      skipped_reason: reason,
      status: 'skipped',
      task_id: node.task.id,
    }
    const skippedTask: DagSkippedTask =
      dependencyTaskId === undefined
        ? { reason, status: 'skipped', task_id: node.task.id }
        : { dependency_task_id: dependencyTaskId, reason, status: 'skipped', task_id: node.task.id }

    handled.add(node.task.id)
    statusByTaskId.set(node.task.id, 'skipped')
    taskResults.push(taskResult)
    skippedTasks.push(skippedTask)
  }

  return { skippedTasks, taskResults }
}

function executorStatus(
  dryRun: boolean,
  taskResults: readonly DagTaskResult[],
  skippedTasks: readonly DagSkippedTask[],
  failedTasks: readonly DagFailedTask[],
): DagExecutorStatus {
  if (dryRun) {
    return 'dry-run'
  }
  if (failedTasks.length === 0 && skippedTasks.length === 0) {
    return 'succeeded'
  }
  if (taskResults.some(({ status }) => status === 'succeeded')) {
    return 'partial'
  }
  return 'failed'
}

function resultFor(
  input: DagExecutorStateInput,
  status: DagExecutorStatus,
  preFanoutGate: PreFanoutGateResult,
  graph: TaskGraph,
  dispatched: readonly TaskId[],
  taskResults: readonly DagTaskResult[],
  skippedTasks: readonly DagSkippedTask[],
  failedTasks: readonly DagFailedTask[],
): DagExecutorStateResult {
  return {
    base_ref: input.base_ref,
    dry_run: input.dry_run,
    failed_tasks: failedTasks,
    integration_branch: input.integration_branch,
    pre_fanout_gate: preFanoutGate,
    run_id: input.run_id,
    skipped_tasks: skippedTasks,
    state: snapshotFor(graph, dispatched, taskResults),
    status,
    task_results: taskResults,
  }
}

function snapshotFor(
  graph: TaskGraph,
  dispatched: readonly TaskId[],
  taskResults: readonly DagTaskResult[],
): DagExecutorStateSnapshot {
  const statusByTaskId = new Map(taskResults.map(({ status, task_id }) => [task_id, status]))
  return {
    dispatched,
    graph,
    task_states: [...graph.nodes.values()].map((node) => ({
      blocked_by: node.blocked_by,
      graph_state: node.state,
      status: statusByTaskId.get(node.task.id) ?? 'pending',
      task_id: node.task.id,
    })),
  }
}
