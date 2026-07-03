import { describe, expect, it } from 'vitest'

import type {
  Amendment,
  DesignLedgerEntry,
  ReviewVerdict,
  RoutingVerdict,
  Story,
  Task,
} from '../../../shared-kernel/index.js'
import {
  assembleCheckpointOnePack,
  assembleCheckpointTwoPack,
  assembleDesignCheckpointPack,
  buildDagByDepth,
  buildDiffStats,
  buildSectionIndex,
  splitDesignLedger,
} from './index.js'
import type { FileDiffStat, TaskExecutionResult } from './index.js'

const baseTask: Task = {
  id: 'T1',
  title: 'Build foundation',
  objective: 'Create the shared module',
  output_format: 'Files and tests',
  paths: ['src/foundation.ts'],
  depends_on: [],
  difficulty: 'moderate',
  model: 'sonnet',
  verify: 'npm test',
  boundaries: 'Only foundation files',
  acceptance_criteria: ['Exports the public API'],
}

const dependentTask: Task = {
  id: 'T2',
  title: 'Wire feature',
  objective: 'Create the shared module',
  output_format: 'Files and tests',
  paths: ['src/feature.ts'],
  depends_on: ['T1'],
  difficulty: 'trivial',
  model: 'haiku',
  verify: ' ',
  boundaries: 'Only foundation files',
}

const routingVerdict: RoutingVerdict = {
  route: 'fanout',
  reasons: 'Tasks are independent after the first depth.',
  candidate_routes: ['single-agent', 'fanout'],
}

const story: Story = {
  title: 'Operator reviews checkpoint',
  status: 'ready',
  goal: 'Approve parallel execution',
  user_value: {
    actor: 'operator',
    capability: 'see the task graph',
    outcome: 'start fanout confidently',
  },
  context: 'Council run',
  acceptance_criteria: ['Shows the graph'],
  scope: {
    in_scope: ['Review pack data'],
    out_of_scope: ['HTML rendering'],
  },
  implementation_notes: {
    files: ['council/ts/src/domain/reviewpack'],
    patterns: ['pure functions'],
    dependencies: [],
    data_config_migration: [],
  },
  tests: {
    unit: ['vitest'],
    integration: [],
    manual_or_workflow: [],
  },
  definition_of_done: ['Pack assembled'],
}

const grillVerdict: ReviewVerdict = {
  satisfied: false,
  reasons: 'Verify command is absent on T2.',
  issues: ['T2 has no automated verification'],
  task_id: 'T2',
  reviewer: 'grill',
}

describe('assembleCheckpointOnePack', () => {
  it('assembles routing verdicts, DAG depths, story cards, grill ledger, and spend', () => {
    const pack = assembleCheckpointOnePack({
      routing_verdict: routingVerdict,
      tasks: [dependentTask, baseTask],
      stories: [story],
      grill_verdicts: [grillVerdict],
      spend: {
        rounds: 1,
        fanout_verifier_calls_per_task: 2,
      },
    })

    expect(pack.kind).toBe('checkpoint-1')
    expect(pack.routing_verdicts).toEqual([routingVerdict])
    expect(pack.dag_by_depth.map((depth) => depth.task_ids)).toEqual([['T1'], ['T2']])
    expect(pack.dag_by_depth[1]?.tasks[0]).toMatchObject({
      id: 'T2',
      verify_present: false,
      acceptance_criteria: [],
    })
    expect(pack.story_cards).toEqual([
      {
        title: 'Operator reviews checkpoint',
        status: 'ready',
        goal: 'Approve parallel execution',
        actor: 'operator',
        capability: 'see the task graph',
        outcome: 'start fanout confidently',
        acceptance_criteria: ['Shows the graph'],
        in_scope: ['Review pack data'],
        out_of_scope: ['HTML rendering'],
      },
    ])
    expect(pack.grill_ledger).toEqual([
      {
        task_id: 'T2',
        reviewer: 'grill',
        satisfied: false,
        reasons: 'Verify command is absent on T2.',
        issues: ['T2 has no automated verification'],
      },
    ])
    expect(pack.spend_estimate).toEqual({
      plan_model_calls: 7,
      fanout_worker_calls: 2,
      fanout_verifier_calls: 4,
      total_model_calls: 13,
      task_count: 2,
      wave_count: 2,
    })
  })

  it('accepts existing routing verdict arrays and default optional inputs', () => {
    const pack = assembleCheckpointOnePack({
      routing_verdict: [routingVerdict],
      tasks: [baseTask],
    })

    expect(pack.routing_verdicts).toEqual([routingVerdict])
    expect(pack.story_cards).toEqual([])
    expect(pack.grill_ledger).toEqual([])
    expect(pack.spend_estimate.total_model_calls).toBe(13)
  })

  it('defaults missing grill verdict metadata to null', () => {
    const pack = assembleCheckpointOnePack({
      routing_verdict: routingVerdict,
      tasks: [baseTask],
      grill_verdicts: [
        {
          satisfied: true,
          reasons: 'No issues.',
          issues: [],
        },
      ],
    })

    expect(pack.grill_ledger).toEqual([
      {
        task_id: null,
        reviewer: null,
        satisfied: true,
        reasons: 'No issues.',
        issues: [],
      },
    ])
  })
})

describe('buildDagByDepth', () => {
  it('sorts ready tasks by id within each depth', () => {
    expect(
      buildDagByDepth([
        {
          ...baseTask,
          id: 'T2',
        },
        {
          ...baseTask,
          id: 'T1',
        },
      ]).map((depth) => depth.task_ids),
    ).toEqual([['T1', 'T2']])
  })

  it('rejects invalid task graphs', () => {
    expect(() => buildDagByDepth([baseTask, { ...baseTask }])).toThrow('duplicate task id: T1')
    expect(() =>
      buildDagByDepth([
        {
          ...baseTask,
          depends_on: ['T2'],
        },
      ]),
    ).toThrow('depends on unknown task T2')
    expect(() =>
      buildDagByDepth([
        {
          ...baseTask,
          depends_on: ['T2'],
        },
        {
          ...dependentTask,
          depends_on: ['T1'],
        },
      ]),
    ).toThrow('dependency cycle among tasks: T1, T2')
  })
})

describe('assembleDesignCheckpointPack', () => {
  it('locks the spec, splits the ledger, builds votes, and exposes the section index', () => {
    const ledgerEntries: readonly DesignLedgerEntry[] = [
      {
        id: 'D2',
        decision: 'Keep renderer separate',
        status: 'contested',
      },
      {
        id: 'D0',
        decision: 'Document the checkpoint contract',
        status: 'approved',
      },
      {
        id: 'D1',
        decision: 'Use pure assembly functions',
        status: 'settled',
      },
    ]

    const pack = assembleDesignCheckpointPack({
      locked_spec: {
        title: 'Review pack spec',
        markdown: '# Review Pack\n\n## Data Model\n\n### Checkpoint 1\n',
        spec_ref: 'specs/001-review-pack/spec.md',
        locked_by: 'design-checkpoint',
      },
      ledger_entries: ledgerEntries,
      votes: [
        {
          voter: 'zoe',
          vote: 'approve',
          option: 'A',
          rationale: 'Smallest scope',
        },
        {
          voter: 'amy',
          vote: 'approve',
          option: 'A',
          conditions: ['Keep rendering out'],
        },
        {
          voter: 'max',
          vote: 'reject',
          option: 'B',
        },
        {
          voter: 'amy',
          vote: 'approve',
          option: 'B',
        },
      ],
    })

    expect(pack.kind).toBe('design-checkpoint')
    expect(pack.locked_spec).toMatchObject({
      title: 'Review pack spec',
      spec_ref: 'specs/001-review-pack/spec.md',
      locked_by: 'design-checkpoint',
    })
    expect(pack.section_index).toEqual([
      {
        section_ref: '§1',
        level: 1,
        title: 'Review Pack',
        anchor: 'review-pack',
        line: 1,
      },
      {
        section_ref: '§2',
        level: 2,
        title: 'Data Model',
        anchor: 'data-model',
        line: 3,
      },
      {
        section_ref: '§3',
        level: 3,
        title: 'Checkpoint 1',
        anchor: 'checkpoint-1',
        line: 5,
      },
    ])
    expect(pack.ledger.settled.map((entry) => entry.id)).toEqual(['D0', 'D1'])
    expect(pack.ledger.contested.map((entry) => entry.id)).toEqual(['D2'])
    expect(pack.vote_table.rows).toEqual([
      {
        voter: 'amy',
        vote: 'approve',
        option: 'A',
        rationale: null,
        conditions: ['Keep rendering out'],
      },
      {
        voter: 'amy',
        vote: 'approve',
        option: 'B',
        rationale: null,
        conditions: [],
      },
      {
        voter: 'max',
        vote: 'reject',
        option: 'B',
        rationale: null,
        conditions: [],
      },
      {
        voter: 'zoe',
        vote: 'approve',
        option: 'A',
        rationale: 'Smallest scope',
        conditions: [],
      },
    ])
    expect(pack.vote_table.counts).toEqual([
      {
        option: 'A',
        count: 2,
      },
      {
        option: 'B',
        count: 2,
      },
    ])
  })

  it('handles unlocked optional spec metadata and headings without matches', () => {
    const pack = assembleDesignCheckpointPack({
      locked_spec: {
        title: 'No headings',
        markdown: 'plain text',
      },
      ledger_entries: [],
      votes: [],
    })

    expect(pack.locked_spec.spec_ref).toBeNull()
    expect(pack.locked_spec.locked_by).toBeNull()
    expect(pack.section_index).toEqual([])
  })
})

describe('design helpers', () => {
  it('indexes closed headings and treats missing ledger statuses as contested', () => {
    expect(buildSectionIndex('#### Deep Heading ####')).toEqual([
      {
        section_ref: '§1',
        level: 4,
        title: 'Deep Heading',
        anchor: 'deep-heading',
        line: 1,
      },
    ])
    expect(
      splitDesignLedger([
        {
          id: 'D1',
          decision: 'Unreviewed',
        },
      ]),
    ).toEqual({
      settled: [],
      contested: [
        {
          id: 'D1',
          decision: 'Unreviewed',
        },
      ],
    })
  })
})

describe('assembleCheckpointTwoPack', () => {
  it('assembles task outcomes, diff stats, discovered work, pruning proposals, and PR link', () => {
    const discoveredWork: Amendment = {
      id: 'A2',
      summary: 'Add follow-up validation',
      status: 'proposed',
    }
    const earlierDiscoveredWork: Amendment = {
      id: 'A1',
      summary: 'Record review pack provenance',
      status: 'proposed',
    }
    const results: readonly TaskExecutionResult[] = [
      {
        task_id: 'T2',
        status: 'verify-failed',
        merge: 'nothing-to-merge',
        model: 'codex:gpt-5',
        files_changed: ['src/feature.ts'],
        verify_rc: 1,
        verdict: null,
        out_of_bounds: ['src/other.ts'],
        branch: 'council/run/T2',
      },
      {
        task_id: 'T1',
        status: 'ok',
        merge: 'ok',
        model: 'codex:gpt-5',
        files_changed: ['src/foundation.ts'],
        verify_rc: 0,
        verdict: {
          satisfied: true,
          reasons: 'Looks complete',
          issues: [],
        },
        out_of_bounds: [],
        branch: 'council/run/T1',
      },
    ]
    const diffStats: readonly FileDiffStat[] = [
      {
        path: 'src/feature.ts',
        additions: 3,
        deletions: 1,
      },
      {
        path: 'src/foundation.ts',
        additions: 10,
        deletions: 0,
      },
    ]

    const pack = assembleCheckpointTwoPack({
      run: '20260702-reviewpack',
      integration_branch: 'council/run/integration',
      integration_worktree: '/tmp/council/run/_integration',
      pr: {
        url: 'https://github.example/pr/12',
        number: 12,
        title: 'Review pack',
      },
      waves: [['T1'], ['T2']],
      tasks: [baseTask, dependentTask],
      task_results: results,
      file_diff_stats: diffStats,
      discovered_work: [discoveredWork, earlierDiscoveredWork],
      pruning_proposals: [
        {
          id: 'P2',
          title: 'Keep lock files',
          recommendation: 'Do not delete',
          delete_now: false,
        },
        {
          id: 'P1',
          title: 'Merge duplicate docs',
          recommendation: 'Consolidate',
          delete_now: false,
          files: ['docs/a.md'],
        },
      ],
    })

    expect(pack.kind).toBe('checkpoint-2')
    expect(pack.summary).toEqual({
      total: 2,
      ok: 1,
      failed: 1,
      merged: 1,
      conflicts: 0,
      no_verify: 1,
    })
    expect(pack.task_outcomes).toEqual([
      {
        task_id: 'T1',
        title: 'Build foundation',
        status: 'ok',
        merge: 'ok',
        model: 'codex:gpt-5',
        files_changed_count: 1,
        verify_rc: 0,
        verifier_satisfied: true,
        out_of_bounds: [],
        branch: 'council/run/T1',
        good: true,
        has_verify: true,
      },
      {
        task_id: 'T2',
        title: 'Wire feature',
        status: 'verify-failed',
        merge: 'nothing-to-merge',
        model: 'codex:gpt-5',
        files_changed_count: 1,
        verify_rc: 1,
        verifier_satisfied: null,
        out_of_bounds: ['src/other.ts'],
        branch: 'council/run/T2',
        good: false,
        has_verify: false,
      },
    ])
    expect(pack.diff_stats).toEqual({
      files_changed: 2,
      additions: 13,
      deletions: 1,
      by_file: [
        {
          path: 'src/feature.ts',
          additions: 3,
          deletions: 1,
        },
        {
          path: 'src/foundation.ts',
          additions: 10,
          deletions: 0,
        },
      ],
    })
    expect(pack.discovered_work.map((amendment) => amendment.id)).toEqual(['A1', 'A2'])
    expect(pack.pruning_proposals.map((proposal) => proposal.id)).toEqual(['P1', 'P2'])
    expect(pack.no_verify_task_ids).toEqual(['T2'])
    expect(pack.pr?.number).toBe(12)
  })

  it('uses null defaults and derives zero diff stats from changed files', () => {
    const pack = assembleCheckpointTwoPack({
      run: 'run',
      integration_branch: 'branch',
      waves: [[]],
      tasks: [],
      task_results: [
        {
          task_id: 'T9',
          status: 'ok',
          files_changed: ['b.ts', 'a.ts', 'b.ts'],
        },
        {
          task_id: 'T8',
          status: 'ok',
          merge: 'conflict',
        },
      ],
    })

    expect(pack.integration_worktree).toBeNull()
    expect(pack.pr).toBeNull()
    expect(pack.summary).toEqual({
      total: 2,
      ok: 2,
      failed: 0,
      merged: 0,
      conflicts: 1,
      no_verify: 2,
    })
    expect(pack.task_outcomes.map((row) => [row.task_id, row.good])).toEqual([
      ['T8', false],
      ['T9', true],
    ])
    expect(pack.diff_stats).toEqual({
      files_changed: 2,
      additions: 0,
      deletions: 0,
      by_file: [
        {
          path: 'a.ts',
          additions: 0,
          deletions: 0,
        },
        {
          path: 'b.ts',
          additions: 0,
          deletions: 0,
        },
      ],
    })
    expect(pack.discovered_work).toEqual([])
    expect(pack.pruning_proposals).toEqual([])
  })
})

describe('buildDiffStats', () => {
  it('sorts supplied stats and sums totals', () => {
    expect(
      buildDiffStats([], [
        {
          path: 'z.ts',
          additions: 1,
          deletions: 2,
        },
        {
          path: 'a.ts',
          additions: 3,
          deletions: 4,
        },
      ]),
    ).toEqual({
      files_changed: 2,
      additions: 4,
      deletions: 6,
      by_file: [
        {
          path: 'a.ts',
          additions: 3,
          deletions: 4,
        },
        {
          path: 'z.ts',
          additions: 1,
          deletions: 2,
        },
      ],
    })
  })
})
