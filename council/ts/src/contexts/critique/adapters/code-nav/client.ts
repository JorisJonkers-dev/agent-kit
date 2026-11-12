import { realpath } from 'node:fs/promises'
import { isAbsolute, join, relative } from 'node:path'

import type { ProcessPort } from '../../../../ports/index.js'
import type { RepoMapPort } from '../../application/ports.js'
import type { RankedFile, RankedSymbol, RepoRanking } from '../../domain/ranking.js'

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
const FILE_LINE = /^(.+) \((\d+) refs, \d+ symbols\)$/
/** A symbol line of the map's text: `<path>:<line> <kind> <name> (<n> refs)`. */
const SYMBOL_LINE = /^(.+):(\d+) (\S+) (.+) \((\d+) refs\)$/

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
    return { ...mapOf(text, canonical), status: 'ranked' }
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

/** The files and the symbols of the map's text, in rank order, as repository paths inside the root. */
function mapOf(text: string, root: string): { readonly files: readonly RankedFile[]; readonly symbols: readonly RankedSymbol[] } {
  const files: RankedFile[] = []
  const symbols: RankedSymbol[] = []
  let section: 'files' | 'symbols' | undefined
  for (const line of text.split('\n')) {
    if (line === 'files:' || line === 'symbols:') {
      section = line === 'files:' ? 'files' : 'symbols'
      continue
    }
    if (section === 'files') {
      const found = FILE_LINE.exec(line)
      const path = found?.[1] === undefined ? undefined : inside(root, found[1])
      if (path !== undefined) files.push({ path, refs: Number(found?.[2]) })
    } else if (section === 'symbols') {
      const found = SYMBOL_LINE.exec(line)
      const path = found?.[1] === undefined ? undefined : inside(root, found[1])
      if (found !== null && path !== undefined) symbols.push({ kind: found[3] ?? '', line: Number(found[2]), name: found[4] ?? '', path, refs: Number(found[5]) })
    }
  }
  return { files, symbols }
}

/** A path the map printed, as a repository path; nothing for one outside the root. */
function inside(root: string, path: string): string | undefined {
  const repository = relative(root, isAbsolute(path) ? path : join(root, path))
  return repository === '' || repository === '..' || repository.startsWith('../') || repository.startsWith('..\\') ? undefined : repository.split('\\').join('/')
}
