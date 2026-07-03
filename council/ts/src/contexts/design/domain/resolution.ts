import type { DesignLedger, DesignLedgerEntry } from '../../../domain/contracts/design-ledger.js'
import type {
  DesignCritiqueAssignment,
  DesignCritiqueRound,
  DesignLedgerSplit,
  DesignOption,
  DesignParticipant,
  DesignVote,
  DesignVoteCount,
  DesignVoteResolution,
  EscalationReason,
  LockSpecMergeInputs,
} from './types.js'

export function buildAdversarialCritiqueRounds(
  participants: readonly DesignParticipant[],
  options: readonly DesignOption[],
  roundCount: number,
): readonly DesignCritiqueRound[] {
  assertPositiveInteger(roundCount, 'roundCount')
  if (participants.length < 2) {
    throw new Error('at least two participants are required')
  }

  return Array.from({ length: roundCount }, (_, index) => {
    const round = index + 1

    return {
      round,
      assignments: options.flatMap((option) => assignmentsForOption(participants, option, round)),
    }
  })
}

export function resolveDesignVote(
  decisionId: string,
  options: readonly DesignOption[],
  votes: readonly DesignVote[],
): DesignVoteResolution {
  const decisionOptions = options.filter((option) => option.decision_id === decisionId)
  if (decisionOptions.length === 0) {
    throw new Error(`decision has no options: ${decisionId}`)
  }

  const optionIds = new Set(decisionOptions.map((option) => option.id))
  const relevantVotes = votes.filter((vote) => vote.decision_id === decisionId)
  const validVotes = relevantVotes.filter((vote) => optionIds.has(vote.option_id))
  const ignoredVotes = relevantVotes.filter((vote) => !optionIds.has(vote.option_id))
  const counts = countVotes(decisionOptions, validVotes)
  const winner = counts.reduce((best, count) => (best.votes >= count.votes ? best : count))

  const supportCount = winner.votes
  const totalVotes = relevantVotes.length
  const reasons = escalationReasons(
    decisionOptions,
    validVotes,
    ignoredVotes,
    counts,
    supportCount,
    validVotes.length,
    totalVotes,
  )
  const escalates = reasons.length > 0
  const resolvedOption = decisionOptions.reduce((selected, option) =>
    option.id === winner.option_id ? option : selected,
  )
  const resolution = formatResolution(supportCount, totalVotes)

  const base = {
    decision_id: decisionId,
    resolution,
    support_count: supportCount,
    total_votes: totalVotes,
    counts,
    escalation_reasons: reasons,
    escalate_to_consolidator: escalates,
    ignored_votes: ignoredVotes,
    winning_option_id: winner.option_id,
  }

  if (escalates) {
    return {
      ...base,
      status: 'contested',
    }
  }

  return {
    ...base,
    status: 'settled',
    resolved_option: resolvedOption,
  }
}

export function resolveDesignVotes(
  options: readonly DesignOption[],
  votes: readonly DesignVote[],
): readonly DesignVoteResolution[] {
  return [...new Set(options.map((option) => option.decision_id))].map((decisionId) =>
    resolveDesignVote(decisionId, options, votes),
  )
}

export function splitDesignLedger(ledger: DesignLedger): DesignLedgerSplit {
  const entries = ledger.entries ?? []
  return {
    settled: { entries: entries.filter((entry) => entry.status === 'settled') },
    contested: { entries: entries.filter((entry) => entry.status !== 'settled') },
  }
}

export function ledgerEntriesFromResolutions(
  resolutions: readonly DesignVoteResolution[],
  options: readonly DesignOption[],
): readonly DesignLedgerEntry[] {
  return resolutions.flatMap((resolution) => entriesForResolution(resolution, options))
}

export function buildLockSpecMergeInputs(split: DesignLedgerSplit): LockSpecMergeInputs {
  const settledEntries = split.settled.entries ?? []
  const contestedEntries = split.contested.entries ?? []

  return {
    settled_entries: settledEntries,
    contested_entries: contestedEntries,
    consolidator_entry_ids: contestedEntries.map((entry) => entry.id),
    can_lock_without_consolidator: contestedEntries.length === 0,
  }
}

function assignmentsForOption(
  participants: readonly DesignParticipant[],
  option: DesignOption,
  round: number,
): readonly DesignCritiqueAssignment[] {
  return participants.flatMap((reviewer, reviewerIndex) => {
    const subjectId = option.proposed_by ?? rotatingSubjectId(participants, reviewerIndex, round)
    if (subjectId === reviewer.id) {
      return []
    }

    return [
      {
        round,
        decision_id: option.decision_id,
        option_id: option.id,
        reviewer_id: reviewer.id,
        subject_id: subjectId,
      },
    ]
  })
}

function rotatingSubjectId(participants: readonly DesignParticipant[], reviewerIndex: number, round: number): string {
  const subjectIndex = (reviewerIndex + round) % participants.length
  const subject = participants.reduce((selected, participant, index) =>
    index === subjectIndex ? participant : selected,
  )
  return subject.id
}

function countVotes(options: readonly DesignOption[], votes: readonly DesignVote[]): readonly DesignVoteCount[] {
  const initialCounts = new Map(options.map((option) => [option.id, 0]))
  for (const vote of votes) {
    initialCounts.set(vote.option_id, (initialCounts.get(vote.option_id) ?? 0) + 1)
  }

  return options
    .map((option) => ({ option_id: option.id, votes: initialCounts.get(option.id) ?? 0 }))
    .sort((left, right) => right.votes - left.votes)
}

function escalationReasons(
  options: readonly DesignOption[],
  votes: readonly DesignVote[],
  ignoredVotes: readonly DesignVote[],
  counts: readonly DesignVoteCount[],
  supportCount: number,
  validVoteCount: number,
  totalVotes: number,
): readonly EscalationReason[] {
  const reasons: EscalationReason[] = []

  if (options.some((option) => option.flagged === true) || votes.some((vote) => vote.flagged === true)) {
    reasons.push('flagged')
  }

  if (ignoredVotes.length > 0) {
    reasons.push('missing-option')
  }

  if (validVoteCount === 0) {
    reasons.push('no-votes')
  } else if (hasTie(counts, supportCount)) {
    reasons.push('tie')
  } else if (supportCount < Math.floor(totalVotes / 2) + 1) {
    reasons.push('no-majority')
  }

  return reasons
}

function hasTie(counts: readonly DesignVoteCount[], supportCount: number): boolean {
  const next = counts[1]
  return next?.votes === supportCount
}

function formatResolution(supportCount: number, totalVotes: number): `${number}/${number}` {
  return `${String(supportCount)}/${String(totalVotes)}` as `${number}/${number}`
}

function entriesForResolution(
  resolution: DesignVoteResolution,
  options: readonly DesignOption[],
): readonly DesignLedgerEntry[] {
  if (resolution.status === 'settled') {
    const option = resolution.resolved_option
    if (option === undefined) {
      throw new Error(`settled decision has no resolved option: ${resolution.decision_id}`)
    }

    return [entryFromOption(option, 'settled', resolution.resolution)]
  }

  return options
    .filter((option) => option.decision_id === resolution.decision_id)
    .map((option) => entryFromOption(option, 'contested', resolution.resolution))
}

function entryFromOption(option: DesignOption, status: 'settled' | 'contested', resolution: string): DesignLedgerEntry {
  return {
    id: option.id,
    decision: option.decision,
    rationale: option.rationale ?? `vote resolved ${resolution}`,
    status,
    ...(option.task_refs === undefined ? {} : { task_refs: option.task_refs }),
    ...(option.context_refs === undefined ? {} : { context_refs: option.context_refs }),
    ...(option.supersedes === undefined ? {} : { supersedes: option.supersedes }),
    ...(option.content_hash === undefined ? {} : { content_hash: option.content_hash }),
  }
}

function assertPositiveInteger(value: number, name: string): void {
  if (!Number.isInteger(value) || value < 1) {
    throw new Error(`${name} must be a positive integer`)
  }
}
