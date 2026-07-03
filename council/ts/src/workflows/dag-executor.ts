import {
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
  WorkerResult,
} from '../ports/index.js'
import type { Task, TaskId } from '../shared-kernel/index.js'

export interface DagExecutorStateInput extends DagExecutorInput {
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
  const provision = await input.hooks.provision({
    assignment,
    base_ref: input.base_ref,
    integration_branch: input.integration_branch,
    run_id: input.run_id,
    task,
  })

  if (provision.status !== 'provisioned') {
    return failedOutcome(task, assignment, provision.error ?? `provision reported ${provision.status}`)
  }

  const provisionedAssignment = provision.assignment ?? assignment
  const branch = provision.branch ?? `worker/${task.id}`
  const worktreePath = provision.worktree_path ?? ''
  const supervised = await input.hooks.supervise({
    assignment: provisionedAssignment,
    branch,
    dry_run: input.dry_run,
    run_id: input.run_id,
    task,
    worktree_path: worktreePath,
  })

  if (supervised.status !== 'succeeded') {
    return failedOutcome(task, provisionedAssignment, `worker reported ${supervised.status}`, {
      branch,
      worker_result: supervised.result,
      worktree_path: worktreePath,
    })
  }

  const verified = await input.hooks.verify({
    assignment: provisionedAssignment,
    command: task.verify,
    run_id: input.run_id,
    task,
    worktree_path: worktreePath,
  })

  if (verified.status !== 'passed') {
    return failedOutcome(task, provisionedAssignment, verifyFailureMessage(verified), {
      branch,
      verify: verified,
      worker_result: supervised.result,
      worktree_path: worktreePath,
    })
  }

  return {
    graphState: 'closed',
    result: {
      assignment: provisionedAssignment,
      branch,
      status: 'succeeded',
      task_id: task.id,
      verify: verified,
      worker_result: supervised.result,
      worktree_path: worktreePath,
    },
  }
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
