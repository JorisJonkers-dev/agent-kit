import type {
  Amendment,
  DesignLedgerEntry,
  ReviewVerdict,
  RoutingVerdict,
  Story,
  Task,
  TaskId,
} from '../../../shared-kernel/index.js'

export interface SpendEstimateInput {
  readonly rounds?: number
  readonly fanout_verifier_calls_per_task?: number
}

export interface SpendEstimate {
  readonly plan_model_calls: number
  readonly fanout_worker_calls: number
  readonly fanout_verifier_calls: number
  readonly total_model_calls: number
  readonly task_count: number
  readonly wave_count: number
}

export interface TaskCard {
  readonly id: TaskId
  readonly title: string
  readonly objective: string
  readonly difficulty: Task['difficulty']
  readonly model: Task['model']
  readonly depends_on: readonly TaskId[]
  readonly paths: readonly string[]
  readonly verify_present: boolean
  readonly boundaries: string
  readonly acceptance_criteria: readonly string[]
}

export interface DagDepth {
  readonly depth: number
  readonly task_ids: readonly TaskId[]
  readonly tasks: readonly TaskCard[]
}

export interface StoryCard {
  readonly title: string
  readonly status: string
  readonly goal: string
  readonly actor: string
  readonly capability: string
  readonly outcome: string
  readonly acceptance_criteria: readonly string[]
  readonly in_scope: readonly string[]
  readonly out_of_scope: readonly string[]
}

export interface GrillLedgerEntry {
  readonly task_id: string | null
  readonly reviewer: string | null
  readonly satisfied: boolean
  readonly reasons: string
  readonly issues: readonly string[]
}

export interface CheckpointOneInput {
  readonly routing_verdict: RoutingVerdict | readonly RoutingVerdict[]
  readonly tasks: readonly Task[]
  readonly stories?: readonly Story[]
  readonly grill_verdicts?: readonly ReviewVerdict[]
  readonly spend?: SpendEstimateInput
}

export interface CheckpointOnePack {
  readonly kind: 'checkpoint-1'
  readonly routing_verdicts: readonly RoutingVerdict[]
  readonly dag_by_depth: readonly DagDepth[]
  readonly story_cards: readonly StoryCard[]
  readonly grill_ledger: readonly GrillLedgerEntry[]
  readonly spend_estimate: SpendEstimate
}

export interface LockedSpecInput {
  readonly title: string
  readonly markdown: string
  readonly spec_ref?: string
  readonly locked_by?: string
}

export interface SectionIndexEntry {
  readonly section_ref: `§${number}`
  readonly level: number
  readonly title: string
  readonly anchor: string
  readonly line: number
}

export interface LockedSpec {
  readonly title: string
  readonly markdown: string
  readonly spec_ref: string | null
  readonly locked_by: string | null
  readonly section_index: readonly SectionIndexEntry[]
}

export interface SettledContestedLedger {
  readonly settled: readonly DesignLedgerEntry[]
  readonly contested: readonly DesignLedgerEntry[]
}

export interface DesignVote {
  readonly voter: string
  readonly vote: string
  readonly option: string
  readonly rationale?: string
  readonly conditions?: readonly string[]
}

export interface DesignVoteRow {
  readonly voter: string
  readonly vote: string
  readonly option: string
  readonly rationale: string | null
  readonly conditions: readonly string[]
}

export interface DesignVoteCount {
  readonly option: string
  readonly count: number
}

export interface DesignVoteTable {
  readonly rows: readonly DesignVoteRow[]
  readonly counts: readonly DesignVoteCount[]
}

export interface DesignCheckpointInput {
  readonly locked_spec: LockedSpecInput
  readonly ledger_entries: readonly DesignLedgerEntry[]
  readonly votes: readonly DesignVote[]
}

export interface DesignCheckpointPack {
  readonly kind: 'design-checkpoint'
  readonly locked_spec: LockedSpec
  readonly ledger: SettledContestedLedger
  readonly vote_table: DesignVoteTable
  readonly section_index: readonly SectionIndexEntry[]
}

export interface TaskExecutionResult {
  readonly task_id: string
  readonly status: string
  readonly merge?: string
  readonly model?: string
  readonly files_changed?: readonly string[]
  readonly verify_rc?: number | null
  readonly verdict?: ReviewVerdict | null
  readonly out_of_bounds?: readonly string[]
  readonly branch?: string
}

export interface TaskOutcomeRow {
  readonly task_id: string
  readonly title: string
  readonly status: string
  readonly merge: string | null
  readonly model: string | null
  readonly files_changed_count: number
  readonly verify_rc: number | null
  readonly verifier_satisfied: boolean | null
  readonly out_of_bounds: readonly string[]
  readonly branch: string | null
  readonly good: boolean
  readonly has_verify: boolean
}

export interface CheckpointTwoSummary {
  readonly total: number
  readonly ok: number
  readonly failed: number
  readonly merged: number
  readonly conflicts: number
  readonly no_verify: number
}

export interface FileDiffStat {
  readonly path: string
  readonly additions: number
  readonly deletions: number
}

export interface DiffStats {
  readonly files_changed: number
  readonly additions: number
  readonly deletions: number
  readonly by_file: readonly FileDiffStat[]
}

export interface PruningProposal {
  readonly id: string
  readonly title: string
  readonly recommendation: string
  readonly delete_now: boolean
  readonly rationale?: string
  readonly files?: readonly string[]
}

export interface PullRequestLink {
  readonly url: string
  readonly number?: number
  readonly title?: string
}

export interface CheckpointTwoInput {
  readonly run: string
  readonly integration_branch: string
  readonly integration_worktree?: string
  readonly pr?: PullRequestLink
  readonly waves: readonly (readonly string[])[]
  readonly tasks: readonly Task[]
  readonly task_results: readonly TaskExecutionResult[]
  readonly file_diff_stats?: readonly FileDiffStat[]
  readonly discovered_work?: readonly Amendment[]
  readonly pruning_proposals?: readonly PruningProposal[]
}

export interface CheckpointTwoPack {
  readonly kind: 'checkpoint-2'
  readonly run: string
  readonly integration_branch: string
  readonly integration_worktree: string | null
  readonly pr: PullRequestLink | null
  readonly summary: CheckpointTwoSummary
  readonly task_outcomes: readonly TaskOutcomeRow[]
  readonly diff_stats: DiffStats
  readonly discovered_work: readonly Amendment[]
  readonly pruning_proposals: readonly PruningProposal[]
  readonly no_verify_task_ids: readonly string[]
}
