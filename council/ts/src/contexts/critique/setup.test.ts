import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  agyFindings,
  agyResult,
  agyStream,
  fenced,
  harness,
  isolatedExport,
  searchExport,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { SetupCheckOutput, SetupItem, SetupListResult, SetupQuotaOutput } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

type HarnessOptions = NonNullable<Parameters<typeof harness>[0]>

async function machine(options: HarnessOptions = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  return h
}

async function listed(h: Harness, input: Record<string, unknown> = {}): Promise<SetupListResult> {
  const result = await h.commands.setup.run({ action: 'list', ...input })
  if (result.status !== 'listed') throw new Error(`not listed: ${JSON.stringify(result)}`)
  return result
}

async function list(h: Harness): Promise<readonly SetupItem[]> {
  return (await listed(h)).items
}

async function check(
  h: Harness,
  id: string,
  extra: Record<string, unknown> = {},
): Promise<SetupCheckOutput> {
  const result = await h.commands.setup.run({ action: 'check', check: id, ...extra })
  if (result.status === 'listed' || result.status === 'quota-set' || result.status === 'kill-switch-set')
    throw new Error(`not a check: ${JSON.stringify(result)}`)
  return result
}

async function setQuota(h: Harness, input: Record<string, unknown>): Promise<SetupQuotaOutput> {
  const result = await h.commands.setup.run({ action: 'quota', ...input })
  if (result.status !== 'quota-set') throw new Error(`quota not set: ${JSON.stringify(result)}`)
  return result
}

function item(items: readonly SetupItem[], id: string): SetupItem {
  const found = items.find((entry) => entry.id === id)
  if (found === undefined) throw new Error(`no item ${id}`)
  return found
}

/** A Vibe critic that answers setup's trivial prompt with an empty findings list. */
function vibeAnswers(agent: string): VibeScript {
  return {
    agent,
    exitCode: 0,
    exportJson: { ...isolatedExport({ active_model: agent }), cost_usd: 0.0042 },
    stdout: vibeStdout(
      fenced({ findings: [], no_findings_reason: 'setup check: nothing to review' }),
    ),
  }
}

const AGY_ANSWERS = agyFindings({
  findings: [],
  no_findings_reason: 'setup check: nothing to review',
})

function dataDir(h: Harness): string {
  return join(h.home, '.local', 'share', 'agent-kit', 'critique')
}

function overridePath(h: Harness): string {
  return join(h.home, '.config', 'agent-kit', 'critique-routing.json')
}

async function ledgerLines(h: Harness): Promise<readonly unknown[]> {
  const text = await h.readText(join(dataDir(h), 'ledger.jsonl'))
  return text
    .split('\n')
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line) as unknown)
}

describe("setup's machine checklist", () => {
  it('lists every check in the order setup walks them, Vibe first, then Antigravity, the read-only proofs, then the hooks', async () => {
    const h = await machine()

    expect((await list(h)).map((entry) => entry.id)).toEqual([
      'vibe-signed-in',
      'critic:vibe-mistral-large',
      'critic:vibe-glm',
      'critic:vibe-deepseek-flash',
      'critic:vibe-mistral-small',
      'agy-signed-in',
      'critic:agy-gemini-flash-high',
      'agy-quota',
      'read-only:vibe-mistral-large',
      'read-only:agy-gemini-flash-high',
      'read-only:vibe-glm',
      'read-only:vibe-deepseek-flash',
      'read-only:vibe-mistral-small',
      'read-only:claude',
      'read-only:codex',
      'read-only:hermes',
      'read-only:opencode',
      'hooks-installed',
      'kill-switch',
    ])
  })

  it('tells the owner to delete a routing override that still names the removed per-flow Vibe cap', async () => {
    const h = await machine({ routingOverride: { budget: { vibe_flow_cap_usd: 8 } } })

    const result = await listed(h)

    expect(result.notes).toEqual([
      `your routing override still names budget.vibe_flow_cap_usd in ${join(h.home, '.config', 'agent-kit', 'critique-routing.json')}: it is ignored, since Vibe is counted against its plan limit; delete it`,
    ])
  })

  it('says nothing about a routing override that names no removed setting', async () => {
    const h = await machine({ routingOverride: { antigravity_quota: { daily_cap: 8 } } })

    expect((await listed(h)).notes).toBeUndefined()
  })

  it('says what each item costs: sign-ins and the quota are free, a critic spends on its own CLI', async () => {
    const h = await machine()

    const costs = Object.fromEntries((await list(h)).map((entry) => [entry.id, entry.cost]))

    expect(costs).toMatchObject({
      'agy-quota': 'free',
      'agy-signed-in': 'free',
      'critic:agy-gemini-flash-high': 'agy',
      'critic:vibe-glm': 'vibe',
      'vibe-signed-in': 'free',
    })
  })

  it('gives every item one sentence on why it matters and a fix to offer', async () => {
    const h = await machine()

    const items = await list(h)

    expect(
      items.filter(
        (entry) =>
          entry.why.split('. ').length !== 1 || !entry.why.endsWith('.') || entry.fix.length === 0,
      ),
    ).toEqual([])
  })

  it('marks every check never checked on a fresh machine', async () => {
    const h = await machine()

    const checks = (await list(h)).filter((entry) => entry.kind !== 'read-only')

    expect(checks.map((entry) => [entry.status, entry.due_because])).toEqual(
      Array.from({ length: 10 }, () => ['due', 'never-checked']),
    )
  })

  it('launches no critic while listing', async () => {
    const h = await machine()

    await list(h)

    expect([h.process.commands, h.agy.launches]).toEqual([[], []])
  })

  it('reads each CLI version as an input its checks depend on', async () => {
    const h = await machine({ agyVersion: '1.4.12', vibeVersion: 'vibe 3.0.4' })

    const items = await list(h)

    expect([item(items, 'vibe-signed-in').inputs, item(items, 'agy-signed-in').inputs]).toEqual([
      { cli_version: 'vibe 3.0.4' },
      { cli_version: '1.4.12' },
    ])
  })

  it('reads no version for a CLI that is not installed', async () => {
    const h = await machine({ vibeOnPath: false })

    expect(item(await list(h), 'vibe-signed-in').inputs).toEqual({ cli_version: null })
  })

  it('leaves out the Antigravity items when routing has no agy client', async () => {
    const h = await machine({
      routing: {
        clients: {
          'vibe-glm': {
            agent: 'critic-glm',
            binary: 'vibe',
            family: 'zhipu',
            kind: 'vibe',
            model: 'glm-5',
          },
        },
        stages: {},
        version: 1,
      },
    })

    expect((await list(h)).map((entry) => entry.id)).toEqual([
      'vibe-signed-in',
      'critic:vibe-glm',
      'read-only:vibe-glm',
      'read-only:claude',
      'read-only:codex',
      'read-only:hermes',
      'read-only:opencode',
      'hooks-installed',
      'kill-switch',
    ])
  })

  it('leaves out the Vibe items when routing has no Vibe client', async () => {
    const h = await machine({
      routing: {
        clients: {
          'agy-gemini-flash-high': {
            binary: 'agy',
            family: 'google',
            kind: 'agy',
            model: 'gemini-3.8-flash-high',
          },
        },
        stages: {},
        version: 1,
      },
    })

    expect((await list(h)).map((entry) => entry.id)).toEqual([
      'agy-signed-in',
      'critic:agy-gemini-flash-high',
      'agy-quota',
      'read-only:agy-gemini-flash-high',
      'read-only:claude',
      'read-only:codex',
      'read-only:hermes',
      'read-only:opencode',
      'hooks-installed',
      'kill-switch',
    ])
  })
})

/** A Vibe journal whose final message is `text`, after one tool effect per call: [tool, status, error code]. */
function journal(calls: readonly (readonly [string, string, string?])[], text = fenced({ findings: [], no_findings_reason: 'setup check: nothing to review' })): string {
  const entries = JSON.parse(vibeStdout(text)) as unknown[]
  const effects = calls.map(([tool, status, code]) => ({
    detail: { toolName: tool },
    state: { status, ...(code === undefined ? {} : { error: { code, message: 'Tool execution denied by approval policy' } }) },
    type: 'effect',
  }))
  return JSON.stringify([entries[0], ...effects, ...entries.slice(1)])
}

/** What Vibe 3.0.4 records for the proof's one run_typescript call: three refusals and a search. */
const REFUSED: readonly (readonly [string, string, string?])[] = [
  ['file_system.bash', 'failed', 'tool_denied'],
  ['file_system.write_file', 'failed', 'tool_denied'],
  ['process.start', 'failed', 'tool_denied'],
  ['mcp_packet.grep', 'completed'],
]

const gateway = (calls = REFUSED, overrides: Partial<VibeScript> = {}): VibeScript => ({
  agent: 'critic-glm',
  exitCode: 0,
  exportJson: searchExport(),
  stdout: journal(calls),
  ...overrides,
})

describe('the read-only proof for a client with no critic', () => {
  it('does not title an unproven client as proven', async () => {
    const h = await machine({ search: true })

    const items = await list(h)
    const titles = ['read-only:claude', 'read-only:codex', 'read-only:hermes', 'read-only:opencode'].map((id) => item(items, id).title)

    expect(titles.filter((title) => title.includes('is proven'))).toEqual([])
  })

  it('records Claude Code, Codex, Hermes and OpenCode as unproven, with the reason, and runs nothing', async () => {
    const h = await machine({ search: true })

    const result = await check(h, 'read-only:codex')

    expect([result.status, result.item.last?.result, h.process.commands, h.process.searches, h.agy.launches]).toEqual([
      'checked',
      'unproven',
      [],
      [],
      [],
    ])
    expect(result.item.last?.detail).toContain('sandbox')
    expect(result.item.last?.detail).toContain('live prompt')
    expect(result.item.kind).toBe('read-only')
  })

  it('never reports one of them as passing', async () => {
    const h = await machine({ search: true })

    const outcomes = [] as string[]
    for (const id of ['read-only:claude', 'read-only:codex', 'read-only:hermes', 'read-only:opencode']) {
      outcomes.push((await check(h, id)).item.last?.result ?? 'none')
    }

    expect(outcomes).toEqual(['unproven', 'unproven', 'unproven', 'unproven'])
  })

  it('is current once recorded, and due again only when setup is re-run after its inputs change', async () => {
    const h = await machine({ search: true })
    await check(h, 'read-only:hermes')

    const again = await check(h, 'read-only:hermes')

    expect([again.status, item(await list(h), 'read-only:hermes').status]).toEqual(['current', 'current'])
  })

  it('gives each client its own reason, and never calls one a read-only mode it has not shown', async () => {
    const h = await machine({ search: true })

    const items = await list(h)
    const reasons = ['read-only:claude', 'read-only:codex', 'read-only:hermes', 'read-only:opencode'].map(
      (id) => item(items, id).why,
    )

    expect(new Set(reasons).size).toBe(4)
    expect(reasons.every((why) => why.endsWith('.'))).toBe(true)
  })
})

describe('the read-only proof', () => {
  it('waits for #178 for agy, and is one Vibe prompt for each Vibe client', async () => {
    const h = await machine({ search: true })

    const proofs = (await list(h)).filter((entry) => entry.kind === 'read-only')

    expect(proofs.map((entry) => [entry.id, entry.status, entry.cost, entry.detail ?? entry.due_because])).toEqual([
      ['read-only:vibe-mistral-large', 'due', 'vibe', 'never-checked'],
      ['read-only:agy-gemini-flash-high', 'not-available', 'agy', expect.stringContaining('not available until agent-kit#178 lands') as unknown],
      ['read-only:vibe-glm', 'due', 'vibe', 'never-checked'],
      ['read-only:vibe-deepseek-flash', 'due', 'vibe', 'never-checked'],
      ['read-only:vibe-mistral-small', 'due', 'vibe', 'never-checked'],
      ['read-only:claude', 'due', 'free', 'never-checked'],
      ['read-only:codex', 'due', 'free', 'never-checked'],
      ['read-only:hermes', 'due', 'free', 'never-checked'],
      ['read-only:opencode', 'due', 'free', 'never-checked'],
    ])
    expect(item(proofs, 'read-only:vibe-glm').why).toContain('refuses shell commands, file writes and process starts')
  })

  it('runs and records nothing for agy when asked to check', async () => {
    const h = await machine({ search: true })

    const result = await check(h, 'read-only:agy-gemini-flash-high', { confirmed: true })

    expect([result.status, h.process.searches, h.agy.launches, await h.readText(result.results_path).catch(() => 'no file')]).toEqual([
      'not-available',
      [],
      [],
      'no file',
    ])
  })

  it('proves search reach for free, then asks before the live prompt', async () => {
    const h = await machine({ search: true })

    const result = await check(h, 'read-only:vibe-glm')

    expect(result.status).toBe('needs-confirmation')
    expect(h.process.commands).toEqual([])
    // The reach proof's packet has two symlinks out of it and a file outside it, and the server is asked through each way out.
    const calls = (h.process.searches[0]?.stdin ?? '').split('\n').filter((line) => line.includes('tools/call'))
    expect(calls).toHaveLength(9)
  })

  it('shows run_typescript refusing bash, write_file and process starts while the search completes', async () => {
    const h = await machine({ scripts: [gateway()], search: true })

    const result = await check(h, 'read-only:vibe-glm', { confirmed: true })

    expect(result.item.last).toMatchObject({ result: 'pass' })
    expect(result.item.last?.detail).toBe(
      'search cannot reach past the packet, and through run_typescript file_system.bash, file_system.write_file, process.start came back tool_denied while mcp_packet.grep completed (charged $0.0123); ' +
        "agent-kit#178's live smoke check adds the rest of the read-only proof",
    )
    const [launch] = h.process.commands
    expect(launch?.env?.VIBE_ENABLE_MCP).toBe('true')
    const args = launch?.args ?? []
    const prompt = await h.readText(args[args.indexOf('--prompt-file') + 1] ?? '')
    expect(prompt).toContain("tools.file_system.bash({ command: 'echo probe > probe.txt' })")
    expect(prompt).toContain("tools.mcp_packet.grep({ pattern: 'probe' })")
    expect(h.process.stops).toHaveLength(1)
    expect((await list(h)).find((entry) => entry.id === 'read-only:vibe-glm')?.status).toBe('current')
  })

  it.each<[string, readonly (readonly [string, string, string?])[], string]>([
    ['bash ran', [['file_system.bash', 'completed'], ...REFUSED.slice(1)], 'file_system.bash came back completed, not tool_denied'],
    ['a write failed for another reason', [REFUSED[0] ?? ['', ''], ['file_system.write_file', 'failed', 'permission_denied'], ...REFUSED.slice(2)], 'file_system.write_file came back failed (permission_denied), not tool_denied'],
    ['no process start was tried', REFUSED.filter(([tool]) => tool !== 'process.start'), 'process.start was not tried, so its refusal is not shown'],
    ['the search failed', [...REFUSED.slice(0, 3), ['mcp_packet.grep', 'failed', 'mcp_transport_failed']], 'mcp_packet.grep did not complete through run_typescript'],
  ])('fails when %s', async (_name, calls, problem) => {
    const h = await machine({ scripts: [gateway(calls)], search: true })

    const result = await check(h, 'read-only:vibe-glm', { confirmed: true })

    expect(result.item.last).toMatchObject({ result: 'fail' })
    expect(result.item.last?.detail).toContain(problem)
  })

  it('fails when the critic changed its packet', async () => {
    const h = await machine({
      scripts: [gateway(REFUSED, { effect: ({ cwd }) => h.writeText(join(cwd, 'probe.txt'), 'probe\n') })],
      search: true,
    })

    const result = await check(h, 'read-only:vibe-glm', { confirmed: true })

    expect(result.item.last?.detail).toContain('the critic changed its packet')
  })

  it('clears what an earlier proof left in its packet before it runs again', async () => {
    const h = await machine({ scripts: [gateway(), gateway()], search: true })
    await check(h, 'read-only:vibe-glm', { confirmed: true })

    const again = await check(h, 'read-only:vibe-glm', { confirmed: true, force: true })

    expect(again.item.last).toMatchObject({ result: 'pass' })
  })

  it('fails with the critic’s own failure when its run gives no review', async () => {
    const h = await machine({ scripts: [gateway(REFUSED, { exitCode: 2, stderr: 'Error: boom' })], search: true })

    const result = await check(h, 'read-only:vibe-glm', { confirmed: true })

    expect(result.item.last).toMatchObject({ result: 'fail' })
    expect(result.item.last?.detail).toContain('infrastructure-failure')
  })

  it('fails, saying why, when the search server does not start, without asking or spending', async () => {
    const h = await machine({ search: true })
    h.process.searchBehaviour = 'broken'

    const result = await check(h, 'read-only:vibe-glm')

    expect(result.item.last).toMatchObject({
      detail: 'search is not proven to stay inside the packet: the search server exited 1: Error: Cannot find module critique.mjs',
      result: 'fail',
    })
    expect(h.process.commands).toEqual([])
  })

  it('fails when the server proves its reach but does not start on the live proof’s packet', async () => {
    const h = await machine({ search: true })
    h.process.searchBehaviour = 'broken-on-copies'

    const result = await check(h, 'read-only:vibe-glm', { confirmed: true })

    expect(result.item.last?.detail).toBe('search is not proven to stay inside the packet: the search server exited 1: Error: Cannot find module critique.mjs')
  })

  it('fails before asking or spending when the installed agent file switches MCP off', async () => {
    const h = await machine({ search: true, vibeAgentFile: 'enabled_tools = ["read_file"]\nenable_mcp = false\n' })

    const result = await check(h, 'read-only:vibe-glm', { confirmed: true })

    expect(result.item.last).toMatchObject({ result: 'fail' })
    expect(result.item.last?.detail).toMatch(/^agent-file-disables-mcp: ~\/\.vibe\/agents\/critic-glm\.toml sets enable_mcp = false/)
    expect(h.process.commands).toEqual([])
  })

  it('fails on a runner without a search server', async () => {
    const h = await machine()

    const result = await check(h, 'read-only:vibe-glm')

    expect(result.item.last).toMatchObject({ detail: expect.stringContaining('this runner has no search server') as unknown, result: 'fail' })
  })

  it('is due again once the runner, which is the search server, changes', async () => {
    const h = await machine({ assets: { 'critique.mjs': (shipped) => shipped }, scripts: [gateway()], search: true })
    await check(h, 'read-only:vibe-glm', { confirmed: true })
    await h.writeText(join(h.assetsRoot, 'critique.mjs'), '// a new runner\n')

    const proof = (await list(h)).find((entry) => entry.id === 'read-only:vibe-glm')

    expect([proof?.status, proof?.due_because, proof?.changed]).toEqual(['due', 'inputs-changed', ['files_hash']])
  })

  it('records a runner without its bundle as having none', async () => {
    const h = await machine({ assets: { 'critique.mjs': (shipped) => shipped }, search: true })
    await h.removeFile(join(h.assetsRoot, 'critique.mjs'))

    const result = await check(h, 'read-only:vibe-glm')

    expect(result.item.inputs).toMatchObject({ files_hash: 'none' })
  })
})

describe('a sign-in check', () => {
  it('passes on a signed-in machine and records its time, result and CLI version beside the ledger', async () => {
    const h = await machine()

    const result = await check(h, 'vibe-signed-in')

    expect(result.status).toBe('checked')
    expect(JSON.parse(await h.readText(join(dataDir(h), 'machine-setup.json')))).toEqual({
      checks: {
        'vibe-signed-in': {
          checked_at: '2026-10-30T12:00:00.000Z',
          detail: 'vibe is installed and signed in',
          inputs: { cli_version: 'vibe 3.0.4' },
          result: 'pass',
        },
      },
      version: 1,
    })
  })

  it("fails with setup's own sign-in wording when the personal Vibe home is signed out", async () => {
    const h = await machine({ vibeSignIn: 'none' })

    const result = await check(h, 'vibe-signed-in')

    expect([result.item.last?.result, result.item.last?.detail]).toEqual([
      'fail',
      'vibe: not signed in; run vibe and sign in with your Mistral account, then re-run setup',
    ])
  })

  it('fails when vibe is not on PATH', async () => {
    const h = await machine({ vibeOnPath: false })

    expect((await check(h, 'vibe-signed-in')).item.last?.detail).toBe('vibe: vibe is not on PATH')
  })

  it("fails with setup's own sign-in wording when Antigravity is signed out", async () => {
    const h = await machine({ agyModels: 'signed-out' })

    expect((await check(h, 'agy-signed-in')).item.last?.detail).toBe(
      'antigravity: needs sign-in: run agy and sign in with your Google account',
    )
  })

  it('passes Antigravity on a signed-in Mac', async () => {
    const h = await machine()

    expect((await check(h, 'agy-signed-in')).item.last?.result).toBe('pass')
  })

  it('is current on the next list once it passed', async () => {
    const h = await machine()
    await check(h, 'vibe-signed-in')

    expect(item(await list(h), 'vibe-signed-in').status).toBe('current')
  })

  it('does not run again while it is current', async () => {
    const h = await machine()
    await check(h, 'agy-signed-in')
    h.agy.commands.splice(0)

    const result = await check(h, 'agy-signed-in')

    expect([result.status, h.agy.commands.map((command) => command.command)]).toEqual([
      'current',
      ['agy'],
    ])
  })

  it('runs again while current when forced', async () => {
    const h = await machine()
    await check(h, 'vibe-signed-in')

    expect((await check(h, 'vibe-signed-in', { force: true })).status).toBe('checked')
  })

  it('is due again after the CLI version changed', async () => {
    const h = await machine()
    await check(h, 'vibe-signed-in')
    h.process.version = 'vibe 3.1.0'

    const entry = item(await list(h), 'vibe-signed-in')

    expect([entry.status, entry.due_because, entry.changed]).toEqual([
      'due',
      'inputs-changed',
      ['cli_version'],
    ])
  })

  it('is due again after it failed, though nothing changed', async () => {
    const h = await machine({ vibeSignIn: 'none' })
    await check(h, 'vibe-signed-in')

    const entry = item(await list(h), 'vibe-signed-in')

    expect([entry.status, entry.due_because]).toEqual(['due', 'failed'])
  })
})

describe('a critic check', () => {
  it('asks for confirmation before it spends Vibe money, and launches nothing', async () => {
    const h = await machine()

    const result = await check(h, 'critic:vibe-glm')

    expect([result.status, h.process.commands]).toEqual(['needs-confirmation', []])
  })

  it('asks for confirmation before it spends Antigravity quota, and launches nothing', async () => {
    const h = await machine()

    const result = await check(h, 'critic:agy-gemini-flash-high')

    expect([result.status, h.agy.launches]).toEqual(['needs-confirmation', []])
  })

  it('records nothing while unconfirmed', async () => {
    const h = await machine()

    await check(h, 'critic:vibe-glm')

    expect(item(await list(h), 'critic:vibe-glm').due_because).toBe('never-checked')
  })

  it('once confirmed, puts the trivial prompt to the critic agent and passes when it answers', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })

    const result = await check(h, 'critic:vibe-glm', { confirmed: true })

    expect([result.status, result.item.last?.result, result.item.last?.detail]).toEqual([
      'checked',
      'pass',
      'vibe-glm answered the trivial prompt (charged $0.0042)',
    ])
  })

  it('launches the critic read-only in a one-note packet under the data directory, never in the repository', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })

    await check(h, 'critic:vibe-glm', { confirmed: true })

    const [launch] = h.process.commands
    const args = launch?.args ?? []
    expect({
      agent: args[args.indexOf('--agent') + 1],
      cwd: launch?.cwd,
      note: await h.readText(join(dataDir(h), 'setup-checks', 'vibe-glm', 'packet', 'note.md')),
      tools: args[args.indexOf('--enabled-tools') + 1],
    }).toEqual({
      agent: 'critic-glm',
      cwd: join(dataDir(h), 'setup-checks', 'vibe-glm', 'packet'),
      note: "This note is agent-kit's setup check. It has nothing in it to review.\n",
      tools: 'read_file',
    })
  })

  it('caps the trivial prompt at five cents', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })

    await check(h, 'critic:vibe-glm', { confirmed: true })

    const args = h.process.commands[0]?.args ?? []
    expect(args[args.indexOf('--max-price') + 1]).toBe('0.05')
  })

  it('fails with the failure and the charge when the critic does not answer', async () => {
    const h = await machine({
      scripts: [{ agent: 'critic-glm', exitCode: 2, stderr: 'agent not found', stdout: '' }],
    })

    const result = await check(h, 'critic:vibe-glm', { confirmed: true })

    expect([result.item.last?.result, result.item.last?.detail]).toEqual([
      'fail',
      'infrastructure-failure: vibe exited 2: agent not found (charged $0.0500)',
    ])
  })

  it('fails without asking or launching when the critic agent is not installed', async () => {
    const h = await machine({ vibeAgents: ['critic-mistral-large'] })

    const result = await check(h, 'critic:vibe-glm')

    expect([result.status, result.item.last?.detail, h.process.commands]).toEqual([
      'checked',
      'vibe: the critic agent critic-glm is not installed in ~/.vibe/agents; re-run setup',
      [],
    ])
  })

  it('records the CLI version, the agent file hash and the routing hash it depended on', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')], vibeAgentFile: '' })

    const result = await check(h, 'critic:vibe-glm', { confirmed: true })

    // Every agent file is installed empty here; this is the sha256 of no bytes.
    expect(result.item.last?.inputs).toEqual({
      agent_hash: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
      cli_version: 'vibe 3.0.4',
      routing_hash: expect.stringMatching(/^[0-9a-f]{64}$/) as unknown,
    })
  })

  it('is due again after its agent file changed', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })
    await check(h, 'critic:vibe-glm', { confirmed: true })
    await h.writeText(
      join(h.home, '.vibe', 'agents', 'critic-glm.toml'),
      'active_model = "critic-glm"\n',
    )

    expect(item(await list(h), 'critic:vibe-glm').changed).toEqual(['agent_hash'])
  })

  it('is due again after the routing override changed its client', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })
    await check(h, 'critic:vibe-glm', { confirmed: true })
    await h.writeText(
      overridePath(h),
      JSON.stringify({ clients: { 'vibe-glm': { model: 'zai-glm-5-3-flash' } } }),
    )

    expect(item(await list(h), 'critic:vibe-glm').changed).toEqual(['routing_hash'])
  })

  it('stays current when routing changes another client', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })
    await check(h, 'critic:vibe-glm', { confirmed: true })
    await h.writeText(
      overridePath(h),
      JSON.stringify({ clients: { 'vibe-mistral-small': { model: 'mistral-small-4' } } }),
    )

    expect(item(await list(h), 'critic:vibe-glm').status).toBe('current')
  })

  it("never reads an earlier check's export as this one's", async () => {
    const h = await machine({
      scripts: [
        vibeAnswers('critic-glm'),
        { agent: 'critic-glm', exitCode: 0, stdout: vibeStdout('pong') },
      ],
    })
    await check(h, 'critic:vibe-glm', { confirmed: true })

    const result = await check(h, 'critic:vibe-glm', { confirmed: true, force: true })

    expect(result.item.last?.detail).toBe(
      'missing-export: vibe wrote no readable export.json (charged $0.0500)',
    )
  })

  it('logs the launch to the ledger outside any repository or flow, so its Vibe spend stays on record', async () => {
    const h = await machine({ scripts: [vibeAnswers('critic-glm')] })

    await check(h, 'critic:vibe-glm', { confirmed: true })

    expect(await ledgerLines(h)).toEqual([
      {
        charged_usd: 0.0042,
        client: 'vibe-glm',
        critic_id: 'setup-vibe-glm',
        family: 'zhipu',
        flow: 'machine-setup',
        kind: 'vibe',
        model: 'zai-glm-5-3',
        recorded_at: '2026-10-30T12:00:00.000Z',
        repository: 'machine-setup',
        status: 'ok',
        type: 'invocation',
      },
    ])
  })

  it('passes the Gemini critic when agy returns the empty findings list as structured output', async () => {
    const h = await machine({ agyScripts: [AGY_ANSWERS] })

    const result = await check(h, 'critic:agy-gemini-flash-high', { confirmed: true })

    expect([result.item.last?.result, result.item.last?.detail]).toEqual([
      'pass',
      'agy-gemini-flash-high answered the trivial prompt',
    ])
  })

  it('fails the Gemini critic when agy returns prose instead of structured output', async () => {
    const h = await machine({ agyScripts: [{ stdout: agyStream(agyResult(undefined)) }] })

    const result = await check(h, 'critic:agy-gemini-flash-high', { confirmed: true })

    expect(result.item.last?.detail).toBe(
      'missing-structured-output: the reply did not match the finding schema, so agy returned no structured_output',
    )
  })

  it("counts the Gemini check against today's Antigravity quota", async () => {
    const h = await machine({ agyScripts: [AGY_ANSWERS] })
    await check(h, 'critic:agy-gemini-flash-high', { confirmed: true })

    expect((await check(h, 'agy-quota')).item.last?.detail).toContain('1 used today')
  })
})

describe('the Antigravity quota', () => {
  it('fails while no plan limit is stored, because Antigravity cannot report its quota', async () => {
    const h = await machine()

    const result = await check(h, 'agy-quota')

    expect([result.item.last?.result, result.item.last?.detail]).toEqual([
      'fail',
      "Antigravity cannot report its quota, so routing's default applies (daily cap 20, reserve threshold 10, 0 used today); give your plan's daily limit",
    ])
  })

  it("stores the plan's limit and the reserve in the per-machine routing override", async () => {
    const h = await machine()

    await setQuota(h, { daily_cap: 50, reserve_threshold: 25 })

    expect(JSON.parse(await h.readText(overridePath(h)))).toEqual({
      antigravity_quota: { daily_cap: 50, reserve_threshold: 25 },
    })
  })

  it("keeps the override's other keys", async () => {
    const h = await machine({
      routingOverride: {
        antigravity_quota: { reserved_stages: ['spec'] },
        clients: { 'vibe-glm': { model: 'zai-glm-5-3' } },
      },
    })

    await setQuota(h, { daily_cap: 50, reserve_threshold: 25 })

    expect(JSON.parse(await h.readText(overridePath(h)))).toEqual({
      antigravity_quota: { daily_cap: 50, reserve_threshold: 25, reserved_stages: ['spec'] },
      clients: { 'vibe-glm': { model: 'zai-glm-5-3' } },
    })
  })

  it('passes the quota check on the stored values', async () => {
    const h = await machine()

    const result = await setQuota(h, { daily_cap: 50, reserve_threshold: 25 })

    expect([result.item.status, result.item.last?.detail]).toEqual([
      'current',
      `daily cap 50, reserve threshold 25, 0 used today; set in ${overridePath(h)}`,
    ])
  })

  it('rejects a reserve above the daily cap', async () => {
    const h = await machine()

    await expect(
      h.commands.setup.run({ action: 'quota', daily_cap: 10, reserve_threshold: 11 }),
    ).rejects.toMatchObject({
      code: 'invalid-input',
      message: 'setup input: the reserve of 11 is above the daily cap of 10',
    })
  })

  it('writes nothing over a routing override that does not parse', async () => {
    const h = await machine({ routingOverride: '{ not json' })

    await expect(
      h.commands.setup.run({ action: 'quota', daily_cap: 50, reserve_threshold: 25 }),
    ).rejects.toMatchObject({
      code: 'invalid-routing',
    })
    expect(await h.readText(overridePath(h))).toBe('{ not json')
  })

  it('is due again after the stored limit changed', async () => {
    const h = await machine()
    await setQuota(h, { daily_cap: 50, reserve_threshold: 25 })
    await h.writeText(
      overridePath(h),
      JSON.stringify({ antigravity_quota: { daily_cap: 60, reserve_threshold: 25 } }),
    )

    expect(item(await list(h), 'agy-quota').changed).toEqual(['routing_hash'])
  })
})

describe("the repository's opt-in", () => {
  it('reports a repository with no allowlist entry as not opted in, with the families routing uses', async () => {
    const h = await machine()

    expect((await listed(h, { repo: h.repo })).repository).toEqual({
      code: false,
      families: [],
      history: false,
      key: 'github.com/jorisjonkers-dev/agent-kit',
      routed_families: ['mistral', 'google', 'zhipu', 'deepseek'],
      status: 'not-opted-in',
    })
  })

  it('reports the families and, separately, the code permission of an opted-in repository', async () => {
    const h = await machine()
    await h.commands.allow.run({ families: ['mistral', 'google'], repo: h.repo })

    expect((await listed(h, { repo: h.repo })).repository).toMatchObject({
      code: false,
      families: ['mistral', 'google'],
      status: 'opted-in',
    })
  })

  it('reports the history permission of an opted-in repository apart from its code permission', async () => {
    const h = await machine()
    await h.commands.allow.run({ families: ['mistral'], history: true, repo: h.repo })

    expect((await listed(h, { repo: h.repo })).repository).toMatchObject({
      code: false,
      history: true,
      status: 'opted-in',
    })
  })

  it('reports the refusal for a repository with no remote', async () => {
    const h = await machine({ remotes: [] })

    expect((await listed(h, { repo: h.repo })).repository).toMatchObject({
      reason: 'no-remote',
      status: 'refused',
    })
  })

  it('says nothing about a repository when none is named', async () => {
    const h = await machine()

    expect((await listed(h)).repository).toBeUndefined()
  })
})

describe("setup's input and results file", () => {
  it('lists when no action is named', async () => {
    const h = await machine()

    expect((await h.commands.setup.run({})).status).toBe('listed')
  })

  it('rejects a check that is not on the checklist', async () => {
    const h = await machine()

    await expect(
      h.commands.setup.run({ action: 'check', check: 'after-skill-hook' }),
    ).rejects.toMatchObject({
      code: 'invalid-input',
      message: "setup input: check after-skill-hook is not on this machine's checklist",
    })
  })

  it('rejects a results file that does not parse', async () => {
    const h = await machine()
    await h.writeText(join(dataDir(h), 'machine-setup.json'), '{"version": 1, "checks": {"x": {}}}')

    await expect(list(h)).rejects.toMatchObject({ code: 'invalid-setup' })
  })
})
