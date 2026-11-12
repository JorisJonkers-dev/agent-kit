import { realpath } from 'node:fs/promises'
import { relative } from 'node:path'

import type { ProcessPort } from '../../../../ports/index.js'
import type { RepoMapPort } from '../../application/ports.js'
import type { RepoRanking } from '../../domain/ranking.js'

/** The longest the map may take: it walks a tree and answers in seconds. */
const MAP_TIMEOUT_MS = 30_000
/** The most files code-nav lists in one answer. */
const MAP_LIMIT = 200

const REQUESTS = [
  { id: 1, jsonrpc: '2.0', method: 'initialize', params: { capabilities: {}, clientInfo: { name: 'critique', version: '1' }, protocolVersion: '2025-06-18' } },
  { jsonrpc: '2.0', method: 'notifications/initialized' },
  { id: 2, jsonrpc: '2.0', method: 'tools/call', params: { arguments: { limit: MAP_LIMIT }, name: 'map' } },
]

/** A file line of the map's text: `<path> (<n> refs, <n> symbols)`. */
const FILE_LINE = /^(.+) \(\d+ refs, \d+ symbols\)$/

/**
 * Asks code-nav's `map` tool, over stdio the way an MCP client does, for a
 * tree's files ranked by how often their symbols are referenced. A code-nav
 * that is not installed, fails or answers something else is no error: the
 * ranking is then unavailable and the packet says so.
 */
export class CodeNavRepoMap implements RepoMapPort {
  constructor(
    private readonly process: ProcessPort,
    private readonly command = 'code-nav',
  ) {}

  async rank(root: string): Promise<RepoRanking> {
    const canonical = await realpath(root).catch(() => root)
    const result = await this.process
      .exec({
        args: ['--root', canonical, '--limit', String(MAP_LIMIT)],
        command: this.command,
        cwd: canonical,
        stdin: REQUESTS.map((request) => `${JSON.stringify(request)}\n`).join(''),
        timeoutMs: MAP_TIMEOUT_MS,
      })
      .catch((error: unknown) => (error instanceof Error ? error.message : String(error)))
    if (typeof result === 'string') return { detail: `code-nav did not start: ${result}`, status: 'unavailable' }
    if (result.timedOut === true) return { detail: 'code-nav did not answer in time', status: 'unavailable' }
    if (result.exitCode !== 0) {
      const reason = result.stderr.trim().split('\n')[0] ?? ''
      return { detail: `code-nav exited ${String(result.exitCode)}${reason === '' ? '' : `: ${reason}`}`, status: 'unavailable' }
    }
    const text = mapText(result.stdout)
    if (text === undefined) return { detail: 'code-nav answered no map', status: 'unavailable' }
    return { paths: filesOf(text, canonical), status: 'ranked' }
  }
}

/** The text of the map call's reply, or nothing when it errored or never came. */
function mapText(stdout: string): string | undefined {
  for (const line of stdout.split('\n')) {
    const reply = parsed(line)
    if (reply?.id !== 2) continue
    const result = reply.result
    if (typeof result !== 'object' || result === null || Array.isArray(result)) return undefined
    const fields = result as Readonly<Record<string, unknown>>
    if (fields.isError === true || !Array.isArray(fields.content)) return undefined
    return fields.content.map((part: unknown) => (typeof part === 'object' && part !== null && 'text' in part && typeof part.text === 'string' ? part.text : '')).join('')
  }
  return undefined
}

function parsed(line: string): Readonly<Record<string, unknown>> | undefined {
  try {
    const value: unknown = JSON.parse(line)
    return typeof value === 'object' && value !== null && !Array.isArray(value) ? (value as Readonly<Record<string, unknown>>) : undefined
  } catch {
    return undefined
  }
}

/** The files section of the map's text, as repository paths in rank order. */
function filesOf(text: string, root: string): readonly string[] {
  const paths: string[] = []
  let inFiles = false
  for (const line of text.split('\n')) {
    if (line === 'files:') inFiles = true
    else if (line === 'symbols:') break
    else if (inFiles) {
      const path = FILE_LINE.exec(line)?.[1]
      const inside = path === undefined ? undefined : relative(root, path)
      if (inside !== undefined && inside !== '' && !inside.startsWith('..')) paths.push(inside.split('\\').join('/'))
    }
  }
  return paths
}
