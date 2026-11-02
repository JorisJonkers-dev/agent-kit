import { createInterface } from 'node:readline'
import type { Readable } from 'node:stream'

import { DEFAULT_SEARCH_LIMITS, SEARCH_TOOL_DEFINITIONS, type SearchLimits } from '../../domain/packet-search.js'
import { isJsonObject } from '../../domain/json.js'
import { PacketSearchEngine } from './engine.js'

/** The MCP version the server speaks when the client asks for none. */
const PROTOCOL_VERSION = '2025-06-18'

const SERVER_INFO = { name: 'critique-packet-search', version: '1' }

/** The server's standard streams: newline-delimited JSON-RPC in, the same out, and a line of diagnostics on failure. */
export interface SearchServerIo {
  readonly stdin: Readable
  writeStdout(line: string): void
  writeStderr(text: string): void
}

export const SEARCH_SERVER_USAGE = 'usage: critique search-server --root DIR [--max-matches N] [--max-bytes N]\n'

/**
 * `critique search-server`: the runner's MCP stdio server for one critic's
 * packet copy (ADR-0006). It answers `initialize`, `ping`, `tools/list` and
 * `tools/call` until stdin closes, then exits 0. It reads the packet and
 * nothing else: it writes no file, runs no command and opens no socket.
 * Arguments it cannot use exit 2; a root it cannot open exits 1.
 */
export async function runSearchServer(argv: readonly string[], io: SearchServerIo): Promise<number> {
  const options = readOptions(argv)
  if (options === undefined) {
    io.writeStderr(SEARCH_SERVER_USAGE)
    return 2
  }
  let engine: PacketSearchEngine
  try {
    engine = await PacketSearchEngine.open(options.root, options.limits)
  } catch (error) {
    io.writeStderr(`critique search-server: ${error instanceof Error ? error.message : String(error)}\n`)
    return 1
  }
  for await (const line of createInterface({ crlfDelay: Infinity, input: io.stdin })) {
    if (line.trim().length === 0) continue
    const reply = await answer(line, engine)
    if (reply !== undefined) io.writeStdout(`${JSON.stringify(reply)}\n`)
  }
  return 0
}

interface ServerOptions {
  readonly root: string
  readonly limits: SearchLimits
}

function readOptions(argv: readonly string[]): ServerOptions | undefined {
  let root: string | undefined
  let limits = DEFAULT_SEARCH_LIMITS
  for (let i = 0; i < argv.length; i += 2) {
    const flag = argv[i]
    const value = argv[i + 1]
    if (value === undefined) return undefined
    if (flag === '--root') root = value
    else if (flag === '--max-matches' || flag === '--max-bytes') {
      const number = Number(value)
      if (!Number.isInteger(number) || number < 1) return undefined
      limits = flag === '--max-matches' ? { ...limits, max_matches: number } : { ...limits, max_bytes: number }
    } else return undefined
  }
  return root === undefined ? undefined : { limits, root }
}

type JsonRpcId = string | number | null

/** The reply to one JSON-RPC line; a notification gets none. */
async function answer(line: string, engine: PacketSearchEngine): Promise<unknown> {
  let message: unknown
  try {
    message = JSON.parse(line) as unknown
  } catch {
    return failure(null, -32700, 'parse error')
  }
  if (!isJsonObject(message) || typeof message.method !== 'string') return failure(null, -32600, 'invalid request')
  if (!('id' in message)) return undefined
  const id = (typeof message.id === 'string' || typeof message.id === 'number' ? message.id : null) satisfies JsonRpcId
  const params = isJsonObject(message.params) ? message.params : {}
  const result = await resultOf(message.method, params, engine)
  return result === undefined ? failure(id, -32601, `method not found: ${message.method}`) : { id, jsonrpc: '2.0', result }
}

async function resultOf(method: string, params: Readonly<Record<string, unknown>>, engine: PacketSearchEngine): Promise<unknown> {
  if (method === 'initialize') {
    const version = typeof params.protocolVersion === 'string' ? params.protocolVersion : PROTOCOL_VERSION
    return { capabilities: { tools: {} }, protocolVersion: version, serverInfo: SERVER_INFO }
  }
  if (method === 'ping') return {}
  if (method === 'tools/list') return { tools: engine.tools.map((tool) => SEARCH_TOOL_DEFINITIONS[tool]) }
  if (method !== 'tools/call') return undefined
  const name = typeof params.name === 'string' ? params.name : ''
  const reply = await engine.call(name, isJsonObject(params.arguments) ? params.arguments : {})
  return { content: [{ text: reply.text, type: 'text' }], ...(reply.isError ? { isError: true } : {}) }
}

function failure(id: JsonRpcId, code: number, message: string): unknown {
  return { error: { code, message }, id, jsonrpc: '2.0' }
}
