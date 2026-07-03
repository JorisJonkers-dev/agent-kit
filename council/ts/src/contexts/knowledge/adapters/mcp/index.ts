import {
  decodeKnowledgeCaptureResult,
  decodeKnowledgeNoteResult,
  decodeKnowledgeRecallResult,
  decodeKnowledgeRelationsResult,
  type KnowledgeCaptureDecisionRequest,
  type KnowledgeCaptureLessonRequest,
  type KnowledgeCaptureResult,
  type KnowledgeGetNoteRequest,
  type KnowledgeIngestNoteRequest,
  type KnowledgeNote,
  type KnowledgeRecallRequest,
  type KnowledgeRecallResult,
  type KnowledgeRelationsRequest,
  type KnowledgeRelationsResult,
} from '../../domain/index.js'
import type { KnowledgeMcpPort } from '../../ports/index.js'

export interface KnowledgeMcpEnvironment {
  require(name: string): string
}

export interface KnowledgeMcpFetchInit {
  readonly body: string
  readonly headers: Readonly<Record<string, string>>
  readonly method: 'POST'
}

export interface KnowledgeMcpFetchResponse {
  readonly ok: boolean
  readonly status: number
  json(): Promise<unknown>
}

export type KnowledgeMcpFetch = (
  url: string,
  init: KnowledgeMcpFetchInit,
) => Promise<KnowledgeMcpFetchResponse>

export interface KnowledgeMcpJsonRpcAdapterOptions {
  readonly env: KnowledgeMcpEnvironment
  readonly fetch: KnowledgeMcpFetch
}

type JsonRecord = Record<string, unknown>

export class KnowledgeMcpAuthError extends Error {
  readonly status: number

  constructor(status: number) {
    super(`Knowledge MCP authentication failed with HTTP ${String(status)}.`)
    this.name = 'KnowledgeMcpAuthError'
    this.status = status
  }
}

export class KnowledgeMcpRpcError extends Error {
  readonly code: number
  readonly data: unknown

  constructor(code: number, message: string, data: unknown) {
    super(`Knowledge MCP JSON-RPC error ${String(code)}: ${message}`)
    this.name = 'KnowledgeMcpRpcError'
    this.code = code
    this.data = data
  }
}

export class KnowledgeMcpJsonRpcAdapter implements KnowledgeMcpPort {
  private nextId = 1
  private readonly endpoint: string
  private readonly fetch: KnowledgeMcpFetch
  private readonly token: string

  constructor(options: KnowledgeMcpJsonRpcAdapterOptions) {
    this.endpoint = normalizeMcpEndpoint(readRequiredEnv(options.env, 'KB_URL'))
    this.fetch = options.fetch
    this.token = readRequiredEnv(options.env, 'KB_BEARER_TOKEN')
  }

  async recall(input: KnowledgeRecallRequest): Promise<KnowledgeRecallResult> {
    const result = await this.callTool('knowledge.recall', input)
    return decodeKnowledgeRecallResult(knowledgeToolResult('knowledge.recall', result))
  }

  async get_note(input: KnowledgeGetNoteRequest): Promise<KnowledgeNote> {
    const result = await this.callTool('knowledge.get_note', input)
    return decodeKnowledgeNoteResult(knowledgeToolResult('knowledge.get_note', result))
  }

  async relations(input: KnowledgeRelationsRequest): Promise<KnowledgeRelationsResult> {
    const result = await this.callTool('knowledge.relations', input)
    return decodeKnowledgeRelationsResult(knowledgeToolResult('knowledge.relations', result))
  }

  async capture_lesson(input: KnowledgeCaptureLessonRequest): Promise<KnowledgeCaptureResult> {
    const result = await this.callTool('knowledge.capture_lesson', input)
    return decodeKnowledgeCaptureResult(knowledgeToolResult('knowledge.capture_lesson', result))
  }

  async capture_decision(input: KnowledgeCaptureDecisionRequest): Promise<KnowledgeCaptureResult> {
    const result = await this.callTool('knowledge.capture_decision', input)
    return decodeKnowledgeCaptureResult(knowledgeToolResult('knowledge.capture_decision', result))
  }

  async ingest_note(input: KnowledgeIngestNoteRequest): Promise<KnowledgeCaptureResult> {
    const result = await this.callTool('knowledge.ingest_note', input)
    return decodeKnowledgeCaptureResult(knowledgeToolResult('knowledge.ingest_note', result))
  }

  private async callTool(name: string, args: unknown): Promise<unknown> {
    const id = this.nextId
    this.nextId += 1
    const response = await this.fetch(this.endpoint, {
      body: JSON.stringify({
        jsonrpc: '2.0',
        id,
        method: 'tools/call',
        params: { name, arguments: args },
      }),
      headers: {
        Authorization: `Bearer ${this.token}`,
        'Content-Type': 'application/json',
      },
      method: 'POST',
    })

    if (response.status === 401 || response.status === 403) {
      throw new KnowledgeMcpAuthError(response.status)
    }

    if (!response.ok) {
      throw new Error(`Knowledge MCP request failed with HTTP ${String(response.status)}.`)
    }

    return decodeJsonRpcResult(await response.json(), id)
  }
}

function readRequiredEnv(env: KnowledgeMcpEnvironment, name: string): string {
  const value = env.require(name).trim()

  if (value.length === 0) {
    throw new Error(`${name} must not be empty.`)
  }

  return value
}

function normalizeMcpEndpoint(value: string): string {
  const withoutTrailingSlash = value.replace(/\/+$/u, '')
  return withoutTrailingSlash.endsWith('/mcp')
    ? withoutTrailingSlash
    : `${withoutTrailingSlash}/mcp`
}

function decodeJsonRpcResult(body: unknown, id: number): unknown {
  const response = expectRecord(body, 'Knowledge MCP JSON-RPC response')

  if (response.jsonrpc !== '2.0') {
    throw new Error('Knowledge MCP JSON-RPC response jsonrpc must be "2.0".')
  }

  if (response.id !== id) {
    throw new Error(`Knowledge MCP JSON-RPC response id must match request id ${String(id)}.`)
  }

  if (response.error !== undefined) {
    const error = expectRecord(response.error, 'Knowledge MCP JSON-RPC error')

    if (typeof error.code !== 'number' || typeof error.message !== 'string') {
      throw new Error('Knowledge MCP JSON-RPC error must include numeric code and string message.')
    }

    throw new KnowledgeMcpRpcError(error.code, error.message, error.data)
  }

  if (!Object.hasOwn(response, 'result')) {
    throw new Error('Knowledge MCP JSON-RPC response must include a result.')
  }

  return response.result
}

function knowledgeToolResult(toolName: string, result: unknown): unknown {
  if (isRecord(result)) {
    if (result.isError === true) {
      throw new Error('Knowledge MCP result is an error result.')
    }

    if (Object.hasOwn(result, 'structuredContent')) {
      return mcpJson(normalizeStructuredContent(toolName, result.structuredContent))
    }
  }

  return result
}

function normalizeStructuredContent(toolName: string, structuredContent: unknown): unknown {
  if (
    toolName === 'knowledge.recall' &&
    isRecord(structuredContent) &&
    Array.isArray(structuredContent.hits)
  ) {
    return { matches: structuredContent.hits }
  }

  return structuredContent
}

function mcpJson(value: unknown): unknown {
  return {
    content: [{ type: 'text', text: JSON.stringify(value) }],
  }
}

function expectRecord(value: unknown, name: string): JsonRecord {
  if (!isRecord(value)) {
    throw new Error(`${name} must be an object.`)
  }

  return value
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null
}
