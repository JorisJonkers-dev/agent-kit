import type {
  KnowledgeCaptureDecisionRequest,
  KnowledgeCaptureLessonRequest,
  KnowledgeCaptureResult,
  KnowledgeGetNoteRequest,
  KnowledgeIngestNoteRequest,
  KnowledgeNote,
  KnowledgeRecallRequest,
  KnowledgeRecallResult,
  KnowledgeRelationsRequest,
  KnowledgeRelationsResult,
} from '../domain/index.js'

export interface KnowledgeMcpPort {
  readonly recall: (input: KnowledgeRecallRequest) => Promise<KnowledgeRecallResult>
  readonly get_note: (input: KnowledgeGetNoteRequest) => Promise<KnowledgeNote>
  readonly relations: (input: KnowledgeRelationsRequest) => Promise<KnowledgeRelationsResult>
  readonly capture_lesson: (
    input: KnowledgeCaptureLessonRequest,
  ) => Promise<KnowledgeCaptureResult>
  readonly capture_decision: (
    input: KnowledgeCaptureDecisionRequest,
  ) => Promise<KnowledgeCaptureResult>
  readonly ingest_note: (input: KnowledgeIngestNoteRequest) => Promise<KnowledgeCaptureResult>
}
