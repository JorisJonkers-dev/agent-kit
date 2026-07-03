import type { JsonRecord, Task, TaskId, TaskModel } from '../shared-kernel/index.js'
import type { WorkerResult } from './run-store.js'

export type DagTaskStatus = 'pending' | 'running' | 'succeeded' | 'failed' | 'skipped' | 'blocked'

export type DagExecutorStatus = 'succeeded' | 'failed' | 'partial' | 'dry-run'

export type DagVerifyStatus = 'passed' | 'failed' | 'skipped'

export type DagProvisionStatus = 'provisioned' | 'dry-run' | 'failed'

export type DagSkipReason =
  | 'dependency-failed'
  | 'dependency-skipped'
  | 'dry-run'
  | 'out-of-scope'
  | 'operator-skipped'

export interface DagAgent {
  readonly id: string
  readonly kind: string
  readonly model?: TaskModel
  readonly labels?: readonly string[]
  readonly max_concurrency?: number
  readonly metadata?: JsonRecord
}

export interface DagAgentAssignment {
  readonly task_id: TaskId
  readonly agent_id: string
  readonly model: TaskModel
  readonly reason?: string
  readonly metadata?: JsonRecord
}

export interface DagAgentPool {
  readonly available: readonly DagAgent[]
  readonly assignments: readonly DagAgentAssignment[]
}

export interface DagConcurrency {
  readonly max_parallel_tasks: number
  readonly per_agent?: Readonly<Record<string, number>>
}

export interface DagEvalConfig {
  readonly enabled: boolean
  readonly command?: string
  readonly require_clean_boundaries?: boolean
  readonly metadata?: JsonRecord
}

export interface DagEvalResult {
  readonly status: DagVerifyStatus
  readonly command?: string
  readonly exit_code?: number | null
  readonly output?: string
  readonly metadata?: JsonRecord
}

export interface DagProvisionInput {
  readonly run_id: string
  readonly task: Task
  readonly assignment: DagAgentAssignment
  readonly base_ref: string
  readonly integration_branch: string
}

export interface DagProvisionResult {
  readonly status: DagProvisionStatus
  readonly assignment?: DagAgentAssignment
  readonly branch?: string
  readonly worktree_path?: string
  readonly error?: string
  readonly metadata?: JsonRecord
}

export interface DagSuperviseInput {
  readonly run_id: string
  readonly task: Task
  readonly assignment: DagAgentAssignment
  readonly worktree_path: string
  readonly branch: string
  readonly dry_run: boolean
}

export interface DagSuperviseResult {
  readonly status: DagTaskStatus
  readonly result: WorkerResult
  readonly metadata?: JsonRecord
}

export interface DagVerifyInput {
  readonly run_id: string
  readonly task: Task
  readonly assignment: DagAgentAssignment
  readonly command: string
  readonly worktree_path: string
}

export interface DagVerifyResult {
  readonly status: DagVerifyStatus
  readonly command: string
  readonly exit_code: number | null
  readonly output?: string
  readonly metadata?: JsonRecord
}

export interface DagExecutorHooks {
  readonly provision: (request: DagProvisionInput) => Promise<DagProvisionResult>
  readonly supervise: (request: DagSuperviseInput) => Promise<DagSuperviseResult>
  readonly verify: (request: DagVerifyInput) => Promise<DagVerifyResult>
}

export interface DagExecutorInput {
  readonly run_id: string
  readonly base_ref: string
  readonly integration_branch: string
  readonly tasks: readonly Task[]
  readonly agent_pool: DagAgentPool
  readonly concurrency: DagConcurrency
  readonly dry_run: boolean
  readonly eval?: DagEvalConfig
  readonly hooks: DagExecutorHooks
}

export interface DagTaskResult {
  readonly task_id: TaskId
  readonly status: DagTaskStatus
  readonly assignment?: DagAgentAssignment
  readonly branch?: string
  readonly worktree_path?: string
  readonly commit?: string
  readonly files_changed?: readonly string[]
  readonly verify?: DagVerifyResult
  readonly worker_result?: WorkerResult
  readonly skipped_reason?: DagSkipReason
  readonly error?: string
  readonly metadata?: JsonRecord
}

export interface DagSkippedTask {
  readonly task_id: TaskId
  readonly status: 'skipped'
  readonly reason: DagSkipReason
  readonly dependency_task_id?: TaskId
}

export interface DagFailedTask {
  readonly task_id: TaskId
  readonly status: 'failed'
  readonly error: string
  readonly dependency_task_id?: TaskId
}

export interface DagExecutorResult {
  readonly run_id: string
  readonly base_ref: string
  readonly integration_branch: string
  readonly dry_run: boolean
  readonly status: DagExecutorStatus
  readonly task_results: readonly DagTaskResult[]
  readonly skipped_tasks: readonly DagSkippedTask[]
  readonly failed_tasks: readonly DagFailedTask[]
  readonly eval?: DagEvalResult
  readonly metadata?: JsonRecord
}

export interface DagExecutorPort {
  execute(input: DagExecutorInput): Promise<DagExecutorResult>
}
