import { mkdir, mkdtemp, realpath, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Readable } from 'node:stream'

import { afterEach, describe, expect, it } from 'vitest'

import { FakeEnv } from '../../../test/critique/harness.js'
import type { ProcessCommand, ProcessPort, ProcessResult } from '../../ports/index.js'
import { mcpServerNames } from './adapters/vibe/index.js'
import { CriticTools } from './application/critic-tools.js'
import { reachProblems } from './domain/packet-search.js'
import type { RosterCritic } from './domain/routing.js'
import { NodeCritiqueFiles, ProcessSearchClient, runSearchServer, SEARCH_SERVER_USAGE, type CriticClients, type SearchClientPort } from './index.js'

/**
 * `critique search-server`, the runner's MCP stdio server (ADR-0006), driven
 * as a client drives it: JSON-RPC lines on stdin, answers on stdout.
 */

const roots: string[] = []

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { force: true, recursive: true })))
})

const SECRET = 'OUTSIDE-SECRET-CANARY'

/** A packet with a few files, beside an outside directory holding the secret, and symlinks out of the packet. */
async function fixture(): Promise<{ readonly packet: string; readonly outside: string }> {
  const base = await realpath(await mkdtemp(join(tmpdir(), 'search-server-')))
  roots.push(base)
  const packet = join(base, 'packet')
  const outside = join(base, 'outside')
  await mkdir(join(packet, 'tree', 'src'), { recursive: true })
  await mkdir(outside, { recursive: true })
  await writeFile(join(packet, 'artifact.md'), 'alpha line\nbeta line\nAlpha again\n')
  await writeFile(join(packet, 'tree', 'src', 'a.ts'), `export const a = 1\n${'x'.repeat(400)}\n`)
  await writeFile(join(outside, 'secret.md'), `${SECRET} alpha\n`)
  await symlink('../outside', join(packet, 'escape'))
  await symlink(join(outside, 'secret.md'), join(packet, 'link.md'))
  await symlink(join(outside, 'secret.md'), join(packet, 'tree', 'REPO-MAP.md'))
  return { outside, packet }
}

interface Served {
  readonly code: number
  readonly replies: readonly Record<string, unknown>[]
  readonly stderr: string
}

async function serve(argv: readonly string[], lines: readonly unknown[]): Promise<Served> {
  const stdout: string[] = []
  let stderr = ''
  const code = await runSearchServer(argv, {
    stdin: Readable.from([lines.map((line) => (typeof line === 'string' ? line : JSON.stringify(line))).join('\n')]),
    writeStderr: (text) => {
      stderr += text
    },
    writeStdout: (line) => stdout.push(line),
  })
  return { code, replies: stdout.map((line) => JSON.parse(line) as Record<string, unknown>), stderr }
}

const call = (id: number, name: string, args: Record<string, unknown>) => ({ id, jsonrpc: '2.0', method: 'tools/call', params: { arguments: args, name } })

/** The text of each tools/call answer, in order. */
async function answers(root: string, calls: readonly [string, Record<string, unknown>][], limits: readonly string[] = []): Promise<readonly string[]> {
  const served = await serve(['--root', root, ...limits], calls.map(([name, args], index) => call(index + 1, name, args)))
  return served.replies.map((reply) => ((reply.result as { content: { text: string }[] }).content[0]?.text ?? ''))
}

describe('the search server stays inside its packet', () => {
  it('finds nothing through `..`, an absolute path, or a symlink out of the packet', async () => {
    const { outside, packet } = await fixture()

    const found = await answers(packet, [
      ['grep', { pattern: 'alpha' }],
      ['grep', { path: '..', pattern: 'alpha' }],
      ['grep', { path: '../outside', pattern: SECRET }],
      ['grep', { path: outside, pattern: SECRET }],
      ['grep', { path: 'escape', pattern: SECRET }],
      ['grep', { path: 'link.md', pattern: SECRET }],
      ['grep', { path: 'escape/secret.md', pattern: SECRET }],
      ['list', { glob: '**' }],
      ['list', { glob: '../**' }],
      ['list', { glob: `${outside}/**` }],
    ])

    expect(found[0]).toBe('artifact.md:1: alpha line')
    expect(found.slice(1, 7)).toEqual(Array(6).fill('Nothing: that path is outside the packet, or is not in it.'))
    expect(found[7]).toBe('artifact.md\ntree/src/a.ts')
    expect(found.slice(8)).toEqual(['No paths match.', 'No paths match.'])
    expect(found.join('\n')).not.toContain(SECRET)
  })

  it('offers no symbol tool when the repo map is a symlink, not a file the packet holds', async () => {
    const { packet } = await fixture()

    const served = await serve(['--root', packet], [{ id: 1, jsonrpc: '2.0', method: 'tools/list' }, call(2, 'symbol', { name: 'a' })])

    const tools = (served.replies[0]?.result as { tools: { name: string }[] }).tools.map((tool) => tool.name)
    expect(tools).toEqual(['grep', 'list'])
    expect(served.replies[1]?.result).toEqual({ content: [{ text: 'unknown tool symbol', type: 'text' }], isError: true })
  })
})

describe('the search server answers a client', () => {
  it('speaks MCP: initialize, ping, tools/list and tools/call, and ignores notifications', async () => {
    const { packet } = await fixture()

    const served = await serve(
      ['--root', packet],
      [
        { id: 1, jsonrpc: '2.0', method: 'initialize', params: { protocolVersion: '2025-11-25' } },
        { id: 'two', jsonrpc: '2.0', method: 'initialize' },
        { jsonrpc: '2.0', method: 'notifications/initialized' },
        '',
        { id: 3, jsonrpc: '2.0', method: 'ping' },
        { id: 4, jsonrpc: '2.0', method: 'resources/list' },
        'not json',
        { id: 5, jsonrpc: '2.0' },
        { id: 6, jsonrpc: '2.0', method: 'tools/call' },
        { id: {}, jsonrpc: '2.0', method: 'ping' },
      ],
    )

    expect(served.code).toBe(0)
    expect(served.replies).toEqual([
      { id: 1, jsonrpc: '2.0', result: { capabilities: { tools: {} }, protocolVersion: '2025-11-25', serverInfo: { name: 'critique-packet-search', version: '1' } } },
      { id: 'two', jsonrpc: '2.0', result: { capabilities: { tools: {} }, protocolVersion: '2025-06-18', serverInfo: { name: 'critique-packet-search', version: '1' } } },
      { id: 3, jsonrpc: '2.0', result: {} },
      { error: { code: -32601, message: 'method not found: resources/list' }, id: 4, jsonrpc: '2.0' },
      { error: { code: -32700, message: 'parse error' }, id: null, jsonrpc: '2.0' },
      { error: { code: -32600, message: 'invalid request' }, id: null, jsonrpc: '2.0' },
      { id: 6, jsonrpc: '2.0', result: { content: [{ text: 'unknown tool ', type: 'text' }], isError: true } },
      { id: null, jsonrpc: '2.0', result: {} },
    ])
  })

  it('narrows grep by path, glob and case, clips long lines, and reports bad arguments as tool errors', async () => {
    const { packet } = await fixture()

    const served = await serve(
      ['--root', packet],
      [
        call(1, 'grep', { ignore_case: true, pattern: '^alpha' }),
        call(2, 'grep', { glob: 'tree/**', pattern: '^x+$' }),
        call(3, 'grep', { path: 'tree', pattern: 'export' }),
        call(4, 'grep', { pattern: '(' }),
        call(5, 'grep', { pattern: '' }),
        call(6, 'grep', { path: 7, pattern: 'a' }),
        call(7, 'grep', { pattern: 'no such text' }),
        call(8, 'list', { glob: 'tree/**/*.md' }),
      ],
    )

    const text = served.replies.map((reply) => reply.result as { content: { text: string }[]; isError?: boolean })
    expect(text[0]?.content[0]?.text).toBe('artifact.md:1: alpha line\nartifact.md:3: Alpha again')
    expect(text[1]?.content[0]?.text).toBe(`tree/src/a.ts:2: ${'x'.repeat(300)}…`)
    expect(text[2]?.content[0]?.text).toBe('tree/src/a.ts:1: export const a = 1')
    expect(text[3]).toMatchObject({ isError: true })
    expect(text[3]?.content[0]?.text).toContain('Invalid regular expression')
    expect(text[4]).toEqual({ content: [{ text: 'pattern must be a non-empty string', type: 'text' }], isError: true })
    expect(text[5]).toEqual({ content: [{ text: 'path must be a string', type: 'text' }], isError: true })
    expect(text[6]?.content[0]?.text).toBe('No matches.')
    expect(text[7]?.content[0]?.text).toBe('No paths match.')
  })

  it('cuts an answer at the match cap and at the byte cap, and says how many were left out', async () => {
    const { packet } = await fixture()

    const byMatches = await answers(packet, [['grep', { pattern: 'line|again' }]], ['--max-matches', '2'])
    const byBytes = await answers(packet, [['grep', { pattern: 'line|again' }]], ['--max-bytes', '40'])

    expect(byMatches).toEqual(['artifact.md:1: alpha line\nartifact.md:2: beta line\n[1 more matches left out; narrow the pattern, path or glob]'])
    expect(byBytes).toEqual(['artifact.md:1: alpha line\n[2 more matches left out; narrow the pattern, path or glob]'])
  })

  it('looks a symbol up in the repo map: an exact name first, else every name containing it', async () => {
    const { packet } = await fixture()
    await rm(join(packet, 'tree', 'REPO-MAP.md'))
    await writeFile(
      join(packet, 'tree', 'REPO-MAP.md'),
      [
        '# Repo map',
        '',
        '- `stray` (function, line 1, 0 refs): `before any file`',
        '',
        '## `src/a.ts` (referenced by 2 files)',
        '',
        '- `a` (const, line 1, 2 refs): `export const a = 1`',
        '- `alpha` (function, line 9, 0 refs): `export function alpha(): void`',
        'not a symbol line',
        '',
      ].join('\n'),
    )

    const found = await answers(packet, [
      ['symbol', { name: 'a' }],
      ['symbol', { name: 'ALP' }],
      ['symbol', { name: 'stray' }],
      ['symbol', {}],
    ])

    expect(found).toEqual([
      'tree/src/a.ts:1: const a, 2 refs: export const a = 1',
      'tree/src/a.ts:9: function alpha, 0 refs: export function alpha(): void',
      'No symbol of that name.',
      'name must be a non-empty string',
    ])
  })
})

describe('the search server refuses what it cannot use', () => {
  it.each([[[]], [['--root']], [['--root', '/x', '--max-matches', '0']], [['--root', '/x', '--max-bytes', 'many']], [['--color', 'yes']]])(
    'prints its usage and exits 2 for %j',
    async (argv) => {
      const served = await serve(argv, [])

      expect([served.code, served.stderr, served.replies]).toEqual([2, SEARCH_SERVER_USAGE, []])
    },
  )

  it('exits 1 for a root that is not a directory', async () => {
    const { packet } = await fixture()

    const missing = await serve(['--root', join(packet, 'nowhere')], [])
    const file = await serve(['--root', join(packet, 'artifact.md')], [])

    expect([missing.code, missing.stderr]).toEqual([1, expect.stringContaining('ENOENT')])
    expect([file.code, file.stderr]).toEqual([1, expect.stringContaining('is not a directory')])
  })
})

/** A process port that answers every command with one canned result, or throws. */
class CannedProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []

  constructor(private readonly answer: ProcessResult | Error) {}

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    if (this.answer instanceof Error) throw this.answer
    return Promise.resolve(this.answer)
  }
}

describe('the runner’s session with its search server', () => {
  const client = (answer: ProcessResult | Error) => new ProcessSearchClient(new CannedProcess(answer), { args: ['/k/critique.mjs'], command: '/bin/node' })
  const limits = { max_bytes: 10, max_matches: 2 }

  it.each<[string, ProcessResult | Error, string]>([
    ['cannot be spawned', new Error('spawn /bin/node ENOENT'), 'the search server did not start: spawn /bin/node ENOENT'],
    ['crashes on a thrown string', 'boom' as unknown as Error, 'the search server did not start: boom'],
    ['never answers', { exitCode: 124, stderr: '', stdout: '', timedOut: true }, 'the search server did not answer in time'],
    ['exits non-zero', { exitCode: 3, stderr: '', stdout: '' }, 'the search server exited 3'],
    ['lists no tools', { exitCode: 0, stderr: '', stdout: 'garbage\n{"id":2,"result":{"tools":"none"}}\n' }, 'the search server listed no search tools'],
  ])('fails when the server %s', async (_name, answer, detail) => {
    expect(await client(answer).session('/p', limits, [])).toEqual({ detail, status: 'failed' })
  })

  it('reads each answer, an error and a missing one included', async () => {
    const stdout = [
      '{"id":2,"result":{"tools":[{"name":"grep"},{"name":"list"},{"name":"other"}]}}',
      '{"id":3,"error":{"message":"bad"}}',
      '{"id":4,"result":{}}',
      '[]',
      '',
    ].join('\n')
    const port = new CannedProcess({ exitCode: 0, stderr: '', stdout })
    const session = await new ProcessSearchClient(port, { args: [], command: 'node' }).session('/p', limits, [
      { arguments: {}, tool: 'grep' },
      { arguments: {}, tool: 'list' },
      { arguments: {}, tool: 'list' },
    ])

    expect(session).toEqual({ answers: ['error: bad', 'error: no answer', 'error: no answer'], status: 'started', tools: ['grep', 'list'] })
    expect(port.commands[0]).toMatchObject({ args: ['search-server', '--root', '/p', '--max-matches', '2', '--max-bytes', '10'], command: 'node', cwd: '/p' })
  })

  it('stops a leftover server by its command line, and shrugs off a missing pkill', async () => {
    const port = new CannedProcess(new Error('spawn pkill ENOENT'))

    await new ProcessSearchClient(port, { args: [], command: 'node' }).stop('/run/c1.x/packet')

    expect(port.commands[0]).toMatchObject({ args: ['-f', '--', 'search-server --root /run/c1\\.x/packet( |$)'], command: 'pkill' })
  })
})

describe('findings of review 20261102T124643Z-ticket-review, as the critics wrote them', () => {
  // F5
  it('leaves the mcp_servers array once its line ends', () => {
    expect(mcpServerNames('mcp_servers = [{ name = "a", transport = "stdio", command = "x" }]\nname = "work"\n')).toEqual(['a'])
  })

  // F3
  it('degrades a critic to read_file when the reach proof rejects, and does not cache the rejection', async () => {
    let symlinkCalls = 0
    class BrokenSymlinks extends NodeCritiqueFiles {
      override async symlink(): Promise<void> {
        symlinkCalls += 1
        return Promise.reject(new Error('EPERM: operation not permitted, symlink'))
      }
    }
    const search: SearchClientPort = {
      server: () => ({ args: [], command: 'node' }),
      session: () => Promise.resolve({ answers: [], status: 'started', tools: ['grep', 'list'] }),
      stop: () => Promise.resolve(),
    }
    // Helper fix: CriticTools now also asks the Vibe client whether its agent file lets it search.
    const clients = { vibe: { searchBlocked: () => Promise.resolve(undefined) } } as unknown as CriticClients
    const base = await realpath(await mkdtemp(join(tmpdir(), 'critic-tools-')))
    roots.push(base)
    const tools = new CriticTools({ clients, env: new FakeEnv({ HOME: base }), files: new BrokenSymlinks(), search }, { max_bytes: 8192, max_matches: 50 }, join(base, 'proof'))
    const critic: RosterCritic = {
      client: 'vibe-glm',
      client_config: { agent: 'critic-glm', binary: 'vibe', family: 'zhipu', kind: 'vibe', model: 'glm' },
      critic_id: 'c1',
      fallbacks: [],
      lenses: [],
      max_price_usd: 0.5,
      role: 'r',
      time_limit_s: 60,
    }

    await expect(tools.choose(critic, '/packet')).resolves.toMatchObject({ toolset: { kind: 'vibe-read' } })
    // Regression check beside the critic's test: the next critic proves again, and says why it reads alone.
    const again = await tools.choose(critic, '/packet')
    expect(again).toEqual({
      searchFailure: 'search is not proven to stay inside the packet: EPERM: operation not permitted, symlink',
      toolset: { kind: 'vibe-read' },
    })
    expect(symlinkCalls).toBe(2)
  })
})

describe('the reach proof', () => {
  it('names each answer that quotes or names the outside file, and a search that found nothing inside', () => {
    expect(reachProblems(['inside.md:1: reach proof', 'secret.md', 'x: OUTSIDE-THE-PACKET'])).toEqual([
      'call 2 returned text from outside the packet',
      'call 3 returned text from outside the packet',
    ])
    expect(reachProblems([])).toEqual(['a search of the whole packet did not find the file inside it'])
  })
})
