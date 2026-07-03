import type {
  Amendment,
  DesignLedger,
  ReviewVerdict,
  RoutingVerdict,
  RunState,
  Story,
  Task,
} from '../shared-kernel/index.js'
import type { RunStoreEvent, WorkerLifecycleEvent } from '../contexts/runstore/index.js'

export interface RunStorePort {
  readState(runId: string): Promise<RunState>
  writeState(runId: string, state: RunState): Promise<void>
  readTasks(runId: string): Promise<readonly Task[]>
  writeTasks(runId: string, tasks: readonly Task[]): Promise<void>
  readStory(runId: string): Promise<Story>
  writeStory(runId: string, story: Story): Promise<void>
  readDesignLedger(runId: string): Promise<DesignLedger>
  writeDesignLedger(runId: string, ledger: DesignLedger): Promise<void>
  appendReviewVerdict(runId: string, verdict: ReviewVerdict): Promise<void>
  appendRoutingVerdict(runId: string, verdict: RoutingVerdict): Promise<void>
  appendAmendment(runId: string, amendment: Amendment): Promise<void>
  appendWorkerEvent(runId: string, event: WorkerLifecycleEvent): Promise<void>
  readEvents(runId: string): Promise<readonly RunStoreEvent[]>
}

export interface WorkerResult {
  readonly task_id: string
  readonly status: string
  readonly title?: string
  readonly model?: string
  readonly suggested_model?: 'haiku' | 'sonnet' | 'opus'
  readonly branch?: string
  readonly worktree?: string
  readonly committed?: boolean
  readonly summary?: string
  readonly files_changed?: readonly string[]
  readonly out_of_bounds?: readonly string[]
  readonly verify_rc?: number | null
  readonly verify_output?: string
  readonly verdict?: ReviewVerdict | null
  readonly merge?: string
  readonly error?: string
  readonly content_hash?: string
  readonly engine?: unknown
  readonly model_tier?: string
}

export interface LegacyTaskReport {
  readonly task_id: string
  readonly status?: string
  readonly merge?: string
  readonly model?: string
  readonly files_changed?: readonly string[]
  readonly verify_rc?: number | null
  readonly verifier_satisfied?: boolean
  readonly out_of_bounds?: readonly string[]
  readonly branch?: string
  readonly good?: boolean
}

export interface LegacyRunReport {
  readonly run: string
  readonly integration_branch?: string
  readonly integration_worktree?: string
  readonly waves: readonly (readonly string[])[]
  readonly tasks: readonly LegacyTaskReport[]
}

export interface NormalizedRunDirectory {
  readonly runId: string
  readonly state: RunState
  readonly tasks: readonly Task[]
  readonly report: LegacyRunReport | undefined
  readonly workerResults: ReadonlyMap<string, WorkerResult>
}

export interface LegacyRunNormalizerPort {
  normalizeRunDir(runDir: string): Promise<NormalizedRunDirectory>
}
