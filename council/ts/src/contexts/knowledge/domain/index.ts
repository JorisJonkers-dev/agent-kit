export * from './emit-mapping.js'
export * from './scope-policy.js'

export type KnowledgeRecallMode = 'fast' | 'hybrid' | 'deep'

export interface KnowledgeRecallRequest {
  readonly query: string
  readonly limit?: number
  readonly mode?: KnowledgeRecallMode
  readonly scope?: string
}

export interface KnowledgeRecallMatch {
  readonly id: string
  readonly title: string
  readonly snippet: string
  readonly score: number
  readonly scope?: string
  readonly topic?: string
  readonly tags?: readonly string[]
}

export interface KnowledgeRecallResult {
  readonly matches: readonly KnowledgeRecallMatch[]
}

export interface KnowledgeGetNoteRequest {
  readonly id: string
}

export interface KnowledgeNote {
  readonly id: string
  readonly title: string
  readonly body: string
  readonly scope?: string
  readonly topic?: string
  readonly tags?: readonly string[]
  readonly created_at?: string
  readonly updated_at?: string
}

export interface KnowledgeRelationsRequest {
  readonly id: string
  readonly depth?: number
}

export interface KnowledgeNoteReference {
  readonly id: string
  readonly title: string
}

export interface KnowledgeRelation {
  readonly from_id: string
  readonly to_id: string
  readonly kind: string
  readonly title?: string
}

export interface KnowledgeRelationsResult {
  readonly note: KnowledgeNoteReference
  readonly relations: readonly KnowledgeRelation[]
}

export interface KnowledgeCaptureLessonRequest {
  readonly title: string
  readonly body: string
  readonly scope?: string
  readonly topic?: string
  readonly tags?: readonly string[]
  readonly source?: string
}

export interface KnowledgeCaptureDecisionRequest {
  readonly title: string
  readonly decision: string
  readonly rationale: string
  readonly body?: string
  readonly scope?: string
  readonly topic?: string
  readonly tags?: readonly string[]
  readonly source?: string
}

export interface KnowledgeIngestNoteRequest {
  readonly id?: string
  readonly title: string
  readonly body: string
  readonly scope: string
  readonly topic?: string
  readonly tags?: readonly string[]
  readonly source?: string
}

export interface KnowledgeCaptureResult {
  readonly id: string
  readonly created: boolean
  readonly path?: string
}

export interface KnowledgeMcpTextContent {
  readonly type: 'text'
  readonly text: string
}

export interface KnowledgeMcpTextResult {
  readonly content: readonly [KnowledgeMcpTextContent]
  readonly isError?: false
}

type JsonRecord = Record<string, unknown>

export function isKnowledgeMcpTextResult(value: unknown): value is KnowledgeMcpTextResult {
  if (!isRecord(value) || value.isError === true || !Array.isArray(value.content)) {
    return false
  }

  const contentItems: readonly unknown[] = value.content
  const content = contentItems[0]

  return (
    contentItems.length === 1 &&
    isRecord(content) &&
    content.type === 'text' &&
    typeof content.text === 'string'
  )
}

export function decodeKnowledgeRecallResult(result: unknown): KnowledgeRecallResult {
  const value = expectRecord(decodeKnowledgeMcpJson(result), 'Knowledge recall result')
  const matches = expectArray(value.matches, 'Knowledge recall result must include a matches array.')

  return { matches: matches.map(decodeRecallMatch) }
}

export function decodeKnowledgeNoteResult(result: unknown): KnowledgeNote {
  return decodeKnowledgeNote(decodeKnowledgeMcpJson(result))
}

export function decodeKnowledgeRelationsResult(result: unknown): KnowledgeRelationsResult {
  const value = expectRecord(decodeKnowledgeMcpJson(result), 'Knowledge relations result')
  const relations = expectArray(
    value.relations,
    'Knowledge relations result must include a relations array.',
  )

  return {
    note: decodeKnowledgeNoteReference(value.note),
    relations: relations.map(decodeKnowledgeRelation),
  }
}

export function decodeKnowledgeCaptureResult(result: unknown): KnowledgeCaptureResult {
  const value = expectRecord(decodeKnowledgeMcpJson(result), 'Knowledge capture result')

  return {
    id: expectString(value.id, 'Knowledge capture result id must be a string.'),
    created: expectBoolean(
      value.created,
      'Knowledge capture result created flag must be a boolean.',
    ),
    ...optionalStringField(value, 'path', 'Knowledge capture result path must be a string.'),
  }
}

function decodeKnowledgeMcpJson(result: unknown): unknown {
  const value = expectRecord(result, 'Knowledge MCP result')

  if (value.isError === true) {
    throw new Error('Knowledge MCP result is an error result.')
  }

  const content = expectArray(
    value.content,
    'Knowledge MCP result content must contain exactly one text item.',
  )
  const [item] = content

  if (content.length !== 1 || !isRecord(item) || item.type !== 'text') {
    throw new Error('Knowledge MCP result content must contain exactly one text item.')
  }

  try {
    return JSON.parse(expectString(item.text, 'Knowledge MCP result text must be a string.'))
  } catch {
    throw new Error('Knowledge MCP result text must be valid JSON.')
  }
}

function decodeRecallMatch(value: unknown): KnowledgeRecallMatch {
  const match = expectRecord(value, 'Knowledge recall match')

  return {
    id: expectString(match.id, 'Knowledge recall match id must be a string.'),
    title: expectString(match.title, 'Knowledge recall match title must be a string.'),
    snippet: expectString(match.snippet, 'Knowledge recall match snippet must be a string.'),
    score: expectFiniteNumber(
      match.score,
      'Knowledge recall match score must be a finite number.',
    ),
    ...optionalStringField(match, 'scope', 'Knowledge recall match scope must be a string.'),
    ...optionalStringField(match, 'topic', 'Knowledge recall match topic must be a string.'),
    ...optionalStringArrayField(match, 'tags', 'Knowledge recall match tags must be strings.'),
  }
}

function decodeKnowledgeNote(value: unknown): KnowledgeNote {
  const note = expectRecord(value, 'Knowledge note')

  return {
    id: expectString(note.id, 'Knowledge note id must be a string.'),
    title: expectString(note.title, 'Knowledge note title must be a string.'),
    body: expectString(note.body, 'Knowledge note body must be a string.'),
    ...optionalStringField(note, 'scope', 'Knowledge note scope must be a string.'),
    ...optionalStringField(note, 'topic', 'Knowledge note topic must be a string.'),
    ...optionalStringArrayField(note, 'tags', 'Knowledge note tags must be strings.'),
    ...optionalStringField(note, 'created_at', 'Knowledge note created_at must be a string.'),
    ...optionalStringField(note, 'updated_at', 'Knowledge note updated_at must be a string.'),
  }
}

function decodeKnowledgeNoteReference(value: unknown): KnowledgeNoteReference {
  const note = expectRecord(value, 'Knowledge note reference')

  return {
    id: expectString(note.id, 'Knowledge note reference id must be a string.'),
    title: expectString(note.title, 'Knowledge note reference title must be a string.'),
  }
}

function decodeKnowledgeRelation(value: unknown): KnowledgeRelation {
  const relation = expectRecord(value, 'Knowledge relation')

  return {
    from_id: expectString(relation.from_id, 'Knowledge relation from_id must be a string.'),
    to_id: expectString(relation.to_id, 'Knowledge relation to_id must be a string.'),
    kind: expectString(relation.kind, 'Knowledge relation kind must be a string.'),
    ...optionalStringField(relation, 'title', 'Knowledge relation title must be a string.'),
  }
}

function optionalStringField(
  value: JsonRecord,
  key: string,
  message: string,
): Record<string, string> {
  return value[key] === undefined ? {} : { [key]: expectString(value[key], message) }
}

function optionalStringArrayField(
  value: JsonRecord,
  key: string,
  message: string,
): Record<string, readonly string[]> {
  return value[key] === undefined ? {} : { [key]: expectStringArray(value[key], message) }
}

function expectRecord(value: unknown, name: string): JsonRecord {
  if (!isRecord(value)) {
    throw new Error(`${name} must be an object.`)
  }

  return value
}

function expectArray(value: unknown, message: string): readonly unknown[] {
  if (!Array.isArray(value)) {
    throw new Error(message)
  }

  return value
}

function expectString(value: unknown, message: string): string {
  if (typeof value !== 'string') {
    throw new Error(message)
  }

  return value
}

function expectStringArray(value: unknown, message: string): readonly string[] {
  const items = expectArray(value, message)

  if (!items.every((item) => typeof item === 'string')) {
    throw new Error(message)
  }

  return items
}

function expectFiniteNumber(value: unknown, message: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new Error(message)
  }

  return value
}

function expectBoolean(value: unknown, message: string): boolean {
  if (typeof value !== 'boolean') {
    throw new Error(message)
  }

  return value
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null
}
