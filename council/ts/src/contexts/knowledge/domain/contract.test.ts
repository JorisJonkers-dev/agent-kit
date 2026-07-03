import { describe, expect, it } from 'vitest'

import type { KnowledgeMcpPort } from '../index.js'
import {
  decodeKnowledgeCaptureResult,
  decodeKnowledgeNoteResult,
  decodeKnowledgeRecallResult,
  decodeKnowledgeRelationsResult,
  isKnowledgeMcpTextResult,
} from '../index.js'

function mcpJson(value: unknown): unknown {
  return {
    content: [{ type: 'text', text: JSON.stringify(value) }],
  }
}

describe('knowledge MCP contract', () => {
  it('defines a pure port with MCP tool-shaped operations and DTOs', async () => {
    const port: KnowledgeMcpPort = {
      recall: (input) =>
        Promise.resolve({
        matches: [
          {
            id: 'note-1',
            title: `Recall ${input.query}`,
            snippet: 'Focused context.',
            score: 0.87,
            scope: input.scope ?? 'project:personal-stack',
            topic: 'topic:agents',
            tags: ['codex'],
          },
        ],
      }),
      get_note: (input) =>
        Promise.resolve({
        id: input.id,
        title: 'Known decision',
        body: 'Use a pure port contract.',
        scope: 'project:personal-stack',
        topic: 'topic:architecture',
        tags: ['council'],
        created_at: '2026-07-03T10:00:00.000Z',
        updated_at: '2026-07-03T11:00:00.000Z',
      }),
      relations: (input) =>
        Promise.resolve({
        note: { id: input.id, title: 'Known decision' },
        relations: [
          {
            from_id: input.id,
            to_id: 'note-2',
            kind: 'supports',
            title: `Depth ${String(input.depth)}`,
          },
        ],
      }),
      capture_lesson: (input) =>
        Promise.resolve({
        id: 'lesson-1',
        created: true,
        path: `${input.scope ?? 'project:personal-stack'}/lesson.md`,
      }),
      capture_decision: (input) =>
        Promise.resolve({
        id: 'decision-1',
        created: true,
        path: `${input.topic ?? 'topic:architecture'}/decision.md`,
      }),
      ingest_note: (input) =>
        Promise.resolve({
        id: input.id ?? 'ingested-1',
        created: false,
      }),
    }

    await expect(
      port.recall({
        query: 'knowledge context port contracts',
        limit: 5,
        mode: 'hybrid',
        scope: 'project:personal-stack',
      }),
    ).resolves.toEqual({
      matches: [
        {
          id: 'note-1',
          title: 'Recall knowledge context port contracts',
          snippet: 'Focused context.',
          score: 0.87,
          scope: 'project:personal-stack',
          topic: 'topic:agents',
          tags: ['codex'],
        },
      ],
    })
    await expect(port.get_note({ id: 'note-1' })).resolves.toMatchObject({
      id: 'note-1',
      title: 'Known decision',
    })
    await expect(port.relations({ id: 'note-1', depth: 1 })).resolves.toMatchObject({
      relations: [{ kind: 'supports' }],
    })
    await expect(
      port.capture_lesson({
        title: 'Validate MCP payloads',
        body: 'Reject malformed tool results before adapters consume them.',
        scope: 'project:personal-stack',
        tags: ['contracts'],
      }),
    ).resolves.toMatchObject({ id: 'lesson-1', created: true })
    await expect(
      port.capture_decision({
        title: 'Keep knowledge contracts pure',
        decision: 'Use hand-written DTO guards.',
        rationale: 'The context must not import adapter or IO dependencies.',
        topic: 'topic:architecture',
      }),
    ).resolves.toMatchObject({ id: 'decision-1', created: true })
    await expect(
      port.ingest_note({
        id: 'note-existing',
        title: 'Imported note',
        body: 'Imported body.',
        scope: 'project:personal-stack',
      }),
    ).resolves.toEqual({ id: 'note-existing', created: false })
  })

  it('decodes valid MCP text results into typed knowledge DTOs', () => {
    expect(isKnowledgeMcpTextResult(mcpJson({ matches: [] }))).toBe(true)
    expect(
      decodeKnowledgeRecallResult(
        mcpJson({
          matches: [
            {
              id: 'note-1',
              title: 'Relevant note',
              snippet: 'A useful excerpt.',
              score: 0.42,
              scope: 'project:personal-stack',
              topic: 'topic:agents',
              tags: ['codex', 'council'],
            },
          ],
        }),
      ),
    ).toEqual({
      matches: [
        {
          id: 'note-1',
          title: 'Relevant note',
          snippet: 'A useful excerpt.',
          score: 0.42,
          scope: 'project:personal-stack',
          topic: 'topic:agents',
          tags: ['codex', 'council'],
        },
      ],
    })
    expect(
      decodeKnowledgeNoteResult(
        mcpJson({
          id: 'note-1',
          title: 'Relevant note',
          body: 'Full note body.',
          scope: 'project:personal-stack',
          topic: 'topic:agents',
          tags: ['codex'],
          created_at: '2026-07-03T10:00:00.000Z',
          updated_at: '2026-07-03T11:00:00.000Z',
        }),
      ),
    ).toEqual({
      id: 'note-1',
      title: 'Relevant note',
      body: 'Full note body.',
      scope: 'project:personal-stack',
      topic: 'topic:agents',
      tags: ['codex'],
      created_at: '2026-07-03T10:00:00.000Z',
      updated_at: '2026-07-03T11:00:00.000Z',
    })
    expect(
      decodeKnowledgeRelationsResult(
        mcpJson({
          note: { id: 'note-1', title: 'Relevant note' },
          relations: [
            {
              from_id: 'note-1',
              to_id: 'note-2',
              kind: 'supports',
              title: 'Related note',
            },
          ],
        }),
      ),
    ).toEqual({
      note: { id: 'note-1', title: 'Relevant note' },
      relations: [
        {
          from_id: 'note-1',
          to_id: 'note-2',
          kind: 'supports',
          title: 'Related note',
        },
      ],
    })
    expect(
      decodeKnowledgeCaptureResult(
        mcpJson({ id: 'lesson-1', created: true, path: 'knowledge/lesson.md' }),
      ),
    ).toEqual({ id: 'lesson-1', created: true, path: 'knowledge/lesson.md' })
  })

  it('rejects malformed MCP envelopes before reading domain payloads', () => {
    expect(isKnowledgeMcpTextResult(null)).toBe(false)
    expect(isKnowledgeMcpTextResult({ content: [{ type: 'image', data: 'nope' }] })).toBe(
      false,
    )
    expect(() => decodeKnowledgeRecallResult(null)).toThrow(
      'Knowledge MCP result must be an object.',
    )
    expect(() =>
      decodeKnowledgeRecallResult({ isError: true, content: [{ type: 'text', text: '{}' }] }),
    ).toThrow('Knowledge MCP result is an error result.')
    expect(() => decodeKnowledgeRecallResult({ content: [] })).toThrow(
      'Knowledge MCP result content must contain exactly one text item.',
    )
    expect(() =>
      decodeKnowledgeRecallResult({ content: [{ type: 'text', text: '{' }] }),
    ).toThrow('Knowledge MCP result text must be valid JSON.')
  })

  it('rejects malformed knowledge payloads inside valid MCP envelopes', () => {
    expect(() => decodeKnowledgeRecallResult(mcpJson({ matches: 'nope' }))).toThrow(
      'Knowledge recall result must include a matches array.',
    )
    expect(() =>
      decodeKnowledgeRecallResult(
        mcpJson({
          matches: [
            {
              id: 'note-1',
              title: 'Bad tags',
              snippet: 'x',
              score: 0.1,
              tags: ['ok', 42],
            },
          ],
        }),
      ),
    ).toThrow('Knowledge recall match tags must be strings.')
    expect(() =>
      decodeKnowledgeRecallResult(
        mcpJson({
          matches: [{ id: 'note-1', title: 'Bad score', snippet: 'x', score: '0.1' }],
        }),
      ),
    ).toThrow('Knowledge recall match score must be a finite number.')
    expect(() =>
      decodeKnowledgeNoteResult(mcpJson({ id: 'note-1', title: 'Missing body' })),
    ).toThrow('Knowledge note body must be a string.')
    expect(() =>
      decodeKnowledgeRelationsResult(mcpJson({ note: { id: 'note-1' }, relations: [] })),
    ).toThrow('Knowledge note reference title must be a string.')
    expect(() =>
      decodeKnowledgeRelationsResult(
        mcpJson({
          note: { id: 'note-1', title: 'Note' },
          relations: [{ from_id: 'note-1', to_id: 'note-2', kind: 42 }],
        }),
      ),
    ).toThrow('Knowledge relation kind must be a string.')
    expect(() =>
      decodeKnowledgeCaptureResult(mcpJson({ id: 'lesson-1', created: 'yes' })),
    ).toThrow('Knowledge capture result created flag must be a boolean.')
  })
})
