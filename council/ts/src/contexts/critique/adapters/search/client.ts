import type { ProcessPort } from '../../../../ports/index.js'
import type { SearchClientPort, SearchServerCommand, SearchServerLaunch, SearchSession } from '../../application/ports.js'
import { jsonObjectOrUndefined } from '../../domain/json.js'
import { SEARCH_TOOLS, type SearchCall, type SearchLimits, type SearchToolName } from '../../domain/packet-search.js'

/** The longest a session with the server may take: it answers in milliseconds. */
const SESSION_TIMEOUT_MS = 20_000

/**
 * Starts the runner's own search server (`critique search-server`) the way
 * an MCP client does, over stdio. The command is the runner's: the Node
 * binary and the bundled `critique.mjs`.
 */
export class ProcessSearchClient implements SearchClientPort {
  constructor(
    private readonly process: ProcessPort,
    private readonly self: SearchServerCommand,
  ) {}

  server(root: string, limits: SearchLimits): SearchServerLaunch {
    return {
      args: [
        ...this.self.args,
        'search-server',
        '--root',
        root,
        '--max-matches',
        String(limits.max_matches),
        '--max-bytes',
        String(limits.max_bytes),
      ],
      command: this.self.command,
    }
  }

  async session(root: string, limits: SearchLimits, calls: readonly SearchCall[]): Promise<SearchSession> {
    const launch = this.server(root, limits)
    const requests = [
      { id: 1, jsonrpc: '2.0', method: 'initialize', params: { capabilities: {}, clientInfo: { name: 'critique', version: '1' }, protocolVersion: '2025-06-18' } },
      { jsonrpc: '2.0', method: 'notifications/initialized' },
      { id: 2, jsonrpc: '2.0', method: 'tools/list' },
      ...calls.map((call, index) => ({
        id: index + 3,
        jsonrpc: '2.0',
        method: 'tools/call',
        params: { arguments: call.arguments, name: call.tool },
      })),
    ]
    const result = await this.process
      .exec({ ...launch, cwd: root, stdin: requests.map((request) => `${JSON.stringify(request)}\n`).join(''), timeoutMs: SESSION_TIMEOUT_MS })
      .catch((error: unknown) => (error instanceof Error ? error.message : String(error)))
    if (typeof result === 'string') return { detail: `the search server did not start: ${result}`, status: 'failed' }
    if (result.timedOut === true) return { detail: 'the search server did not answer in time', status: 'failed' }
    if (result.exitCode !== 0) {
      const reason = result.stderr.trim().split('\n')[0] ?? ''
      return { detail: `the search server exited ${String(result.exitCode)}${reason === '' ? '' : `: ${reason}`}`, status: 'failed' }
    }
    const replies = new Map<unknown, Readonly<Record<string, unknown>>>()
    for (const line of result.stdout.split('\n').filter((text) => text.trim().length > 0)) {
      const reply = parsed(line)
      if (reply !== undefined) replies.set(reply.id, reply)
    }
    const listed = jsonObjectOrUndefined(replies.get(2)?.result)?.tools
    const tools = Array.isArray(listed) ? listed.map((tool) => jsonObjectOrUndefined(tool)?.name) : []
    const offered = SEARCH_TOOLS.filter((tool) => tools.includes(tool))
    if (offered.length === 0) return { detail: 'the search server listed no search tools', status: 'failed' }
    return { answers: calls.map((_call, index) => answerText(replies.get(index + 3))), status: 'started', tools: offered satisfies readonly SearchToolName[] }
  }

  /**
   * Stops every search server still running on `root`, by its command line:
   * no other process carries `search-server --root <root>`, and one critic's
   * packet path is never a prefix of another's followed by a space.
   */
  async stop(root: string): Promise<void> {
    const pattern = `search-server --root ${root.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}( |$)`
    await this.process.exec({ args: ['-f', '--', pattern], command: 'pkill', timeoutMs: SESSION_TIMEOUT_MS }).catch(() => undefined)
  }
}

function parsed(line: string): Readonly<Record<string, unknown>> | undefined {
  try {
    return jsonObjectOrUndefined(JSON.parse(line))
  } catch {
    return undefined
  }
}

/** A call's text, or what went wrong with it. */
function answerText(reply: Readonly<Record<string, unknown>> | undefined): string {
  const error = jsonObjectOrUndefined(reply?.error)
  if (error !== undefined) return `error: ${String(error.message)}`
  const content = jsonObjectOrUndefined(reply?.result)?.content
  if (!Array.isArray(content)) return 'error: no answer'
  return content
    .map((part) => jsonObjectOrUndefined(part)?.text)
    .map((text) => (typeof text === 'string' ? text : ''))
    .join('')
}
