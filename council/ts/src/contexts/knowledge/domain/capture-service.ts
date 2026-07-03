import {
  type KnowledgeArchiveNoteRequest,
  type KnowledgeDecisionCaptureRequest,
  type KnowledgeEmission,
  type KnowledgeEmissionMapping,
  type KnowledgeLessonCaptureRequest,
  mapKnowledgeEmission,
} from './emit-mapping.js'
import { CURATED_DEFAULT_KNOWLEDGE_SCOPE } from './scope-policy.js'

export type KnowledgeCaptureServiceOperation =
  | 'capture_decision'
  | 'capture_lesson'
  | 'ingest_note'

export interface KnowledgeCaptureServiceRecallRequest {
  readonly query: string
  readonly limit: 1
  readonly scope: string
}

export interface KnowledgeCaptureServiceRecallMatch {
  readonly id: string
  readonly title: string
  readonly snippet: string
  readonly score: number
  readonly scope?: string
  readonly topic?: string
  readonly tags?: readonly string[]
}

export interface KnowledgeCaptureServiceRecallResult {
  readonly matches: readonly KnowledgeCaptureServiceRecallMatch[]
}

export interface KnowledgeCaptureServiceResultValue {
  readonly id: string
  readonly created: boolean
  readonly path?: string
}

export interface KnowledgeCaptureServiceIngestNoteRequest extends KnowledgeArchiveNoteRequest {
  readonly scope: string
}

export interface KnowledgeCaptureServicePort {
  readonly recall: (
    input: KnowledgeCaptureServiceRecallRequest,
  ) => Promise<KnowledgeCaptureServiceRecallResult>
  readonly capture_lesson: (
    input: KnowledgeLessonCaptureRequest,
  ) => Promise<KnowledgeCaptureServiceResultValue>
  readonly capture_decision: (
    input: KnowledgeDecisionCaptureRequest,
  ) => Promise<KnowledgeCaptureServiceResultValue>
  readonly ingest_note: (
    input: KnowledgeCaptureServiceIngestNoteRequest,
  ) => Promise<KnowledgeCaptureServiceResultValue>
}

export interface KnowledgeCaptureServiceCaptured {
  readonly status: 'captured'
  readonly operation: KnowledgeCaptureServiceOperation
  readonly result: KnowledgeCaptureServiceResultValue
}

export interface KnowledgeCaptureServiceDuplicate {
  readonly status: 'duplicate'
  readonly operation: KnowledgeCaptureServiceOperation
  readonly duplicate: KnowledgeCaptureServiceRecallMatch
}

export type KnowledgeCaptureServiceResult =
  | KnowledgeCaptureServiceCaptured
  | KnowledgeCaptureServiceDuplicate

export interface KnowledgeCaptureService {
  readonly capture: (emission: KnowledgeEmission) => Promise<KnowledgeCaptureServiceResult>
}

export function createKnowledgeCaptureService(
  port: KnowledgeCaptureServicePort,
): KnowledgeCaptureService {
  return {
    capture: (emission) => captureKnowledgeEmission(port, emission),
  }
}

async function captureKnowledgeEmission(
  port: KnowledgeCaptureServicePort,
  emission: KnowledgeEmission,
): Promise<KnowledgeCaptureServiceResult> {
  const mapping = mapKnowledgeEmission(emission)
  const operation = captureOperation(mapping)
  const recall = await port.recall(recallRequest(mapping))
  const duplicate = recall.matches[0]

  if (duplicate !== undefined) {
    return { status: 'duplicate', operation, duplicate }
  }

  return {
    status: 'captured',
    operation,
    result: await captureMappedEmission(port, mapping),
  }
}

function captureOperation(mapping: KnowledgeEmissionMapping): KnowledgeCaptureServiceOperation {
  return mapping.operation === 'archive_note' ? 'ingest_note' : mapping.operation
}

function recallRequest(mapping: KnowledgeEmissionMapping): KnowledgeCaptureServiceRecallRequest {
  return {
    query: captureTitle(mapping),
    limit: 1,
    scope: captureScope(mapping) ?? CURATED_DEFAULT_KNOWLEDGE_SCOPE,
  }
}

function captureTitle(mapping: KnowledgeEmissionMapping): string {
  return mapping.operation === 'archive_note' ? mapping.note.title : mapping.request.title
}

function captureScope(mapping: KnowledgeEmissionMapping): string | undefined {
  return mapping.operation === 'archive_note' ? mapping.note.scope : mapping.request.scope
}

function captureMappedEmission(
  port: KnowledgeCaptureServicePort,
  mapping: KnowledgeEmissionMapping,
): Promise<KnowledgeCaptureServiceResultValue> {
  switch (mapping.operation) {
    case 'capture_decision':
      return port.capture_decision(mapping.request)
    case 'capture_lesson':
      return port.capture_lesson(mapping.request)
    case 'archive_note':
      return port.ingest_note(ingestNoteRequest(mapping.note))
  }
}

function ingestNoteRequest(
  note: KnowledgeArchiveNoteRequest,
): KnowledgeCaptureServiceIngestNoteRequest {
  return {
    ...note,
    scope: note.scope ?? CURATED_DEFAULT_KNOWLEDGE_SCOPE,
  }
}
