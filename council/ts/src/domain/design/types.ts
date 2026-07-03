import type { DesignLedger, DesignLedgerEntry } from '../contracts/design-ledger.js'

export type DesignDecisionStatus = 'settled' | 'contested'

export type EscalationReason = 'flagged' | 'tie' | 'no-majority' | 'no-votes' | 'missing-option'

export interface DesignParticipant {
  readonly id: string
  readonly label?: string
}

export interface DesignOption {
  readonly id: string
  readonly decision_id: string
  readonly decision: string
  readonly rationale?: string
  readonly proposed_by?: string
  readonly flagged?: boolean
  readonly task_refs?: readonly string[]
  readonly context_refs?: readonly string[]
  readonly supersedes?: readonly string[]
  readonly content_hash?: string
}

export interface DesignCritiqueAssignment {
  readonly round: number
  readonly decision_id: string
  readonly option_id: string
  readonly reviewer_id: string
  readonly subject_id: string
}

export interface DesignCritiqueRound {
  readonly round: number
  readonly assignments: readonly DesignCritiqueAssignment[]
}

export interface DesignVote {
  readonly voter_id: string
  readonly decision_id: string
  readonly option_id: string
  readonly rationale?: string
  readonly flagged?: boolean
}

export interface DesignVoteCount {
  readonly option_id: string
  readonly votes: number
}

export interface DesignVoteResolution {
  readonly decision_id: string
  readonly status: DesignDecisionStatus
  readonly resolution: `${number}/${number}`
  readonly support_count: number
  readonly total_votes: number
  readonly counts: readonly DesignVoteCount[]
  readonly escalation_reasons: readonly EscalationReason[]
  readonly escalate_to_consolidator: boolean
  readonly ignored_votes: readonly DesignVote[]
  readonly winning_option_id?: string
  readonly resolved_option?: DesignOption
}

export interface DesignLedgerSplit {
  readonly settled: DesignLedger
  readonly contested: DesignLedger
}

export interface LockSpecMergeInputs {
  readonly settled_entries: readonly DesignLedgerEntry[]
  readonly contested_entries: readonly DesignLedgerEntry[]
  readonly consolidator_entry_ids: readonly string[]
  readonly can_lock_without_consolidator: boolean
}
