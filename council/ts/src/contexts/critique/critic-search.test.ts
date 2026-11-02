import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  agyFindings,
  codeStageRouting,
  fenced,
  finding,
  harness,
  isolatedExport,
  launchServers,
  SEARCH_COMMAND,
  searchExport,
  SPEC_CANARY,
  vibeStdout,
  type Harness,
  type SearchHelper,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { FanOutResult, PrepareResult } from './index.js'

/**
 * Critics search their own packet with read-only tools (ADR-0006, #255).
 * A Vibe critic searches through the runner's own MCP server, confined to
 * its packet copy; an agy critic uses agy's own read-only tools.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const PACKET = (run: PrepareResult, critic = 'c1-vibe-mistral-large') => join(run.run_dir, 'critics', critic, 'packet')

const HOME_CONFIG = [
  'active_model = "mistral-large"',
  '',
  '[[mcp_servers]]',
  'name = "hindsight"',
  'transport = "streamable-http"',
  'url = "https://example.invalid/mcp"',
  '',
  '[[mcp_servers]]',
  "name = 'packet'",
  'transport = "stdio"',
  'command = "rm"',
  '',
  '[tools.bash]',
  'name = "not-a-server"',
  '',
].join('\n')

/** A critic agent file as setup installed it before #255: MCP off, `read_file` alone. */
const AGENT_BEFORE_255 = [
  'display_name = "Critic Mistral Large"',
  'safety = "safe"',
  'active_model = "critic-mistral-large"',
  'enabled_tools = ["read_file"]',
  'outside_workdir = "deny"',
  'enable_mcp = false',
  'include_project_context = false',
  '',
  '[session_logging]',
  'enabled = false',
  '',
].join('\n')

/** A Vibe critic that found nothing, reporting the effective config its launch asked for. */
const searching = (overrides: Partial<VibeScript> = {}): VibeScript => ({
  exitCode: 0,
  exportJson: searchExport(),
  stdout: vibeStdout(fenced({ findings: [finding()] })),
  ...overrides,
})

async function specRun(
  options: Parameters<typeof harness>[0] = {},
  families: readonly string[] = ['mistral'],
): Promise<{ h: Harness; run: PrepareResult }> {
  const h = await harness({ search: true, vibeConfig: HOME_CONFIG, ...options })
  open.push(h)
  await h.commands.allow.run({ families, repo: h.repo })
  const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
  if (run.status !== 'ready') throw new Error('not ready')
  return { h, run }
}

async function fanOut(h: Harness, run: PrepareResult, ...scripts: VibeScript[]): Promise<FanOutResult> {
  h.process.queue(...scripts)
  return h.commands.fanOut.run({ run_dir: run.run_dir })
}

/** Records what the critic's search calls answer while it runs. */
function recorder(calls: readonly Parameters<SearchHelper>[0][]): { readonly answers: string[]; readonly effect: NonNullable<VibeScript['effect']> } {
  const answers: string[] = []
  return {
    answers,
    effect: async ({ search }) => {
      for (const call of calls) answers.push(await search(call))
    },
  }
}

function promptOf(h: Harness, index = 0): Promise<string> {
  const args = h.process.commands[index]?.args ?? []
  return readFile(args[args.indexOf('--prompt-file') + 1] ?? '', 'utf8')
}

const enabledTools = (args: readonly string[]) => args.flatMap((arg, i) => (args[i - 1] === '--enabled-tools' ? [arg] : []))

describe('a Vibe critic searches its packet copy through the runner’s server', () => {
  it('starts the server confined to the critic’s own packet copy, as the only live MCP server', async () => {
    const { h, run } = await specRun()

    const result = await fanOut(h, run, searching())

    const [launch] = h.process.commands
    const packet = PACKET(run)
    expect(launch?.env).toMatchObject({ VIBE_ENABLE_CONNECTORS: 'false', VIBE_ENABLE_MCP: 'true' })
    const servers = launchServers(launch)
    expect(servers.filter((server) => server.disabled !== true)).toEqual([
      {
        args: [...SEARCH_COMMAND.args, 'search-server', '--root', packet, '--max-matches', '50', '--max-bytes', '8192'],
        command: SEARCH_COMMAND.command,
        cwd: packet,
        name: 'packet',
        startup_timeout_sec: 10,
        tool_timeout_sec: 30,
        transport: 'stdio',
      },
    ])
    // The home's own servers are switched off by name, so none of them starts.
    expect(servers.filter((server) => server.disabled === true).map((server) => [server.name, server.command])).toEqual([['hindsight', 'false']])
    expect(enabledTools(launch?.args ?? [])).toEqual(['read_file', 'packet_grep', 'packet_list'])
    expect(result.critics[0]).toMatchObject({ status: 'ok', tools: ['read_file', 'run_typescript', 'mcp_packet.grep', 'mcp_packet.list'] })
    expect(result.critics[0]?.search_failure).toBeUndefined()
  })

  it('switches off the servers a home declares as an array of inline tables too', async () => {
    const { h, run } = await specRun({
      vibeConfig: 'mcp_servers = [\n  { name = "inline-one", transport = "stdio", command = "x" },\n  { name = \'inline-two\', url = "u" },\n]\n',
    })

    await fanOut(h, run, searching())

    const off = launchServers(h.process.commands[0]).filter((server) => server.disabled === true)
    expect(off.map((server) => server.name)).toEqual(['inline-one', 'inline-two'])
  })

  it('answers grep and list with packet text only', async () => {
    const { h, run } = await specRun()
    const critic = recorder([
      { arguments: { pattern: 'frobnicated' }, tool: 'grep' },
      { arguments: { glob: '**' }, tool: 'list' },
      { arguments: { pattern: 'canary', path: '..' }, tool: 'grep' },
      { arguments: { pattern: 'canary', path: 'artifact/spec.md' }, tool: 'grep' },
      { arguments: { pattern: 'canary', path: 'artifact/nowhere.md' }, tool: 'grep' },
    ])

    const result = await fanOut(h, run, searching({ effect: critic.effect }))

    expect(critic.answers).toEqual([
      `artifact/spec.md:7: 3. ${SPEC_CANARY}\nreferences/CONTEXT.md:3: Decided: widgets are frobnicated by the nightly job only.`,
      'artifact/spec.md\nreferences/CONTEXT.md',
      'Nothing: that path is outside the packet, or is not in it.',
      `artifact/spec.md:7: 3. ${SPEC_CANARY}`,
      'Nothing: that path is outside the packet, or is not in it.',
    ])
    expect(result.critics[0]?.status).toBe('ok')
  })

  it('names exactly the critic’s tools in its prompt: grep and list, without a repo map', async () => {
    const { h, run } = await specRun()

    await fanOut(h, run, searching())

    const prompt = await promptOf(h)
    expect(prompt).toContain('call from `run_typescript` as `tools.mcp_packet.<function>`')
    expect(prompt).toContain('function grep(args:')
    expect(prompt).toContain('function list(args:')
    expect(prompt).not.toContain('function symbol(')
    expect(prompt).toContain('An answer holds at most 50 matches and 8192 bytes')
    expect(prompt).not.toContain('You cannot search')
  })

  it('offers symbol on a code packet, which holds a repo map, and answers it from the map', async () => {
    const h = await harness({ routing: codeStageRouting(), search: true })
    open.push(h)
    await h.commands.allow.run({ code: true, families: ['mistral'], repo: h.repo })
    await h.writeText(join(h.repo, '.scratch/widgets/issues/01-frobnicate.md'), '# 01: Frobnicate widgets\n')
    await h.writeText(join(h.repo, 'src/count.ts'), 'export function count(widgets: number[]): number {\n  return widgets.length\n}\n')
    await h.writeText(join(h.repo, 'src/widget.ts'), "import { count } from './count.js'\n\nexport function frobnicate(w: number[]): number {\n  return count(w)\n}\n")
    await h.git.commit()
    await h.writeText(join(h.repo, 'src/widget.ts'), "import { count } from './count.js'\n\nexport function frobnicate(w: number[]): number {\n  return count(w) - 1\n}\n")
    await h.git.commit()
    const run = await h.commands.prepare.run({
      base: 'HEAD~1',
      orchestrator_family: 'anthropic',
      repo: h.repo,
      spec: '.scratch/widgets/spec.md',
      stage: 'ticket-review',
      ticket: '.scratch/widgets/issues/01-frobnicate.md',
    })
    if (run.status !== 'ready') throw new Error('not ready')
    const critic = recorder([
      { arguments: { name: 'count' }, tool: 'symbol' },
      { arguments: { name: 'FROB' }, tool: 'symbol' },
      { arguments: { name: 'nothing-like-it' }, tool: 'symbol' },
      { arguments: { glob: 'tree/src/*.ts' }, tool: 'list' },
    ])

    const result = await fanOut(h, run, searching({ effect: critic.effect, stdout: vibeStdout(fenced({ findings: [], no_findings_reason: 'Read the change.' })) }))

    expect(critic.answers).toEqual([
      'tree/src/count.ts:1: function count, 1 refs: export function count(widgets: number[]): number',
      'tree/src/widget.ts:3: function frobnicate, 0 refs: export function frobnicate(w: number[]): number',
      'No symbol of that name.',
      'tree/src/count.ts\ntree/src/widget.ts',
    ])
    expect(enabledTools(h.process.commands[0]?.args ?? [])).toEqual(['read_file', 'packet_grep', 'packet_list', 'packet_symbol'])
    expect(result.critics[0]?.tools).toEqual(['read_file', 'run_typescript', 'mcp_packet.grep', 'mcp_packet.list', 'mcp_packet.symbol'])
    expect(await promptOf(h)).toContain('function symbol(args: { name: string }): Promise<string>')
  })

  it('cuts an answer to the limits routing sets, which an override changes, and says how many were left out', async () => {
    const { h, run } = await specRun({ routingOverride: { search: { max_bytes: 8192, max_matches: 1 } } })
    const critic = recorder([
      { arguments: { pattern: 'widget', ignore_case: true }, tool: 'grep' },
      { arguments: { glob: '**/*.md' }, tool: 'list' },
    ])

    await fanOut(h, run, searching({ effect: critic.effect }))

    expect(critic.answers).toEqual([
      'artifact/spec.md:1: # Widgets\n[3 more matches left out; narrow the pattern, path or glob]',
      'artifact/spec.md\n[1 more paths left out; narrow the pattern, path or glob]',
    ])
    expect(await promptOf(h)).toContain('An answer holds at most 1 matches and 8192 bytes')
  })

  it('keeps the routed limits in the run, so they hold for the whole run', async () => {
    const { run } = await specRun({ routingOverride: { search: { max_bytes: 100, max_matches: 7 } } })

    const manifest = JSON.parse(await readFile(run.manifest_path, 'utf8')) as { limits: { search: unknown } }
    expect(manifest.limits.search).toEqual({ max_bytes: 100, max_matches: 7 })
  })

  it('stops the server once the critic ends, and once the outer timer kills it', async () => {
    const { h, run } = await specRun({
      routing: {
        clients: {
          'vibe-glm': { agent: 'critic-glm', binary: 'vibe', family: 'zhipu', kind: 'vibe', model: 'glm-5' },
          'vibe-mistral-large': { agent: 'critic-mistral-large', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-large-4-0' },
        },
        stages: {
          spec: {
            base: [
              { client: 'vibe-mistral-large', lenses: ['fidelity'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 },
              { client: 'vibe-glm', lenses: ['fidelity'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 },
            ],
            distinct_families: true,
          },
        },
        version: 1,
      },
    }, ['mistral', 'zhipu'])

    const result = await fanOut(h, run, searching({ agent: 'critic-mistral-large' }), { agent: 'critic-glm', exitCode: 124, stdout: '', timedOut: true })

    expect(result.critics.map((critic) => critic.status)).toEqual(['ok', 'failed'])
    expect(h.process.stops.map((stop) => stop.args).sort()).toEqual(
      [PACKET(run), PACKET(run, 'c2-vibe-glm')].map((packet) => ['-f', '--', `search-server --root ${packet.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}( |$)`]).sort(),
    )
  })
})

describe('a Vibe critic whose search cannot be had reads alone', () => {
  it('runs on read_file with the read-only prompt when the server does not start, and says why', async () => {
    const { h, run } = await specRun()
    h.process.searchBehaviour = 'broken'

    const result = await fanOut(h, run, {
      exitCode: 0,
      exportJson: searchExport({ enable_mcp: false, enabled_tools: ['read_file'] }),
      stdout: vibeStdout(fenced({ findings: [finding()] })),
    })

    const [launch] = h.process.commands
    expect(launch?.env?.VIBE_ENABLE_MCP).toBe('false')
    expect(launch?.env?.VIBE_MCP_SERVERS).toBeUndefined()
    expect(enabledTools(launch?.args ?? [])).toEqual(['read_file'])
    expect(h.process.stops).toEqual([])
    const prompt = await promptOf(h)
    expect(prompt).toContain('You have one tool: `read_file`')
    expect(prompt).toContain('You cannot search')
    expect(result.critics[0]).toMatchObject({ status: 'ok', tools: ['read_file'] })
    expect(result.critics[0]?.search_failure).toBe(
      'search is not proven to stay inside the packet: the search server exited 1: Error: Cannot find module critique.mjs',
    )
  })

  it('runs on read_file when the server proves its reach but does not start on the critic’s copy', async () => {
    const { h, run } = await specRun()
    h.process.searchBehaviour = 'broken-on-copies'

    const result = await fanOut(h, run, { exitCode: 0, exportJson: searchExport({ enable_mcp: false }), stdout: vibeStdout(fenced({ findings: [finding()] })) })

    expect(result.critics[0]).toMatchObject({
      search_failure: 'the search server exited 1: Error: Cannot find module critique.mjs',
      status: 'ok',
      tools: ['read_file'],
    })
  })

  it('reads alone, and is never voided for it, when the installed agent file switches MCP off', async () => {
    // The agent file setup installed before #255: it sits above the launch's environment.
    const { h, run } = await specRun({ vibeAgentFile: AGENT_BEFORE_255 })

    const result = await fanOut(h, run, { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [finding()] })) })

    const [launch] = h.process.commands
    expect(launch?.env?.VIBE_ENABLE_MCP).toBe('false')
    expect(launch?.env?.VIBE_MCP_SERVERS).toBeUndefined()
    expect(enabledTools(launch?.args ?? [])).toEqual(['read_file'])
    // Settled before launch: no search server was even proven.
    expect(h.process.searches).toEqual([])
    expect(await promptOf(h)).toContain('You have one tool: `read_file`')
    expect(result.critics[0]).toMatchObject({ finding_count: 1, status: 'ok', tools: ['read_file'] })
    expect(result.critics[0]?.failure).toBeUndefined()
    expect(result.critics[0]?.search_failure).toBe(
      'agent-file-disables-mcp: ~/.vibe/agents/critic-mistral-large.toml sets enable_mcp = false, which the launch cannot override; re-run agent-kit setup to install the current critic agents',
    )
  })

  it.each<[string, string]>([
    ['leaves a search tool out of enabled_tools', 'enabled_tools = ["read_file", "packet_grep", "packet_list"]\n[tools.packet_grep]\npermission = "always"\n'],
    [
      'does not approve a search tool',
      'enabled_tools = ["read_file", "packet_grep", "packet_list", "packet_symbol"]\n\n[tools.packet_grep]\npermission = "always"\n\n[tools.packet_list]\npermission = "ask"\n[session_logging]\nenabled = false\n',
    ],
    ['has no tables and no tools', 'display_name = "Critic"\n'],
  ])('reads alone when the installed agent file %s', async (_name, agentFile) => {
    const { h, run } = await specRun({ vibeAgentFile: agentFile })

    const result = await fanOut(h, run, { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [finding()] })) })

    expect(result.critics[0]).toMatchObject({ status: 'ok', tools: ['read_file'] })
    expect(result.critics[0]?.search_failure).toMatch(/^agent-file-lacks-search-tools: ~\/\.vibe\/agents\/critic-mistral-large\.toml does not enable and approve /)
  })

  // F1 of review 20261102T124643Z-ticket-review, as the critic wrote it.
  it('reads alone when the installed agent file declares its own MCP server', async () => {
    const { h, run } = await specRun({
      vibeAgentFile:
        'enabled_tools = ["read_file", "packet_grep", "packet_list", "packet_symbol"]\n\n[tools.packet_grep]\npermission = "always"\n\n[tools.packet_list]\npermission = "always"\n\n[tools.packet_symbol]\npermission = "always"\n\nmcp_servers = [{ name = "extra", transport = "streamable-http", url = "https://example.invalid/mcp" }]\n',
    })

    const result = await fanOut(h, run, { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [finding()] })) })

    expect(result.critics[0]).toMatchObject({ tools: ['read_file'] })
    expect(result.critics[0]?.search_failure).toContain('extra')
  })

  // F4 of review 20261102T124643Z-ticket-review, as the critic wrote it.
  it('searches when the installed agent file single-quotes its enabled_tools list', async () => {
    const { h, run } = await specRun({
      vibeAgentFile:
        "enabled_tools = ['read_file', 'packet_grep', 'packet_list', 'packet_symbol']\n\n[tools.packet_grep]\npermission = \"always\"\n\n[tools.packet_list]\npermission = \"always\"\n\n[tools.packet_symbol]\npermission = \"always\"\n",
    })

    const result = await fanOut(h, run, searching())

    expect(result.critics[0]).toMatchObject({ tools: ['read_file', 'run_typescript', 'mcp_packet.grep', 'mcp_packet.list'] })
  })

  // F6 of review 20261102T124643Z-ticket-review, as the critic wrote it.
  it('searches when the installed agent file single-quotes its tools and permissions', async () => {
    const { h, run } = await specRun({
      vibeAgentFile:
        "enabled_tools = ['read_file', 'packet_grep', 'packet_list', 'packet_symbol']\n\n[tools.packet_grep]\npermission = 'always'\n\n[tools.packet_list]\npermission = 'always'\n\n[tools.packet_symbol]\npermission = 'always'\n",
    })

    const result = await fanOut(h, run, searching())

    expect(result.critics[0]?.search_failure).toBeUndefined()
    expect(result.critics[0]?.tools).toContain('mcp_packet.grep')
  })

  it('reads alone when the agent file is gone by the time the critic launches', async () => {
    const { h, run } = await specRun()
    await h.removeFile(join(h.home, '.vibe', 'agents', 'critic-mistral-large.toml'))

    const result = await fanOut(h, run, { exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [finding()] })) })

    expect(result.critics[0]?.search_failure).toBe('agent-file-missing: ~/.vibe/agents/critic-mistral-large.toml is not installed; re-run agent-kit setup')
  })

  it('reads alone on a runner that has no search server', async () => {
    const { h, run } = await specRun({ search: false })

    const result = await fanOut(h, run, { exitCode: 0, exportJson: searchExport({ enable_mcp: false }), stdout: vibeStdout(fenced({ findings: [] , no_findings_reason: 'Read both.' })) })

    expect(h.process.searches).toEqual([])
    expect(result.critics[0]).toMatchObject({ status: 'ok', tools: ['read_file'] })
    expect(result.critics[0]?.search_failure).toBeUndefined()
  })

  it('names in the report a server that died during the run, and keeps the review', async () => {
    const { h, run } = await specRun()
    const journal = JSON.parse(vibeStdout(fenced({ findings: [finding()] }))) as unknown[]
    const effect = (status: string, error?: unknown) => ({
      detail: { toolName: 'mcp_packet.grep' },
      state: { status, ...(error === undefined ? {} : { error }) },
      type: 'effect',
    })
    journal.splice(1, 0, effect('completed'), effect('failed', { code: 'mcp_transport_failed', message: 'The provided tool call failed' }), effect('failed', { code: 'mcp_transport_failed', message: 'The provided tool call failed' }))

    const result = await fanOut(h, run, searching({ stdout: JSON.stringify(journal) }))

    expect(result.critics[0]).toMatchObject({
      search_failure: '2 of 3 search calls failed, the first with mcp_transport_failed: The provided tool call failed',
      status: 'ok',
    })
    const recorded = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', reason: 'The quotes show it.', verdict: 'accepted' }] })
    const report = await readFile(recorded.report_path, 'utf8')
    expect(report).toContain('- Search failure: 2 of 3 search calls failed, the first with mcp_transport_failed: The provided tool call failed')
    expect(report).toContain('- Tools: read_file, run_typescript, mcp_packet.grep, mcp_packet.list')
  })
})

describe('the effective config of a searching critic is checked', () => {
  it.each<[string, Record<string, unknown>, string]>([
    ['MCP off', { enable_mcp: false }, 'enable_mcp is not true'],
    ['a home server left on', { mcp_servers: [{ cwd: 'x', name: 'hindsight' }] }, 'MCP servers other than the search server are on: hindsight'],
    ['no search server', { mcp_servers: [] }, 'the packet MCP server is not the runner'],
    ['bash enabled', { enabled_tools: ['read_file', 'packet_grep', 'bash'] }, 'enabled_tools'],
  ])('discards the review of a critic whose effective config has %s', async (_name, overrides, detail) => {
    const { h, run } = await specRun()

    const result = await fanOut(h, run, searching({ exportJson: searchExport(overrides) }))

    expect(result.critics[0]).toMatchObject({ failure: 'unsafe-config', finding_count: 0, status: 'failed' })
    expect(result.critics[0]?.detail).toContain(detail)
  })

  it('discards the review of a critic whose search server ran somewhere else', async () => {
    const { h, run } = await specRun()
    const elsewhere = (launch: Parameters<ReturnType<typeof searchExport>>[0]) => {
      const exported = searchExport()(launch)
      const config = exported.config as { mcp_servers: Record<string, unknown>[] }
      return { ...exported, config: { ...config, mcp_servers: config.mcp_servers.map((server) => ({ ...server, cwd: h.repo })) } }
    }

    const result = await fanOut(h, run, searching({ exportJson: elsewhere }))

    expect(result.critics[0]).toMatchObject({ failure: 'unsafe-config', status: 'failed' })
  })

  it('still voids a searching critic that changes its packet', async () => {
    const { h, run } = await specRun()

    const result = await fanOut(h, run, searching({ effect: ({ cwd }) => h.writeText(join(cwd, 'artifact', 'spec.md'), 'rewritten\n') }))

    expect(result.critics[0]).toMatchObject({ finding_count: 0, status: 'violated-readonly' })
  })
})

describe('an agy critic reads alone until agy’s read-only proof exists (F2 of review 20261102T124643Z-ticket-review)', () => {
  const AGY_ROUTING = {
    clients: { 'agy-gemini-flash-high': { binary: 'agy', family: 'google', kind: 'agy', model: 'gemini-3.8-flash-high' } },
    stages: {
      spec: {
        base: [{ client: 'agy-gemini-flash-high', lenses: ['fidelity'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 }],
        distinct_families: true,
      },
    },
    version: 1,
  }

  async function agyRun(setupResults?: unknown): Promise<{ h: Harness; result: FanOutResult; prompt: string; settings: { permissions: { deny: string[] } } }> {
    const h = await harness({ agyScripts: [agyFindings({ findings: [finding()] }, ['view_file'])], routing: AGY_ROUTING, search: true })
    open.push(h)
    if (setupResults !== undefined) {
      await h.writeText(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'machine-setup.json'), JSON.stringify(setupResults))
    }
    await h.commands.allow.run({ families: ['google'], repo: h.repo })
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
    if (run.status !== 'ready') throw new Error('not ready')
    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })
    const args = h.agy.launches[0]?.args ?? []
    return {
      h,
      prompt: args[args.indexOf('-p') + 1] ?? '',
      result,
      settings: JSON.parse(args[args.indexOf('--settings') + 1] ?? '{}') as { permissions: { deny: string[] } },
    }
  }

  it('names only view_file, denies agy’s listing and search tools, and records agy-search-unproven', async () => {
    const { h, prompt, result, settings } = await agyRun()

    expect(prompt).toContain('You have one tool: `view_file`')
    expect(prompt).toContain('Read each one with view_file')
    for (const tool of ['list_dir', 'grep_search', 'find_by_name']) expect(prompt).not.toContain(`\`${tool}\``)
    expect(settings.permissions.deny).toEqual(['mcp(*)', 'read_url(*)', 'execute_url(*)', 'command(*)', 'write_file(*)', 'list_dir(*)', 'grep_search(*)', 'find_by_name(*)'])
    expect(result.critics[0]).toMatchObject({ status: 'ok', tools: ['view_file'] })
    expect(result.critics[0]?.search_failure).toBe(
      "agy-search-unproven: agy-gemini-flash-high's read-only proof, which must show its search cannot reach past the packet, waits for agent-kit#178",
    )
    // agy needs no search server of the runner's.
    expect(h.process.searches).toEqual([])
  })

  it('searches with agy’s own tools once setup records agy’s read-only proof passing', async () => {
    const pass = { checked_at: '2026-11-02T00:00:00.000Z', detail: 'proven', inputs: {}, result: 'pass' }
    const { prompt, result, settings } = await agyRun({ checks: { 'read-only:agy-gemini-flash-high': pass }, version: 1 })

    expect(prompt).toContain('`view_file` reads one file, `list_dir` lists a directory, `grep_search` searches the contents of files and `find_by_name` finds files by name')
    expect(settings.permissions.deny).toEqual(['mcp(*)', 'read_url(*)', 'execute_url(*)', 'command(*)', 'write_file(*)'])
    expect(result.critics[0]).toMatchObject({ status: 'ok', tools: ['view_file', 'list_dir', 'grep_search', 'find_by_name'] })
    expect(result.critics[0]?.search_failure).toBeUndefined()
  })

  it('reads alone while agy’s last read-only proof failed', async () => {
    const fail = { checked_at: '2026-11-02T00:00:00.000Z', detail: 'leaked', inputs: {}, result: 'fail' }
    const { result } = await agyRun({ checks: { 'read-only:agy-gemini-flash-high': fail }, version: 1 })

    expect(result.critics[0]).toMatchObject({ tools: ['view_file'] })
  })
})

describe('the refute round', () => {
  it('gives a Vibe refuter the search server on its own packet copy', async () => {
    const slot = (client: string) => ({ client, lenses: ['fidelity'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 })
    const { h, run } = await specRun({
      routing: {
        clients: {
          'vibe-glm': { agent: 'critic-glm', binary: 'vibe', family: 'zhipu', kind: 'vibe', model: 'zai-glm-5-3' },
          'vibe-mistral-large': { agent: 'critic-mistral-large', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-large-4-0' },
        },
        stages: { spec: { base: [slot('vibe-mistral-large')], deep: [slot('vibe-glm')], distinct_families: true } },
        version: 1,
      },
    }, ['mistral', 'zhipu'])
    await fanOut(h, run, searching({ stdout: vibeStdout(fenced({ findings: [finding({ severity: 'critical' })] })) }))
    const refuter = recorder([{ arguments: { pattern: 'nightly' }, tool: 'grep' }])
    h.process.queue(searching({ agent: 'critic-glm', effect: refuter.effect, stdout: vibeStdout(fenced({ findings: [], no_findings_reason: 'Nothing to object to.' })) }))

    const result = await h.commands.refute.run({ run_dir: run.run_dir, verdicts: [{ finding_id: 'F1', verdict: 'accepted' }] })

    expect(result.results[0]?.refuter).toMatchObject({ tools: ['read_file', 'run_typescript', 'mcp_packet.grep', 'mcp_packet.list'] })
    expect(refuter.answers).toEqual(['references/CONTEXT.md:3: Decided: widgets are frobnicated by the nightly job only.'])
    const servers = launchServers(h.process.commands[1])
    expect(servers.find((server) => server.name === 'packet')?.cwd).toBe(join(run.run_dir, 'refute', 'F1', 'packet'))
    expect(await promptOf(h, 1)).toContain('tools.mcp_packet')
  })
})
