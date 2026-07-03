import { describe, expect, it } from 'vitest'

import {
  DEFAULT_REVIEWERS,
  advisoryFinding,
  blockingFinding,
  buildDuplicatedCodeChecklist,
  buildSpecAdherenceLens,
  planReviewStrategy,
  reviewerCountForDifficulty,
  selectReviewers,
  triageReviewFindings,
} from './index.js'
import type { LockedSpecSection, ReviewStrategyInput } from './index.js'

const baseTask: ReviewStrategyInput['task'] = {
  acceptance_criteria: ['task behavior is covered'],
  difficulty: 'hard',
  paths: ['src/service.ts'],
  verify: 'npm test',
}

const baseStory: ReviewStrategyInput['story'] = {
  acceptance_criteria: ['story behavior is preserved'],
}

describe('reviewer findings', () => {
  it('constructs blocking and advisory findings with optional AC references', () => {
    expect(blockingFinding('Missing test', 'diff lacks test', 'AC-1')).toEqual({
      severity: 'blocking',
      claim: 'Missing test',
      evidence: 'diff lacks test',
      ac_ref: 'AC-1',
    })
    expect(advisoryFinding('Rename helper', 'name is vague')).toEqual({
      severity: 'advisory',
      claim: 'Rename helper',
      evidence: 'name is vague',
    })
  })
})

describe('reviewer selection and strategy', () => {
  it('scales reviewer count by task difficulty', () => {
    expect(reviewerCountForDifficulty('trivial')).toBe(1)
    expect(reviewerCountForDifficulty('moderate')).toBe(2)
    expect(reviewerCountForDifficulty('hard')).toBe(3)
    expect(selectReviewers('moderate', ['a', 'b', 'c'])).toEqual(['a', 'b'])
  })

  it('rejects a council reviewer pool that is too small', () => {
    expect(() => selectReviewers('hard', ['only-one'])).toThrow(
      'reviewer pool must contain at least 3 reviewer(s)',
    )
  })

  it('preserves the legacy single-verifier path when review council is disabled', () => {
    expect(
      planReviewStrategy({
        review: { council: false },
        task: baseTask,
        story: baseStory,
        legacyVerifier: 'existing-verifier',
      }),
    ).toEqual({
      mode: 'legacy-single-verifier',
      reviewer: 'existing-verifier',
      verify: 'npm test',
    })
    expect(planReviewStrategy({ task: baseTask, story: baseStory })).toEqual({
      mode: 'legacy-single-verifier',
      reviewer: 'legacy-single-verifier',
      verify: 'npm test',
    })
  })

  it('creates the council path with difficulty-scaled reviewers and review lenses', () => {
    const strategy = planReviewStrategy({
      review: { council: true },
      task: { ...baseTask, difficulty: 'trivial' },
      story: baseStory,
      lockedSpecSections: [{ section: '2.1', requirement: 'do not change schema' }],
    })

    expect(strategy).toEqual({
      mode: 'review-council',
      reviewers: [DEFAULT_REVIEWERS[0]],
      checklist: [
        {
          id: 'story-ac-1',
          lens: 'spec-adherence',
          prompt: 'Compare the diff to story AC 1: story behavior is preserved',
          ref: 'story.ac[1]',
        },
        {
          id: 'task-ac-1',
          lens: 'spec-adherence',
          prompt: 'Compare the diff to task AC 1: task behavior is covered',
          ref: 'task.ac[1]',
        },
        {
          id: 'locked-spec-2.1',
          lens: 'spec-adherence',
          prompt: 'Compare the diff to locked-spec §2.1: do not change schema',
          ref: 'locked-spec §2.1',
        },
        {
          id: 'duplicated-code-search',
          lens: 'duplicated-code',
          prompt: 'Check src/service.ts for copied logic, repeated conditionals, or duplicate data tables.',
        },
        {
          id: 'duplicated-code-resolution',
          lens: 'duplicated-code',
          prompt:
            'Require a blocking finding when duplicated production code should be shared instead.',
        },
      ],
    })
  })
})

describe('review lenses', () => {
  it('compares the diff against story ACs, task ACs, and cited locked-spec sections', () => {
    const lockedSpecSections: readonly LockedSpecSection[] = [
      { section: '1', requirement: 'keep audit trail', title: 'Audit' },
    ]

    expect(
      buildSpecAdherenceLens({
        storyAcceptanceCriteria: ['story AC'],
        taskAcceptanceCriteria: ['task AC'],
        lockedSpecSections,
      }),
    ).toEqual([
      {
        id: 'story-ac-1',
        lens: 'spec-adherence',
        prompt: 'Compare the diff to story AC 1: story AC',
        ref: 'story.ac[1]',
      },
      {
        id: 'task-ac-1',
        lens: 'spec-adherence',
        prompt: 'Compare the diff to task AC 1: task AC',
        ref: 'task.ac[1]',
      },
      {
        id: 'locked-spec-1',
        lens: 'spec-adherence',
        prompt: 'Compare the diff to locked-spec §1: keep audit trail',
        ref: 'locked-spec §1',
      },
    ])
  })

  it('builds a duplicated-code checklist for the whole diff when paths are unknown', () => {
    expect(buildDuplicatedCodeChecklist()).toEqual([
      {
        id: 'duplicated-code-search',
        lens: 'duplicated-code',
        prompt:
          'Check the diff for copied logic, repeated conditionals, or duplicate data tables.',
      },
      {
        id: 'duplicated-code-resolution',
        lens: 'duplicated-code',
        prompt:
          'Require a blocking finding when duplicated production code should be shared instead.',
      },
    ])
  })
})

describe('triageReviewFindings', () => {
  it('dedupes, merges, refutes, and bounds blocking fixes', () => {
    const verdict = triageReviewFindings({
      maxFixes: 1.8,
      refutations: [{ claim: 'obsolete complaint', reason: 'diff already changed' }],
      reports: [
        {
          reviewer: 'spec',
          findings: [
            blockingFinding('Missing AC coverage', 'AC assertion absent', 'AC-1'),
            advisoryFinding('Obsolete complaint', 'old diff hunk'),
          ],
        },
        {
          reviewer: 'tests',
          findings: [
            advisoryFinding('  missing   ac coverage ', 'test does not assert AC', 'AC-1'),
            blockingFinding('Second blocker', 'another issue'),
          ],
        },
      ],
    })

    expect(verdict).toEqual({
      decision: 'changes-requested',
      findings: [
        {
          severity: 'blocking',
          claim: 'Missing AC coverage',
          evidence: ['AC assertion absent', 'test does not assert AC'],
          reviewers: ['spec', 'tests'],
          ac_ref: 'AC-1',
        },
        {
          severity: 'blocking',
          claim: 'Second blocker',
          evidence: ['another issue'],
          reviewers: ['tests'],
        },
      ],
      refuted_findings: [
        {
          severity: 'advisory',
          claim: 'Obsolete complaint',
          evidence: 'old diff hunk',
        },
      ],
      fix_list: [
        {
          claim: 'Missing AC coverage',
          evidence: ['AC assertion absent', 'test does not assert AC'],
          reviewers: ['spec', 'tests'],
          ac_ref: 'AC-1',
        },
      ],
    })
  })

  it('approves when only advisory findings remain and supports zero allowed fixes', () => {
    const verdict = triageReviewFindings({
      maxFixes: -1,
      refutations: [{ claim: 'missing lock', reason: 'wrong AC', ac_ref: 'AC-2' }],
      reports: [
        {
          reviewer: 'impl',
          findings: [
            advisoryFinding('Use clearer name', 'helper name is generic'),
            blockingFinding('Missing lock', 'AC-1 still lacks guard', 'AC-1'),
          ],
        },
      ],
    })

    expect(verdict.decision).toBe('changes-requested')
    expect(verdict.refuted_findings).toEqual([])
    expect(verdict.fix_list).toEqual([])
    expect(verdict.findings).toEqual([
      {
        severity: 'advisory',
        claim: 'Use clearer name',
        evidence: ['helper name is generic'],
        reviewers: ['impl'],
      },
      {
        severity: 'blocking',
        claim: 'Missing lock',
        evidence: ['AC-1 still lacks guard'],
        reviewers: ['impl'],
        ac_ref: 'AC-1',
      },
    ])
  })

  it('approves with an empty fix list after a matching AC-specific refutation', () => {
    expect(
      triageReviewFindings({
        refutations: [{ claim: 'Missing lock', reason: 'guard added', ac_ref: 'AC-1' }],
        reports: [
          {
            reviewer: 'impl',
            findings: [blockingFinding('Missing lock', 'AC-1 still lacks guard', 'AC-1')],
          },
        ],
      }),
    ).toEqual({
      decision: 'approve',
      findings: [],
      refuted_findings: [
        {
          severity: 'blocking',
          claim: 'Missing lock',
          evidence: 'AC-1 still lacks guard',
          ac_ref: 'AC-1',
        },
      ],
      fix_list: [],
    })
  })

  it('serializes a bounded fix that has no AC reference', () => {
    expect(
      triageReviewFindings({
        reports: [
          {
            reviewer: 'duplication',
            findings: [blockingFinding('Duplicate parser', 'same parser exists elsewhere')],
          },
        ],
      }).fix_list,
    ).toEqual([
      {
        claim: 'Duplicate parser',
        evidence: ['same parser exists elsewhere'],
        reviewers: ['duplication'],
      },
    ])
  })
})
