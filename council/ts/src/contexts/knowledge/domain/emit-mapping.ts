import { resolveKnowledgeScope } from './scope-policy.js'

export type KnowledgeEmissionClassification = 'decision' | 'lesson' | 'archival'
export type KnowledgeEmissionBodySource = 'curated' | 'stdout' | 'stderr' | 'event_json'

export interface KnowledgeEmissionBase {
  readonly runId: string
  readonly phase: string
  readonly title: string
  readonly confidence: number
  readonly scope?: string
  readonly gitRemote?: string
  readonly allowAll?: boolean
  readonly tags?: readonly string[]
  readonly bodySource?: KnowledgeEmissionBodySource
}

export interface KnowledgeDecisionEmission extends KnowledgeEmissionBase {
  readonly classification: 'decision'
  readonly decision: string
  readonly rationale: string
  readonly body?: string
}

export interface KnowledgeLessonEmission extends KnowledgeEmissionBase {
  readonly classification: 'lesson'
  readonly body: string
}

export interface KnowledgeArchivalEmission extends KnowledgeEmissionBase {
  readonly classification: 'archival'
  readonly body: string
}

export type KnowledgeEmission =
  | KnowledgeDecisionEmission
  | KnowledgeLessonEmission
  | KnowledgeArchivalEmission

export interface KnowledgeDecisionCaptureRequest {
  readonly title: string
  readonly decision: string
  readonly rationale: string
  readonly body?: string
  readonly scope?: string
  readonly tags?: readonly string[]
  readonly source?: string
}

export interface KnowledgeLessonCaptureRequest {
  readonly title: string
  readonly body: string
  readonly scope?: string
  readonly tags?: readonly string[]
  readonly source?: string
}

export interface KnowledgeArchiveNoteRequest {
  readonly title: string
  readonly body: string
  readonly scope?: string
  readonly tags?: readonly string[]
  readonly source?: string
}

export type KnowledgeEmissionMapping =
  | {
      readonly classification: 'decision'
      readonly operation: 'capture_decision'
      readonly confidence: number
      readonly request: KnowledgeDecisionCaptureRequest
    }
  | {
      readonly classification: 'lesson'
      readonly operation: 'capture_lesson'
      readonly confidence: number
      readonly request: KnowledgeLessonCaptureRequest
    }
  | {
      readonly classification: 'archival'
      readonly operation: 'archive_note'
      readonly confidence: number
      readonly note: KnowledgeArchiveNoteRequest
    }

export type KnowledgeDecisionEmissionMapping = Extract<
  KnowledgeEmissionMapping,
  { readonly classification: 'decision' }
>
export type KnowledgeLessonEmissionMapping = Extract<
  KnowledgeEmissionMapping,
  { readonly classification: 'lesson' }
>
export type KnowledgeArchivalEmissionMapping = Extract<
  KnowledgeEmissionMapping,
  { readonly classification: 'archival' }
>

interface CommonEmissionFields {
  readonly scope?: string
  readonly tags?: readonly string[]
  readonly source: string
}

export function mapKnowledgeEmission(input: KnowledgeDecisionEmission): KnowledgeDecisionEmissionMapping
export function mapKnowledgeEmission(input: KnowledgeLessonEmission): KnowledgeLessonEmissionMapping
export function mapKnowledgeEmission(input: KnowledgeArchivalEmission): KnowledgeArchivalEmissionMapping
export function mapKnowledgeEmission(input: KnowledgeEmission): KnowledgeEmissionMapping
export function mapKnowledgeEmission(input: KnowledgeEmission): KnowledgeEmissionMapping {
  assertCuratedBody(input.bodySource)

  const common = commonEmissionFields(input)
  switch (input.classification) {
    case 'decision':
      return {
        classification: input.classification,
        operation: 'capture_decision',
        confidence: input.confidence,
        request: {
          title: input.title,
          decision: input.decision,
          rationale: input.rationale,
          ...(input.body === undefined ? {} : { body: input.body }),
          ...common,
        },
      }
    case 'lesson':
      return {
        classification: input.classification,
        operation: 'capture_lesson',
        confidence: input.confidence,
        request: {
          title: input.title,
          body: input.body,
          ...common,
        },
      }
    case 'archival':
      return {
        classification: input.classification,
        operation: 'archive_note',
        confidence: input.confidence,
        note: {
          title: input.title,
          body: input.body,
          ...common,
        },
      }
  }
}

function commonEmissionFields(input: KnowledgeEmission): CommonEmissionFields {
  const scope = resolveKnowledgeScope({
    requestedScope: input.scope,
    gitRemote: input.gitRemote,
    allowAll: input.allowAll,
  })

  return {
    ...(scope === undefined ? {} : { scope }),
    ...(input.tags === undefined ? {} : { tags: input.tags }),
    source: `council:${input.runId}:${input.phase}`,
  }
}

function assertCuratedBody(source: KnowledgeEmissionBodySource | undefined): void {
  if (source !== undefined && source !== 'curated') {
    throw new Error(`knowledge emission body must be curated text, not ${source}.`)
  }
}
