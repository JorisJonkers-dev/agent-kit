import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { fenced, finding, harness, piStream, type Harness, type PiCli } from '../../../test/critique/harness.js'
import type { CriticLaunch } from './application/ports.js'
import { clientReadiness, clientSignIn, clientVersion, reviewWith } from './application/ports.js'
import { authReadiness, classifyPiRun, modelsReadiness, outsidePacket, readPiStream, PI_SYSTEM_PROMPT } from './adapters/pi/index.js'
import { readCriticClient, type PiCriticClient } from './domain/routing.js'
import { JsonFields } from './domain/json.js'
import type { FanOutResult, PrepareResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const PI_CLIENT = { family: 'zhipu', kind: 'pi', model: 'glm-5-3', provider: 'litellm-vibe-study' } as const
const CONFIG: PiCriticClient = { binary: 'pi', ...PI_CLIENT }

/** A routing override whose spec roster is one Pi critic, base and deep. */
function piRouting(client: Record<string, unknown> = PI_CLIENT): Record<string, unknown> {
  const slot = { client: 'pi-vibe-study-glm', lenses: ['fidelity'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 600 }
  return { clients: { 'pi-vibe-study-glm': client }, stages: { spec: { base: [slot], deep: [] } } }
}

async function piOnly(cli: PiCli, client?: Record<string, unknown>): Promise<{ h: Harness; run: PrepareResult }> {
  const h = await harness({ piCli: cli, routingOverride: piRouting(client) })
  open.push(h)
  await h.commands.allow.run({ families: ['zhipu'], repo: h.repo })
  const run = (await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })) as PrepareResult
  return { h, run }
}

async function piReview(stdout: string): Promise<{ h: Harness; run: PrepareResult; result: FanOutResult }> {
  const { h, run } = await piOnly({ scripts: [{ stdout }] })
  const result = await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { h, result, run }
}

function argAfter(args: readonly string[], flag: string): string | undefined {
  const at = args.indexOf(flag)
  return at === -1 ? undefined : args[at + 1]
}

describe('routing a Pi critic client', () => {
  const read = (value: unknown) => readCriticClient(JsonFields.of(value, 'clients.x'))

  it('reads the provider, model and family, with pi as the binary unless routing names one', () => {
    expect(read(PI_CLIENT)).toEqual(CONFIG)
    expect(read({ ...PI_CLIENT, binary: '/opt/pi' })).toEqual({ ...CONFIG, binary: '/opt/pi' })
  })

  it("refuses a provider that is not a LiteLLM route's, and a family outside the model families", () => {
    expect(() => read({ ...PI_CLIENT, provider: 'openai' })).toThrow(/litellm-<route>/)
    expect(() => read({ ...PI_CLIENT, provider: 'litellm-' })).toThrow(/litellm-<route>/)
    expect(() => read({ ...PI_CLIENT, family: 'nobody' })).toThrow(/family/)
  })
})

describe('the Pi critic', () => {
  it('runs pi in print mode with its JSON stream, the prompt on stdin and the read tools alone, in the packet copy', async () => {
    const { h, result, run } = await piReview(piStream(`Review.\n${fenced({ findings: [finding()] })}`))

    const [command] = h.pi?.launches ?? []
    const criticDir = join(run.run_dir, 'critics', 'c1-pi-vibe-study-glm')
    expect(command?.command).toBe('pi')
    expect(command?.cwd).toBe(join(criticDir, 'packet'))
    expect(command?.stdin).toBe(await readFile(join(criticDir, 'prompt.md'), 'utf8'))
    expect(command?.timeoutMs).toBe(600_000)
    const args = command?.args ?? []
    expect(args.slice(0, 4)).toEqual(['-p', '--mode', 'json', '--no-session'])
    expect(argAfter(args, '--provider')).toBe('litellm-vibe-study')
    expect(argAfter(args, '--model')).toBe('glm-5-3')
    expect(argAfter(args, '--tools')).toBe('read,grep,find,ls')
    expect(argAfter(args, '--system-prompt')).toBe(PI_SYSTEM_PROMPT)
    expect(args).toEqual(
      expect.arrayContaining(['--offline', '--no-extensions', '--no-skills', '--no-prompt-templates', '--no-themes', '--no-context-files', '--no-mcp', '--no-approve']),
    )
    expect(result.critics[0]).toMatchObject({ client: 'pi-vibe-study-glm', cost_usd: 0.002, status: 'ok', tokens: { cached: 30, input: 130, output: 20 }, tools: ['read', 'grep', 'find', 'ls'] })
    const prompt = await readFile(join(criticDir, 'prompt.md'), 'utf8')
    expect(prompt).toContain('`read` reads one file, `ls` lists a directory')
    expect(JSON.parse(await readFile(join(criticDir, 'result.json'), 'utf8'))).toEqual({
      answer: `Review.\n${fenced({ findings: [finding()] })}`,
      cost_usd: 0.002,
      tokens: { cached: 30, input: 130, output: 20 },
    })
  })

  it('logs the launch in both ledgers as a pi invocation, charged what Pi priced it at', async () => {
    const { h } = await piReview(piStream(`Review.\n${fenced({ findings: [] })}`))

    const gateway = (await h.gatewayLedger()).trim().split('\n').map((line) => JSON.parse(line) as Record<string, unknown>)
    expect(gateway.at(-1)).toMatchObject({ charged_usd: 0.002, client: 'pi', critic_client: 'pi-vibe-study-glm', kind: 'pi', model: 'glm-5-3' })
    const ledger = await readFile(join(h.home, '.local', 'state', 'agent-kit', 'critique', 'ledger.jsonl'), 'utf8').catch(() => '')
    if (ledger.length > 0) expect(ledger).toContain('"kind":"pi"')
  })

  it('voids a review that used a tool beyond the read tools, or reached past its packet', async () => {
    const bash = await piReview(piStream(fenced({ findings: [] }), { calls: [{ args: { command: 'ls' }, tool: 'bash' }] }))
    expect(bash.result.critics[0]).toMatchObject({ detail: 'the critic used tools beyond reading its packet: bash', failure: 'unsafe-tool', status: 'failed' })

    const outside = await piReview(piStream(fenced({ findings: [] }), { calls: [{ args: { path: '/etc/hostname' }, tool: 'read' }] }))
    expect(outside.result.critics[0]).toMatchObject({ detail: 'the critic reached outside its packet: read /etc/hostname', failure: 'unsafe-tool' })
  })

  it('reads a provider error as a failed critic although pi exits 0', async () => {
    const { result } = await piReview(piStream('', { errorMessage: 'Connection error.', stopReason: 'error' }))
    expect(result.critics[0]).toMatchObject({ detail: 'pi stopped on an error: Connection error.', failure: 'infrastructure-failure', status: 'failed' })
  })

  it('drops the critic in prepare when its provider is not ready', async () => {
    const { run } = await piOnly({ auth: 'not_ready' })
    expect(JSON.stringify(run)).toContain('the litellm-vibe-study provider is not_ready (provider_not_found)')
  })
})

describe('the Pi stream', () => {
  const ok = { exitCode: 0, stderr: '', stdout: 'x' }
  const classify = (stdout: string, result: Partial<typeof ok> & { timedOut?: boolean } = {}) =>
    classifyPiRun({ ...ok, stdout, ...result }, readPiStream(stdout), '/runs/c1/packet', 600)

  it('fails a run the outer timer killed, one that exited non-zero, and one that printed nothing', () => {
    expect(classify('', { timedOut: true })).toMatchObject({ detail: 'pi did not return within 600s, so the outer timer killed it', failure: 'timeout' })
    expect(classify('', { exitCode: 2, stderr: '\n  bad flag\n' })).toMatchObject({ detail: 'pi exited 2: bad flag', failure: 'exit-2' })
    expect(classify('', { exitCode: 3 })).toMatchObject({ detail: 'pi exited 3', failure: 'exit-3' })
    expect(classify('  \n')).toMatchObject({ failure: 'empty-output' })
  })

  it('fails output that is not a JSON event stream, or ends in no assistant message', () => {
    expect(classify('not json\n')).toMatchObject({ detail: 'stdout is not a Pi JSON event stream', failure: 'unparseable-output' })
    expect(classify('[1]\n')).toMatchObject({ failure: 'unparseable-output' })
    expect(classify('{"type":"agent_start"}\n')).toMatchObject({ detail: 'stdout is not a Pi event stream with an assistant message', failure: 'unparseable-output' })
  })

  it('fails an aborted run, an error without a message, and a reply without a findings block', () => {
    expect(classify(piStream('', { stopReason: 'aborted' }))).toMatchObject({ detail: 'pi was aborted', failure: 'stopped-early' })
    expect(classify(piStream('', { stopReason: 'error' }))).toMatchObject({ detail: 'pi stopped on an error', failure: 'infrastructure-failure' })
    expect(classify(piStream('no block'))).toMatchObject({ failure: 'no-findings-block', status: 'failed' })
  })

  it('keeps a no-findings reason, and sums usage over every assistant message', () => {
    const turns = [
      piStream('first', { usage: { cost: { total: 0.25 }, input: 10, output: 1 } }),
      piStream(fenced({ findings: [], no_findings_reason: 'nothing' }), { usage: { input: 5, output: 2 } }),
    ].join('\n')
    const stream = readPiStream(turns)
    expect(stream).toMatchObject({ answer: fenced({ findings: [], no_findings_reason: 'nothing' }), costUsd: 0.25, tokens: { cached: 0, input: 15, output: 3 } })
    expect(classifyPiRun({ ...ok, stdout: turns }, stream, '/p', 1)).toEqual({ cost_usd: 0.25, findings: [], no_findings_reason: 'nothing', status: 'ok' })
  })

  it('counts no tokens or price a message does not report, and reads only text parts', () => {
    const stdout = [
      JSON.stringify({ message: { content: [{ text: 'a', type: 'text' }, { type: 'thinking' }, { text: 7, type: 'text' }], role: 'assistant' }, type: 'message_end' }),
      JSON.stringify({ message: { content: 'plain', role: 'assistant', usage: { cost: { total: -1 }, input: 1 } }, type: 'message_end' }),
    ].join('\n')
    expect(readPiStream(stdout)).toMatchObject({ answer: '', costUsd: null, tokens: undefined })
    expect(readPiStream(stdout.split('\n')[0] ?? '').answer).toBe('a')
  })

  it('names every call that left the packet once, by its tool and argument', () => {
    const calls = [
      { args: { path: 'tree/a.ts' }, tool: 'read' },
      { args: { path: '/runs/c1/packet/tree/b.ts' }, tool: 'read' },
      { args: { path: '/runs/c1/packet' }, tool: 'ls' },
      { args: { glob: '../**' }, tool: 'grep' },
      { args: { pattern: '/etc/*' }, tool: 'find' },
      { args: { pattern: '../x' }, tool: 'grep' },
      { args: { path: '~/.ssh/id' }, tool: 'read' },
      { args: { path: '~/.ssh/id' }, tool: 'read' },
      { tool: 'ls' },
    ]
    expect(classify(piStream(fenced({ findings: [] }), { calls: calls as { tool: string; args: Record<string, unknown> }[] }))).toMatchObject({
      detail: 'the critic reached outside its packet: grep ../**, find /etc/*, read ~/.ssh/id',
      failure: 'unsafe-tool',
    })
  })

  it('resolves a path the way Pi does before judging it', () => {
    const root = '/runs/c1/packet'
    expect(outsidePacket('tree/../tree/a.ts', root)).toBe(false)
    expect(outsidePacket('./a/./b', root)).toBe(false)
    expect(outsidePacket('@tree/a.ts', root)).toBe(false)
    expect(outsidePacket('..', root)).toBe(true)
    expect(outsidePacket('a/../../b', root)).toBe(true)
    expect(outsidePacket('@/etc/passwd', root)).toBe(true)
    expect(outsidePacket('~', root)).toBe(true)
    expect(outsidePacket('/runs/c1/packet/../secret', root)).toBe(true)
    expect(outsidePacket('/runs/c1/packetx', root)).toBe(true)
    expect(outsidePacket('/runs', root)).toBe(true)
    expect(outsidePacket('/../runs/c1/packet/a', root)).toBe(true)
  })
})

describe("the Pi client's readiness", () => {
  const result = (stdout: string, exitCode = 0) => ({ exitCode, stderr: '', stdout })

  it('reads pi auth check: ready, a status with or without its reason, and an answer it cannot read', () => {
    expect(authReadiness(result('{"status":"ready"}\n'), CONFIG)).toEqual({ ready: true })
    expect(authReadiness(result('{"status":"not_ready","reason":"provider_not_found"}', 1), CONFIG)).toEqual({
      detail: 'pi: the litellm-vibe-study provider is not_ready (provider_not_found); re-run agent-kit setup so it writes the LiteLLM providers into ~/.pi/agent/models.json',
      ready: false,
    })
    expect(authReadiness(result('{"status":"invalid"}', 2), CONFIG)).toMatchObject({ detail: expect.stringContaining('is invalid; re-run') as string })
    expect(authReadiness(result('Error: unknown command', 1), CONFIG)).toEqual({ detail: 'pi: auth check for litellm-vibe-study failed: Error: unknown command', ready: false })
    expect(authReadiness(result('', 1), CONFIG)).toEqual({ detail: 'pi: auth check for litellm-vibe-study failed: exit 1', ready: false })
    expect(authReadiness('spawn pi EACCES', CONFIG)).toEqual({ detail: 'pi: auth check for litellm-vibe-study failed: spawn pi EACCES', ready: false })
    expect(authReadiness({ ...result(''), timedOut: true }, CONFIG)).toEqual({ detail: 'pi: auth check for litellm-vibe-study failed: no answer within 20s', ready: false })
  })

  it('reads pi --list-models: the routed model under the route provider, or why not', () => {
    expect(modelsReadiness(result('provider model\nlitellm-vibe-study  glm-5-3  128K\n'), CONFIG)).toEqual({ ready: true })
    expect(modelsReadiness(result('litellm-other glm-5-3\nlitellm-vibe-study glm-5-3-mini\n'), CONFIG)).toEqual({
      detail: 'pi: the litellm-vibe-study provider does not offer glm-5-3',
      ready: false,
    })
    expect(modelsReadiness(result('boom', 1), CONFIG)).toEqual({ detail: 'pi: pi --list-models failed: boom', ready: false })
  })

  it('needs pi on PATH, then a ready provider, then the model, and reads its version', async () => {
    const unoffered = await piOnly({ models: [['litellm-vibe-study', 'other']] })
    expect(JSON.stringify(unoffered.run)).toContain('does not offer glm-5-3')
    const offPath = await piOnly({ onPath: false })
    expect(JSON.stringify(offPath.run)).toContain('pi: pi is not on PATH')
    const spawn = await piOnly({ auth: 'spawn-error' })
    expect(JSON.stringify(spawn.run)).toContain('pi: auth check for litellm-vibe-study failed: spawn pi EACCES')
    const absolute = await piOnly({}, { ...PI_CLIENT, binary: '/nowhere/pi' })
    expect(JSON.stringify(absolute.run)).toContain('pi: /nowhere/pi is not on PATH')
  })
})

describe('a machine that wires no Pi client', () => {
  const launch = { critic: { client_config: CONFIG } } as unknown as CriticLaunch

  it('drops every Pi critic as unwired, and reads no version', async () => {
    const unwired = { detail: 'no pi critic client is wired on this machine', ready: false }
    const clients = {} as Parameters<typeof clientReadiness>[0]
    await expect(clientReadiness(clients, CONFIG)).resolves.toEqual(unwired)
    await expect(clientSignIn(clients, CONFIG)).resolves.toEqual(unwired)
    await expect(clientVersion(clients, CONFIG)).resolves.toBeNull()
    await expect(reviewWith(clients, launch)).rejects.toThrow('no pi critic client is wired on this machine')
  })
})

describe("setup's Pi items", () => {
  async function setupHarness(cli: PiCli = {}): Promise<Harness> {
    const h = await harness({ piCli: cli, routingOverride: piRouting() })
    open.push(h)
    return h
  }

  it('lists the provider check, one trivial-prompt check per Pi client, and a read-only item that is not available', async () => {
    const h = await setupHarness()
    const listed = await h.commands.setup.run({ action: 'list' })
    if (listed.status !== 'listed') throw new Error('not listed')
    const ids = listed.items.map((item) => item.id)
    expect(ids).toEqual(expect.arrayContaining(['pi-signed-in', 'critic:pi-vibe-study-glm', 'read-only:pi-vibe-study-glm']))
    expect(listed.items.find((item) => item.id === 'critic:pi-vibe-study-glm')).toMatchObject({ cost: 'pi', why: expect.stringContaining('glm-5-3 runs through pi on litellm-vibe-study') as string })
    expect(listed.items.find((item) => item.id === 'read-only:pi-vibe-study-glm')).toMatchObject({ cost: 'free', status: 'not-available' })
  })

  it('passes the provider check for free, and runs the trivial prompt through pi once confirmed', async () => {
    const h = await setupHarness({ scripts: [{ stdout: piStream(fenced({ findings: [], no_findings_reason: 'setup check: nothing to review' })) }] })
    expect(await h.commands.setup.run({ action: 'check', check: 'pi-signed-in' })).toMatchObject({ item: { last: { detail: 'pi is installed and signed in', result: 'pass' } }, status: 'checked' })
    expect(await h.commands.setup.run({ action: 'check', check: 'critic:pi-vibe-study-glm' })).toMatchObject({ status: 'needs-confirmation' })
    expect(await h.commands.setup.run({ action: 'check', check: 'critic:pi-vibe-study-glm', confirmed: true })).toMatchObject({
      item: { last: { detail: 'pi-vibe-study-glm answered the trivial prompt', result: 'pass' } },
    })
    expect(await h.commands.setup.run({ action: 'check', check: 'read-only:pi-vibe-study-glm' })).toMatchObject({ status: 'not-available' })
  })

  it('fails the provider check with the reason pi gives', async () => {
    const h = await setupHarness({ auth: 'invalid' })
    expect(await h.commands.setup.run({ action: 'check', check: 'pi-signed-in' })).toMatchObject({ item: { last: { result: 'fail' } } })
  })
})
