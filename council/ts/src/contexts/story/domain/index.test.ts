import { describe, expect, it } from 'vitest'

import type { Task } from '../../../shared-kernel/task.js'
import { renderStoryMarkdown, validateStoryReadiness } from './index.js'

const BASE_TASK: Task = {
  id: 'T14',
  title: 'Render story shards',
  objective: 'Render task stories from cited spec sections and context fragments',
  output_format: 'story.md',
  paths: ['council/ts/src/domain/story'],
  depends_on: [],
  difficulty: 'moderate',
  model: 'haiku',
  verify: 'npm test',
  boundaries: 'Only story domain files',
  acceptance_criteria: ['Only cited spec sections are included', 'Only cited context snippets are included'],
  dev_notes: 'Keep story template prose out of domain logic.',
  spec_ref: 'spec.md#1, spec.md#3',
  context_refs: ['ctx-a', 'ctx-c'],
  engine: { cli: 'codex', model: 'gpt-5', label: 'Codex' },
}

function requiredTask(paths: readonly string[] = ['council/ts/src/domain/story']): Task {
  return {
    id: 'T14',
    title: 'Render story shards',
    objective: 'Render task stories from cited spec sections and context fragments',
    output_format: 'story.md',
    paths,
    depends_on: [],
    difficulty: 'moderate',
    model: 'haiku',
    verify: 'npm test',
    boundaries: 'Only story domain files',
    engine: { cli: 'codex', model: 'gpt-5', label: 'Codex' },
  }
}

describe('renderStoryMarkdown', () => {
  it('renders story.md sections while sharding spec and context to cited refs only', () => {
    const story = renderStoryMarkdown({
      task: BASE_TASK,
      specSections: [
        { ref: 'spec.md#1', title: 'Scope', text: 'Include this spec shard.' },
        { ref: 'spec.md#2', title: 'Skipped', text: 'Do not include the whole spec.' },
        { ref: 'spec.md#3', text: 'Include this unlabeled shard.' },
      ],
      contextPack: {
        summary: 'Context pack summary is allowed only with cited snippets.',
        snippets: [
          { ref: 'ctx-a', path: 'a.ts', text: 'Cited context A.' },
          { ref: 'ctx-b', path: 'b.ts', text: 'Uncited context B.' },
          { ref: 'ctx-c', text: 'Cited context C.' },
        ],
      },
      structureNotes: [
        { path: 'council/ts/src/domain/story', note: 'Pure domain logic lives here.' },
        { path: 'council/ts/src/domain/contracts', note: 'Do not render this unrelated note.' },
      ],
    })

    expect(story).toContain('# Story: Render story shards')
    expect(story).toContain('## Story')
    expect(story).toContain('## Acceptance Criteria')
    expect(story).toContain('## Tasks-Subtasks')
    expect(story).toContain('## Dev Notes')
    expect(story).toContain('## Structure Notes')
    expect(story).toContain('## Dev Agent Record')
    expect(story).toContain('## File List')
    expect(story).toContain('spec.md#1 (Scope)')
    expect(story).toContain('spec.md#3')
    expect(story).toContain('Cited context A.')
    expect(story).toContain('ctx-c')
    expect(story).toContain('Pure domain logic lives here.')
    expect(story).not.toContain('Do not include the whole spec.')
    expect(story).not.toContain('Uncited context B.')
    expect(story).not.toContain('unrelated note')
  })

  it('uses placeholders when optional story shards are absent', () => {
    const story = renderStoryMarkdown({
      task: requiredTask([]),
      contextPack: {
        summary: 'Summary without cited snippets must not be inlined.',
        snippets: [{ ref: 'not-cited', text: 'not cited' }],
      },
      specSections: [{ ref: 'not-cited', text: 'not cited spec' }],
    })

    expect(story).toContain('_Not recorded yet._')
    expect(story).not.toContain('Summary without cited snippets')
    expect(story).not.toContain('not cited spec')
  })
})

describe('validateStoryReadiness', () => {
  it('passes concrete stories with known paths and pinned library assumptions', () => {
    const story = renderStoryMarkdown({ task: BASE_TASK })
    const result = validateStoryReadiness({
      task: BASE_TASK,
      storyMarkdown: story,
      knownPaths: new Set(BASE_TASK.paths),
      libraryAssumptions: [{ name: 'vitest', version: '^4.1.9' }],
    })

    expect(result).toEqual({
      ready: true,
      issues: [],
      revision: { kind: 'none', reason: 'ready' },
    })
  })

  it('flags vague objectives and missing acceptance criteria with one revision signal', () => {
    const result = validateStoryReadiness({
      task: {
        ...BASE_TASK,
        objective: 'TBD',
        acceptance_criteria: [],
      },
      storyMarkdown: '## Acceptance Criteria\n_Not recorded yet._',
    })

    expect(result.ready).toBe(false)
    expect(result.issues.map((issue) => issue.code)).toEqual([
      'vague-objective',
      'missing-acceptance-criteria',
    ])
    expect(result.revision).toEqual({
      kind: 'revise-once',
      round: 1,
      issueCodes: ['vague-objective', 'missing-acceptance-criteria'],
      message: 'Revise the story once to resolve the blocking readiness issues.',
    })
  })

  it('accepts acceptance criteria that are present in the story even when task metadata lacks them', () => {
    const result = validateStoryReadiness({
      task: requiredTask(),
      storyMarkdown: '## Acceptance Criteria\n1. Worker can validate the rendered story.\n## Tasks',
    })

    expect(result.issues.find((issue) => issue.code === 'missing-acceptance-criteria')).toBeUndefined()
  })

  it('flags unknown and malformed paths, deduplicating their revision code', () => {
    const result = validateStoryReadiness({
      task: {
        ...BASE_TASK,
        paths: ['known/path', 'unknown/path', '../escape'],
      },
      storyMarkdown:
        '## Structure Notes\nnot a list item\n- Allowed path: known/path\n- known/path: already covered\n## File List\n- _Not recorded yet._\n- story/unknown',
      knownPaths: new Set(['known/path']),
    })

    expect(result.issues).toEqual([
      {
        code: 'wrong-path',
        message: 'Path is not known in this repository: unknown/path',
        blocking: true,
      },
      {
        code: 'wrong-path',
        message: 'Path is not known in this repository: ../escape',
        blocking: true,
      },
      {
        code: 'wrong-path',
        message: 'Path is not known in this repository: story/unknown',
        blocking: true,
      },
    ])
    expect(result.revision.kind === 'revise-once' ? result.revision.issueCodes : []).toEqual([
      'wrong-path',
    ])
  })

  it('flags malformed paths without requiring a known-path set', () => {
    const result = validateStoryReadiness({
      task: {
        ...BASE_TASK,
        paths: [' bad'],
      },
      storyMarkdown: renderStoryMarkdown({ task: BASE_TASK }),
    })

    expect(result.issues).toEqual([
      {
        code: 'wrong-path',
        message: 'Path is malformed:  bad',
        blocking: true,
      },
    ])
  })

  it('flags explicit and inferred unpinned library assumptions', () => {
    const explicit = validateStoryReadiness({
      task: BASE_TASK,
      storyMarkdown: renderStoryMarkdown({ task: BASE_TASK }),
      libraryAssumptions: [
        { name: 'typescript', version: '5.9.3' },
        { name: 'eslint', version: 'latest' },
        { name: 'prettier' },
      ],
    })
    const inferred = validateStoryReadiness({
      task: {
        ...BASE_TASK,
        objective: 'Render stories using lodash and use src/domain/story for local behavior',
        dev_notes: 'Install zod@next for validation.',
      },
      storyMarkdown: 'Use vitest@4.1.9 for tests.',
    })

    expect(explicit.issues.map((issue) => issue.message)).toEqual([
      'Library assumption must be pinned to a concrete version: eslint',
      'Library assumption must be pinned to a concrete version: prettier',
    ])
    expect(inferred.issues.map((issue) => issue.message)).toEqual([
      'Library assumption must be pinned to a concrete version: lodash',
      'Library assumption must be pinned to a concrete version: zod',
    ])
  })

  it('suppresses the revision signal after the one-round budget is spent', () => {
    const result = validateStoryReadiness({
      task: {
        ...BASE_TASK,
        objective: 'misc',
        acceptance_criteria: [],
      },
      storyMarkdown: '',
      revisionRound: 1,
    })

    expect(result.ready).toBe(false)
    expect(result.revision).toEqual({ kind: 'none', reason: 'revision-budget-exhausted' })
  })
})
