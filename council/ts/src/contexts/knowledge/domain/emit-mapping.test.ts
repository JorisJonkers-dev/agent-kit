import { describe, expect, it } from 'vitest'

import { mapKnowledgeEmission } from './emit-mapping.js'

describe('knowledge emit mapping', () => {
  it('maps decision emissions to capture_decision requests with source, scope, tags, and confidence', () => {
    expect(
      mapKnowledgeEmission({
        classification: 'decision',
        runId: 'run-42',
        phase: 'design',
        title: 'Use a pure domain policy',
        decision: 'Keep capture mapping free of adapters.',
        rationale: 'The mapper must be testable without git or MCP.',
        body: 'Adapters receive a typed command after policy has run.',
        scope: 'project:acme/widget',
        tags: ['architecture', 'knowledge'],
        confidence: 0.91,
      }),
    ).toEqual({
      classification: 'decision',
      operation: 'capture_decision',
      confidence: 0.91,
      request: {
        title: 'Use a pure domain policy',
        decision: 'Keep capture mapping free of adapters.',
        rationale: 'The mapper must be testable without git or MCP.',
        body: 'Adapters receive a typed command after policy has run.',
        scope: 'project:acme/widget',
        tags: ['architecture', 'knowledge'],
        source: 'council:run-42:design',
      },
    })

    expect(
      mapKnowledgeEmission({
        classification: 'decision',
        runId: 'run-43',
        phase: 'triage',
        title: 'Use the small path',
        decision: 'Do not synthesize body text.',
        rationale: 'The decision and rationale fields already carry the capture.',
        bodySource: 'curated',
        confidence: 0.7,
      }),
    ).toEqual({
      classification: 'decision',
      operation: 'capture_decision',
      confidence: 0.7,
      request: {
        title: 'Use the small path',
        decision: 'Do not synthesize body text.',
        rationale: 'The decision and rationale fields already carry the capture.',
        source: 'council:run-43:triage',
      },
    })
  })

  it('maps lesson emissions and omits optional tags plus the curated default scope', () => {
    const mapped = mapKnowledgeEmission({
      classification: 'lesson',
      runId: 'run-42',
      phase: 'review',
      title: 'Cover policy helpers directly',
      body: 'Pure helpers need direct branch coverage under the global threshold.',
      scope: 'project:personal-stack',
      confidence: 0.84,
    })

    expect(mapped).toEqual({
      classification: 'lesson',
      operation: 'capture_lesson',
      confidence: 0.84,
      request: {
        title: 'Cover policy helpers directly',
        body: 'Pure helpers need direct branch coverage under the global threshold.',
        source: 'council:run-42:review',
      },
    })
    expect('scope' in mapped.request).toBe(false)
    expect('tags' in mapped.request).toBe(false)
  })

  it('maps archival emissions to archive notes with canonical scope inferred from a GitHub remote', () => {
    expect(
      mapKnowledgeEmission({
        classification: 'archival',
        runId: 'run-99',
        phase: 'complete',
        title: 'Council final summary',
        body: 'The final summary is curated and intentionally compact.',
        gitRemote: 'git@github.com:Acme-Corp/Worker-Service.git',
        tags: ['summary'],
        confidence: 0.77,
      }),
    ).toEqual({
      classification: 'archival',
      operation: 'archive_note',
      confidence: 0.77,
      note: {
        title: 'Council final summary',
        body: 'The final summary is curated and intentionally compact.',
        scope: 'project:acme-corp/worker-service',
        tags: ['summary'],
        source: 'council:run-99:complete',
      },
    })
  })

  it('rejects raw process or event payloads as capture body text', () => {
    expect(() =>
      mapKnowledgeEmission({
        classification: 'lesson',
        runId: 'run-42',
        phase: 'verify',
        title: 'Do not capture stdout',
        body: 'npm test raw output',
        bodySource: 'stdout',
        confidence: 0.5,
      }),
    ).toThrow('knowledge emission body must be curated text, not stdout.')

    expect(() =>
      mapKnowledgeEmission({
        classification: 'archival',
        runId: 'run-42',
        phase: 'events',
        title: 'Do not capture event JSON',
        body: '{"type":"delta"}',
        bodySource: 'event_json',
        confidence: 0.5,
      }),
    ).toThrow('knowledge emission body must be curated text, not event_json.')
  })
})
