import { describe, expect, it } from 'vitest'
import type { Story, Task } from '../contracts/index.js'
import {
  buildStatusLabelTransition,
  chooseMilestone,
  findIssueByTaskMarker,
  issueStateAfterMirror,
  renderPullRequestBody,
  renderTaskIssueBody,
  selectBestFitLabels,
  taskMarker,
} from './index.js'

const task: Task = {
  id: 'T15',
  title: 'GitHub mirror state machine',
  objective: 'Build pure mirror payloads',
  output_format: 'TypeScript module',
  paths: ['council/ts/src/domain/github/index.ts'],
  depends_on: ['T1'],
  difficulty: 'moderate',
  model: 'sonnet',
  verify: 'npm test',
  boundaries: 'No gh calls',
  acceptance_criteria: ['reuses matching issues', 'never invents labels'],
  dev_notes: 'Keep pure.',
  spec_ref: '001-github-mirror',
  archetype: 'domain-builder',
  context_profile: 'thin',
  discovered_from: 'ck-survey',
  supersedes: ['T2'],
}

const story: Story = {
  title: 'Mirror tasks to GitHub',
  status: 'ready',
  goal: 'Represent council tasks as GitHub issues',
  user_value: {
    actor: 'a maintainer',
    capability: 'can inspect mirrored task state',
    outcome: 'work is coordinated in GitHub',
  },
  context: 'Story file content is the issue source of truth.',
  acceptance_criteria: ['issue body includes story fields'],
  scope: {
    in_scope: ['issue body assembly'],
    out_of_scope: ['gh subprocesses'],
  },
  implementation_notes: {
    files: ['council/ts/src/domain/github/index.ts'],
    patterns: ['pure functions'],
    dependencies: ['none'],
    data_config_migration: ['bootstrap labels externally'],
  },
  tests: {
    unit: ['domain builders'],
    integration: ['adapter contract'],
    manual_or_workflow: ['open generated issue'],
  },
  definition_of_done: ['100% line coverage'],
}

describe('github mirror domain', () => {
  it('reuses milestones by exact title and otherwise builds a create payload', () => {
    const milestones = [
      { number: 1, title: 'github mirror' },
      { number: 2, title: 'GitHub Mirror' },
    ]

    expect(chooseMilestone('GitHub Mirror', milestones)).toEqual({
      kind: 'reuse',
      title: 'GitHub Mirror',
      milestone: { number: 2, title: 'GitHub Mirror' },
    })
    expect(chooseMilestone('github Mirror', milestones)).toEqual({
      kind: 'create',
      title: 'github Mirror',
      create: { title: 'github Mirror' },
    })
  })

  it('finds existing issues by exact run/task marker and reports duplicates', () => {
    const marker = taskMarker('run-7', 'T15')
    const match = findIssueByTaskMarker('run-7', 'T15', [
      { number: 1, title: 'near miss', body: '<!-- council-task-id: run-7/T150 -->' },
      { number: 2, title: 'primary', body: marker },
      { number: 3, title: 'missing body' },
      { number: 4, title: 'duplicate', body: `prefix\n${marker}\nsuffix` },
    ])

    expect(match).toEqual({
      marker,
      issue: { number: 2, title: 'primary', body: marker },
      duplicates: [{ number: 4, title: 'duplicate', body: `prefix\n${marker}\nsuffix` }],
    })
    expect(findIssueByTaskMarker('run-7', 'T3', [])).toEqual({
      marker: taskMarker('run-7', 'T3'),
      duplicates: [],
    })
  })

  it('assembles task issue bodies with story content, typed edges, markers, and spec refs', () => {
    const body = renderTaskIssueBody({
      runId: 'run-7',
      task,
      story,
      edgeIssueNumbers: new Map([
        ['T1', 11],
        ['ck-survey', 12],
        ['T2', 13],
      ]),
    })

    expect(body).toContain('<!-- council-task-id: run-7/T15 -->')
    expect(body).toContain('# T15: GitHub mirror state machine')
    expect(body).toContain('Objective: Build pure mirror payloads')
    expect(body).toContain('Story acceptance criteria:\n- issue body includes story fields')
    expect(body).toContain('Blocked by #11')
    expect(body).toContain('discovered-from #12')
    expect(body).toContain('supersedes #13')
    expect(body).toContain('spec_ref: 001-github-mirror')
  })

  it('omits optional story, edge, list, and spec sections when no data is present', () => {
    const minimalTask: Task = {
      id: task.id,
      title: task.title,
      objective: task.objective,
      output_format: task.output_format,
      depends_on: [],
      difficulty: task.difficulty,
      model: task.model,
      paths: [],
      verify: task.verify,
      boundaries: task.boundaries,
    }

    expect(renderTaskIssueBody({ runId: 'run-8', task: minimalTask })).toBe(
      [
        '<!-- council-task-id: run-8/T15 -->',
        '',
        '# T15: GitHub mirror state machine',
        '',
        '## Task',
        '',
        'Objective: Build pure mirror payloads',
        'Output: TypeScript module',
        'Paths: none',
        'Verify: npm test',
        'Boundaries: No gh calls',
        '',
      ].join('\n'),
    )
  })

  it('renders unresolved typed edges by task id when issue numbers are not known yet', () => {
    const { spec_ref: removedSpecRef, ...taskWithoutSpec } = task
    const body = renderTaskIssueBody({
      runId: 'run-7',
      task: taskWithoutSpec,
      specRef: 'override-spec',
    })

    expect(removedSpecRef).toBe('001-github-mirror')
    expect(body).toContain('Blocked by T1')
    expect(body).toContain('discovered-from ck-survey')
    expect(body).toContain('supersedes T2')
    expect(body).toContain('spec_ref: override-spec')
  })

  it('selects only best-fit labels that already exist', () => {
    const labels = selectBestFitLabels({
      existingLabels: [
        'council/status:in-progress',
        'difficulty:moderate',
        'model:sonnet',
        'council/archetype:domain-builder',
        { name: 'area:council' },
        'manual',
      ],
      status: 'In Progress',
      preferred: ['manual', 'invented'],
      task,
    })

    expect(labels).toEqual([
      'manual',
      'council/status:in-progress',
      'difficulty:moderate',
      'model:sonnet',
      'council/archetype:domain-builder',
      'area:council',
    ])
  })

  it('returns no labels when neither status nor task candidates can be matched', () => {
    expect(selectBestFitLabels({ existingLabels: [], status: 'Blocked' })).toEqual([])
    expect(selectBestFitLabels({ existingLabels: ['other'] })).toEqual([])
  })

  it('transitions status labels using only the bootstrap-provided existing set', () => {
    expect(
      buildStatusLabelTransition(
        ['council/status:queued', { name: 'keep' }],
        ['council/status:in-progress'],
        'In Progress',
      ),
    ).toEqual({
      add: ['council/status:in-progress'],
      remove: ['council/status:queued'],
      labels: ['keep', 'council/status:in-progress'],
    })

    expect(
      buildStatusLabelTransition(
        ['council/status:in-progress', 'keep'],
        ['council/status:in-progress'],
        'In Progress',
      ),
    ).toEqual({
      add: [],
      remove: [],
      labels: ['council/status:in-progress', 'keep'],
    })

    expect(
      buildStatusLabelTransition(
        ['council/status:queued', 'keep'],
        ['unrelated'],
        'Blocked',
      ),
    ).toEqual({
      add: [],
      remove: ['council/status:queued'],
      labels: ['keep'],
    })
  })

  it('only closes mirrored issues after landed status is confirmed as landed', () => {
    expect(issueStateAfterMirror('landed', true)).toBe('closed')
    expect(issueStateAfterMirror('Landed', false)).toBe('open')
    expect(issueStateAfterMirror('merged', true)).toBe('open')
  })

  it('assembles pull request bodies with Closes and refs lines', () => {
    expect(
      renderPullRequestBody({
        summary: 'Implements the mirror domain.',
        closingIssueNumbers: [9, 9, 10],
        referenceIssueNumbers: [10, 11, 11],
        extraSections: ['Validation: npm test', ''],
      }),
    ).toBe(
      [
        'Implements the mirror domain.',
        '',
        'Closes #9',
        'Closes #10',
        'refs #11',
        '',
        'Validation: npm test',
        '',
      ].join('\n'),
    )
  })
})
