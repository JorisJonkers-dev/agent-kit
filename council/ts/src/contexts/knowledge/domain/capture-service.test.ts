import { describe, expect, it } from 'vitest'

import type {
  KnowledgeCaptureDecisionRequest,
  KnowledgeCaptureLessonRequest,
  KnowledgeCaptureResult,
  KnowledgeEmissionBodySource,
  KnowledgeIngestNoteRequest,
  KnowledgeMcpPort,
  KnowledgeNote,
  KnowledgeRecallRequest,
  KnowledgeRecallResult,
  KnowledgeRelationsResult,
} from '../index.js'
import { createKnowledgeCaptureService } from './capture-service.js'

class FakeKnowledgeMcpPort implements KnowledgeMcpPort {
  readonly calls: string[] = []
  readonly recalls: KnowledgeRecallRequest[] = []
  readonly lessons: KnowledgeCaptureLessonRequest[] = []
  readonly decisions: KnowledgeCaptureDecisionRequest[] = []
  readonly ingested: KnowledgeIngestNoteRequest[] = []

  constructor(private readonly recallResult: KnowledgeRecallResult = { matches: [] }) {}

  readonly recall = (input: KnowledgeRecallRequest): Promise<KnowledgeRecallResult> => {
    this.calls.push('recall')
    this.recalls.push(input)
    return Promise.resolve(this.recallResult)
  }

  readonly get_note = (): Promise<KnowledgeNote> =>
    Promise.reject(new Error('unexpected get_note call'))

  readonly relations = (): Promise<KnowledgeRelationsResult> =>
    Promise.reject(new Error('unexpected relations call'))

  readonly capture_lesson = (
    input: KnowledgeCaptureLessonRequest,
  ): Promise<KnowledgeCaptureResult> => {
    this.calls.push('capture_lesson')
    this.lessons.push(input)
    return Promise.resolve({ id: 'lesson-1', created: true })
  }

  readonly capture_decision = (
    input: KnowledgeCaptureDecisionRequest,
  ): Promise<KnowledgeCaptureResult> => {
    this.calls.push('capture_decision')
    this.decisions.push(input)
    return Promise.resolve({ id: 'decision-1', created: true })
  }

  readonly ingest_note = (input: KnowledgeIngestNoteRequest): Promise<KnowledgeCaptureResult> => {
    this.calls.push('ingest_note')
    this.ingested.push(input)
    return Promise.resolve({ id: 'note-1', created: true, path: 'knowledge/note.md' })
  }
}

describe('knowledge capture service', () => {
  it('recalls with limit one before capturing a non-duplicate lesson', async () => {
    const port = new FakeKnowledgeMcpPort()
    const service = createKnowledgeCaptureService(port)

    await expect(
      service.capture({
        classification: 'lesson',
        runId: 'run-42',
        phase: 'verify',
        title: 'Cover service policy',
        body: 'Capture service behavior belongs in pure domain tests.',
        scope: 'project:personal-stack',
        tags: ['knowledge'],
        confidence: 0.88,
      }),
    ).resolves.toEqual({
      status: 'captured',
      operation: 'capture_lesson',
      result: { id: 'lesson-1', created: true },
    })

    expect(port.calls).toEqual(['recall', 'capture_lesson'])
    expect(port.recalls).toEqual([
      { query: 'Cover service policy', limit: 1, scope: 'project:personal-stack' },
    ])
    expect(port.lessons).toEqual([
      {
        title: 'Cover service policy',
        body: 'Capture service behavior belongs in pure domain tests.',
        tags: ['knowledge'],
        source: 'council:run-42:verify',
      },
    ])
  })

  it('suppresses capture when recall finds an existing note', async () => {
    const duplicate = {
      id: 'existing-lesson',
      title: 'Existing service policy',
      snippet: 'Already captured.',
      score: 0.93,
      scope: 'project:personal-stack',
    }
    const port = new FakeKnowledgeMcpPort({ matches: [duplicate] })
    const service = createKnowledgeCaptureService(port)

    await expect(
      service.capture({
        classification: 'lesson',
        runId: 'run-42',
        phase: 'review',
        title: 'Existing service policy',
        body: 'Already captured.',
        confidence: 0.82,
      }),
    ).resolves.toEqual({
      status: 'duplicate',
      operation: 'capture_lesson',
      duplicate,
    })

    expect(port.calls).toEqual(['recall'])
    expect(port.lessons).toEqual([])
    expect(port.decisions).toEqual([])
    expect(port.ingested).toEqual([])
  })

  it('uses the decision capture method for non-duplicate decision emissions', async () => {
    const port = new FakeKnowledgeMcpPort()
    const service = createKnowledgeCaptureService(port)

    await expect(
      service.capture({
        classification: 'decision',
        runId: 'run-43',
        phase: 'design',
        title: 'Keep capture pure',
        decision: 'Depend on a port-shaped interface.',
        rationale: 'The domain service must stay testable without MCP IO.',
        body: 'Only curated summaries are acceptable as optional body text.',
        gitRemote: 'git@github.com:Acme/Worker.git',
        confidence: 0.91,
      }),
    ).resolves.toEqual({
      status: 'captured',
      operation: 'capture_decision',
      result: { id: 'decision-1', created: true },
    })

    expect(port.calls).toEqual(['recall', 'capture_decision'])
    expect(port.recalls).toEqual([
      { query: 'Keep capture pure', limit: 1, scope: 'project:acme/worker' },
    ])
    expect(port.decisions).toEqual([
      {
        title: 'Keep capture pure',
        decision: 'Depend on a port-shaped interface.',
        rationale: 'The domain service must stay testable without MCP IO.',
        body: 'Only curated summaries are acceptable as optional body text.',
        scope: 'project:acme/worker',
        source: 'council:run-43:design',
      },
    ])
  })

  it('uses ingest_note for archival emissions and supplies the curated default scope', async () => {
    const port = new FakeKnowledgeMcpPort()
    const service = createKnowledgeCaptureService(port)

    await expect(
      service.capture({
        classification: 'archival',
        runId: 'run-44',
        phase: 'complete',
        title: 'Final summary',
        body: 'A compact curated summary of the run outcome.',
        confidence: 0.76,
      }),
    ).resolves.toEqual({
      status: 'captured',
      operation: 'ingest_note',
      result: { id: 'note-1', created: true, path: 'knowledge/note.md' },
    })

    expect(port.calls).toEqual(['recall', 'ingest_note'])
    expect(port.recalls).toEqual([
      { query: 'Final summary', limit: 1, scope: 'project:personal-stack' },
    ])
    expect(port.ingested).toEqual([
      {
        title: 'Final summary',
        body: 'A compact curated summary of the run outcome.',
        scope: 'project:personal-stack',
        source: 'council:run-44:complete',
      },
    ])
  })

  it.each<KnowledgeEmissionBodySource>(['stdout', 'stderr', 'event_json'])(
    'rejects raw %s payloads before recall or capture',
    async (bodySource) => {
      const port = new FakeKnowledgeMcpPort()
      const service = createKnowledgeCaptureService(port)

      await expect(
        service.capture({
          classification: 'lesson',
          runId: 'run-45',
          phase: 'verify',
          title: 'Do not capture raw output',
          body: 'unfiltered process payload',
          bodySource,
          confidence: 0.5,
        }),
      ).rejects.toThrow(`knowledge emission body must be curated text, not ${bodySource}.`)

      expect(port.calls).toEqual([])
    },
  )
})
