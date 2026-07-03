import { describe, expect, it } from 'vitest'

import type { DesignLedger } from '../../../shared-kernel/design-ledger.js'
import {
  buildAdversarialCritiqueRounds,
  buildLockSpecMergeInputs,
  ledgerEntriesFromResolutions,
  resolveDesignVote,
  resolveDesignVotes,
  splitDesignLedger,
} from './index.js'
import type { DesignOption, DesignParticipant, DesignVote } from './types.js'

const participants: readonly DesignParticipant[] = [
  { id: 'codex' },
  { id: 'claude' },
  { id: 'judge' },
]

const apiFirst: DesignOption = {
  id: 'api-first',
  decision_id: 'transport',
  decision: 'Use the existing API transport.',
  rationale: 'It matches the current ports.',
  proposed_by: 'codex',
  task_refs: ['T1'],
  context_refs: ['ctx-api'],
  supersedes: ['old-transport'],
  content_hash: 'sha256:api',
}

const queueFirst: DesignOption = {
  id: 'queue-first',
  decision_id: 'transport',
  decision: 'Introduce a queue transport.',
  proposed_by: 'claude',
}

const jsonLedger: DesignOption = {
  id: 'json-ledger',
  decision_id: 'ledger',
  decision: 'Persist the design ledger as JSON.',
}

const options: readonly DesignOption[] = [apiFirst, queueFirst, jsonLedger]

describe('buildAdversarialCritiqueRounds', () => {
  it('assigns each participant to critique authored options without self-review', () => {
    const rounds = buildAdversarialCritiqueRounds(participants, options, 2)

    expect(rounds).toEqual([
      {
        round: 1,
        assignments: [
          {
            round: 1,
            decision_id: 'transport',
            option_id: 'api-first',
            reviewer_id: 'claude',
            subject_id: 'codex',
          },
          {
            round: 1,
            decision_id: 'transport',
            option_id: 'api-first',
            reviewer_id: 'judge',
            subject_id: 'codex',
          },
          {
            round: 1,
            decision_id: 'transport',
            option_id: 'queue-first',
            reviewer_id: 'codex',
            subject_id: 'claude',
          },
          {
            round: 1,
            decision_id: 'transport',
            option_id: 'queue-first',
            reviewer_id: 'judge',
            subject_id: 'claude',
          },
          {
            round: 1,
            decision_id: 'ledger',
            option_id: 'json-ledger',
            reviewer_id: 'codex',
            subject_id: 'claude',
          },
          {
            round: 1,
            decision_id: 'ledger',
            option_id: 'json-ledger',
            reviewer_id: 'claude',
            subject_id: 'judge',
          },
          {
            round: 1,
            decision_id: 'ledger',
            option_id: 'json-ledger',
            reviewer_id: 'judge',
            subject_id: 'codex',
          },
        ],
      },
      {
        round: 2,
        assignments: [
          {
            round: 2,
            decision_id: 'transport',
            option_id: 'api-first',
            reviewer_id: 'claude',
            subject_id: 'codex',
          },
          {
            round: 2,
            decision_id: 'transport',
            option_id: 'api-first',
            reviewer_id: 'judge',
            subject_id: 'codex',
          },
          {
            round: 2,
            decision_id: 'transport',
            option_id: 'queue-first',
            reviewer_id: 'codex',
            subject_id: 'claude',
          },
          {
            round: 2,
            decision_id: 'transport',
            option_id: 'queue-first',
            reviewer_id: 'judge',
            subject_id: 'claude',
          },
          {
            round: 2,
            decision_id: 'ledger',
            option_id: 'json-ledger',
            reviewer_id: 'codex',
            subject_id: 'judge',
          },
          {
            round: 2,
            decision_id: 'ledger',
            option_id: 'json-ledger',
            reviewer_id: 'claude',
            subject_id: 'codex',
          },
          {
            round: 2,
            decision_id: 'ledger',
            option_id: 'json-ledger',
            reviewer_id: 'judge',
            subject_id: 'claude',
          },
        ],
      },
    ])
  })

  it('requires multiple participants and a positive number of rounds', () => {
    expect(() => buildAdversarialCritiqueRounds(participants, options, 0)).toThrow(
      'roundCount must be a positive integer',
    )
    expect(() => buildAdversarialCritiqueRounds([{ id: 'solo' }], options, 1)).toThrow(
      'at least two participants are required',
    )
  })
})

describe('resolveDesignVote', () => {
  it('settles a decision when one option has a majority', () => {
    const resolution = resolveDesignVote('transport', options, [
      { voter_id: 'codex', decision_id: 'transport', option_id: 'api-first' },
      { voter_id: 'claude', decision_id: 'transport', option_id: 'api-first' },
      { voter_id: 'judge', decision_id: 'transport', option_id: 'queue-first' },
    ])

    expect(resolution).toMatchObject({
      decision_id: 'transport',
      status: 'settled',
      resolution: '2/3',
      support_count: 2,
      total_votes: 3,
      escalation_reasons: [],
      escalate_to_consolidator: false,
      winning_option_id: 'api-first',
      resolved_option: apiFirst,
    })
    expect(resolution.counts).toEqual([
      { option_id: 'api-first', votes: 2 },
      { option_id: 'queue-first', votes: 1 },
    ])
  })

  it('contests ties, flagged votes, flagged options, missing options, empty votes, and plurality-only results', () => {
    const tied = resolveDesignVote('transport', options, [
      { voter_id: 'codex', decision_id: 'transport', option_id: 'api-first' },
      { voter_id: 'claude', decision_id: 'transport', option_id: 'queue-first' },
    ])
    const flaggedVote = resolveDesignVote('transport', options, [
      { voter_id: 'codex', decision_id: 'transport', option_id: 'api-first', flagged: true },
      { voter_id: 'claude', decision_id: 'transport', option_id: 'api-first' },
      { voter_id: 'judge', decision_id: 'transport', option_id: 'queue-first' },
    ])
    const flaggedOption = resolveDesignVote(
      'ledger',
      [{ ...jsonLedger, flagged: true }],
      [{ voter_id: 'codex', decision_id: 'ledger', option_id: 'json-ledger' }],
    )
    const missingOption = resolveDesignVote('transport', options, [
      { voter_id: 'codex', decision_id: 'transport', option_id: 'api-first' },
      { voter_id: 'claude', decision_id: 'transport', option_id: 'unknown' },
    ])
    const empty = resolveDesignVote('ledger', options, [])
    const pluralityOnly = resolveDesignVote(
      'transport',
      [
        ...options.filter((option) => option.decision_id === 'transport'),
        { id: 'rpc-first', decision_id: 'transport', decision: 'Use RPC.' },
      ],
      [
        { voter_id: 'codex', decision_id: 'transport', option_id: 'api-first' },
        { voter_id: 'claude', decision_id: 'transport', option_id: 'api-first' },
        { voter_id: 'judge', decision_id: 'transport', option_id: 'queue-first' },
        { voter_id: 'other', decision_id: 'transport', option_id: 'rpc-first' },
      ],
    )

    expect(tied.escalation_reasons).toEqual(['tie'])
    expect(flaggedVote.escalation_reasons).toEqual(['flagged'])
    expect(flaggedOption.escalation_reasons).toEqual(['flagged'])
    expect(missingOption.escalation_reasons).toEqual(['missing-option', 'no-majority'])
    expect(missingOption.resolution).toBe('1/2')
    expect(missingOption.ignored_votes).toEqual([
      { voter_id: 'claude', decision_id: 'transport', option_id: 'unknown' },
    ])
    expect(empty.escalation_reasons).toEqual(['no-votes'])
    expect(pluralityOnly.escalation_reasons).toEqual(['no-majority'])
    for (const resolution of [tied, flaggedVote, flaggedOption, missingOption, empty, pluralityOnly]) {
      expect(resolution.status).toBe('contested')
      expect(resolution.escalate_to_consolidator).toBe(true)
    }
  })

  it('requires a decision with options', () => {
    expect(() => resolveDesignVote('missing', options, [])).toThrow('decision has no options: missing')
  })
})

describe('resolveDesignVotes', () => {
  it('resolves each distinct decision from the option set', () => {
    const votes: readonly DesignVote[] = [
      { voter_id: 'codex', decision_id: 'transport', option_id: 'api-first' },
      { voter_id: 'claude', decision_id: 'transport', option_id: 'api-first' },
      { voter_id: 'codex', decision_id: 'ledger', option_id: 'json-ledger' },
    ]

    expect(resolveDesignVotes(options, votes).map((resolution) => resolution.decision_id)).toEqual([
      'transport',
      'ledger',
    ])
  })
})

describe('ledger split and lock-spec merge inputs', () => {
  it('turns resolutions into settled and contested ledger entries', () => {
    const resolutions = resolveDesignVotes(options, [
      { voter_id: 'codex', decision_id: 'transport', option_id: 'api-first' },
      { voter_id: 'claude', decision_id: 'transport', option_id: 'api-first' },
      { voter_id: 'judge', decision_id: 'transport', option_id: 'queue-first' },
    ])
    const ledger: DesignLedger = { entries: ledgerEntriesFromResolutions(resolutions, options) }
    const split = splitDesignLedger(ledger)
    const mergeInputs = buildLockSpecMergeInputs(split)

    expect(split.settled.entries).toEqual([
      {
        id: 'api-first',
        decision: 'Use the existing API transport.',
        rationale: 'It matches the current ports.',
        status: 'settled',
        task_refs: ['T1'],
        context_refs: ['ctx-api'],
        supersedes: ['old-transport'],
        content_hash: 'sha256:api',
      },
    ])
    expect(split.contested.entries).toEqual([
      {
        id: 'json-ledger',
        decision: 'Persist the design ledger as JSON.',
        rationale: 'vote resolved 0/0',
        status: 'contested',
      },
    ])
    expect(mergeInputs).toEqual({
      settled_entries: split.settled.entries,
      contested_entries: split.contested.entries,
      consolidator_entry_ids: ['json-ledger'],
      can_lock_without_consolidator: false,
    })
  })

  it('treats undefined ledger entries as empty and permits locking fully settled ledgers', () => {
    const emptySplit = splitDesignLedger({})
    const settledOnlySplit = splitDesignLedger({
      entries: [{ id: 'done', decision: 'Ship it.', status: 'settled' }],
    })

    expect(emptySplit).toEqual({ settled: { entries: [] }, contested: { entries: [] } })
    expect(buildLockSpecMergeInputs(settledOnlySplit)).toEqual({
      settled_entries: [{ id: 'done', decision: 'Ship it.', status: 'settled' }],
      contested_entries: [],
      consolidator_entry_ids: [],
      can_lock_without_consolidator: true,
    })
  })

  it('rejects settled resolutions without a resolved option', () => {
    expect(() =>
      ledgerEntriesFromResolutions(
        [
          {
            decision_id: 'broken',
            status: 'settled',
            resolution: '1/1',
            support_count: 1,
            total_votes: 1,
            counts: [],
            escalation_reasons: [],
            escalate_to_consolidator: false,
            ignored_votes: [],
            winning_option_id: 'missing',
          },
        ],
        [],
      ),
    ).toThrow('settled decision has no resolved option: broken')
  })
})
