import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { ProcessCommand } from '../../../ports/index.js'
import { FakeEnv, FakeFiles, FakeVibeProcess, agyResult, agyStream, type VibeScript } from '../../../../test/model-gateway/harness.js'
import { ClientRun, type ClientRunRequest } from './client-run.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

/** One in-process launch: a temporary home, a temporary work directory, and a fake client that plays the scripts the test queues. */
interface Workspace {
  readonly dir: string
  readonly home: string
  readonly process: FakeVibeProcess
  readonly run: ClientRun
  /** The request a test sends unless it says otherwise: one Vibe launch of the critic's model. */
  request(overrides?: Partial<ClientRunRequest>): ClientRunRequest
}

async function workspace(scripts: readonly VibeScript[] = []): Promise<Workspace> {
  const dir = await mkdtemp(join(tmpdir(), 'model-gateway-client-run-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const home = join(dir, 'home')
  const workDir = join(dir, 'work')
  await mkdir(workDir, { recursive: true })
  const process = new FakeVibeProcess([...scripts])
  const run = new ClientRun({ env: new FakeEnv({ HOME: home }), files: new FakeFiles(), process })
  return {
    dir,
    home,
    process,
    request: (overrides = {}) => ({
      binary: 'vibe',
      client: 'vibe',
      extras: {},
      model: 'glm-5-3',
      paths: { exportDir: join(dir, 'export'), promptPath: join(dir, 'prompt.md'), workDir },
      prompt: 'read this',
      time_limit_s: 900,
      ...overrides,
    }),
    run,
  }
}

/** A home whose Vibe configuration declares two MCP servers, one of them named as the run's own. */
async function homeWithServers(w: Workspace): Promise<void> {
  await mkdir(join(w.home, '.vibe'), { recursive: true })
  await writeFile(
    join(w.home, '.vibe', 'config.toml'),
    '[[mcp_servers]]\nname = "hindsight"\ntransport = "stdio"\ncommand = "hindsight"\n\n[[mcp_servers]]\nname = "packet"\ntransport = "stdio"\ncommand = "rm"\n',
  )
}

/** The value of one flag of a launch. */
function flag(command: ProcessCommand | undefined, name: string): string | undefined {
  const at = command?.args.indexOf(name) ?? -1
  return at === -1 ? undefined : command?.args[at + 1]
}

/** The tools a launch enables, in the order it names them. */
function enabledTools(command: ProcessCommand | undefined): readonly string[] {
  const args = command?.args ?? []
  return args.flatMap((arg, i) => (args[i - 1] === '--enabled-tools' ? [arg] : []))
}

/** The extras a caller that starts its own search server sends: the server, the home off, the agent and the tools. */
const SEARCH_EXTRAS = {
  agent: 'critic-glm',
  disable_home_mcp: true,
  env: { VIBE_ENABLE_CONNECTORS: 'false' },
  mcp_servers: [{ args: ['server.mjs', '--root', '/p'], command: 'node', name: 'packet' }],
  tools: ['read_file', 'packet_grep'],
}

describe('the gateway launch path, for a caller inside this process', () => {
  it('attaches the servers the caller names, switches the home’s off, and enables the tools it names', async () => {
    const w = await workspace([{ exportJson: { cost_usd: 0.01 }, stdout: '{"type":"assistant","content":"fine"}\n' }])
    await homeWithServers(w)

    const outcome = await w.run.launch(w.request({ extras: SEARCH_EXTRAS }))

    expect(outcome.result.exitCode).toBe(0)
    const [command] = w.process.commands
    expect(command?.command).toBe('env')
    expect(command?.args.slice(0, 5)).toEqual(['-u', 'MISTRAL_API_KEY', '-u', 'VIBE_HOME', 'vibe'])
    expect(flag(command, '--agent')).toBe('critic-glm')
    expect(enabledTools(command)).toEqual(['read_file', 'packet_grep'])
    expect(command?.env).toEqual({
      VIBE_ENABLE_CONNECTORS: 'false',
      VIBE_ENABLE_MCP: 'true',
      VIBE_ENABLE_SUBAGENTS: 'false',
      VIBE_INCLUDE_PROJECT_CONTEXT: 'false',
      VIBE_MCP_SERVERS: JSON.stringify([
        {
          args: ['server.mjs', '--root', '/p'],
          command: 'node',
          cwd: join(w.dir, 'work'),
          name: 'packet',
          startup_timeout_sec: 10,
          tool_timeout_sec: 30,
          transport: 'stdio',
        },
        { command: 'false', disabled: true, name: 'hindsight', transport: 'stdio' },
      ]),
      VIBE_OUTSIDE_WORKDIR: 'deny',
    })
  })

  it('leaves the model to the agent a caller names, since the agent’s file pins one and Vibe refuses a second', async () => {
    const w = await workspace([{ stdout: '{"type":"assistant","content":"fine"}\n' }])

    await w.run.launch(w.request({ extras: { agent: 'critic-glm' } }))

    const [command] = w.process.commands
    expect(command?.args).not.toContain('--model')
    expect(command?.args).not.toContain('glm-5-3')
    expect(flag(command, '--agent')).toBe('critic-glm')
  })

  it('still names the model the run was checked for, beside the read-only agent, when no agent is named', async () => {
    const w = await workspace([{ stdout: '{"type":"assistant","content":"fine"}\n' }])

    await w.run.launch(w.request({ extras: { tools: ['read_file'] } }))

    const [command] = w.process.commands
    expect(flag(command, '--model')).toBe('glm-5-3')
    expect(flag(command, '--agent')).toBe('delegate-readonly')
  })

  it('leaves the home’s servers alone when the caller does not switch them off', async () => {
    const w = await workspace([{ stdout: '{"type":"assistant","content":"fine"}\n' }])
    await homeWithServers(w)

    await w.run.launch(w.request({ extras: { mcp_servers: [{ args: [], command: 'node', name: 'packet' }] } }))

    const servers = JSON.parse(String(w.process.commands[0]?.env?.VIBE_MCP_SERVERS)) as readonly { name: string }[]
    expect(servers.map((server) => server.name)).toEqual(['packet'])
  })

  it('refuses to switch the home off when its configuration does not parse, naming the file and the error', async () => {
    const w = await workspace()
    await mkdir(join(w.home, '.vibe'), { recursive: true })
    await writeFile(join(w.home, '.vibe', 'config.toml'), '[[mcp_servers]\n')

    await expect(
      w.run.launch(
        w.request({ extras: { disable_home_mcp: true, mcp_servers: [{ args: [], command: 'node', name: 'packet' }] } }),
      ),
    ).rejects.toMatchObject({
      code: 'home-config-unreadable',
      message: `${join(w.home, '.vibe', 'config.toml')}: cannot read the home's MCP servers: line 1: expected '.'`,
    })
    expect(w.process.commands).toEqual([])
  })

  it('switches nothing off when the home declares no configuration at all', async () => {
    const w = await workspace([{ stdout: '{"type":"assistant","content":"fine"}\n' }])

    await w.run.launch(w.request({ extras: { disable_home_mcp: true } }))

    expect(w.process.commands[0]?.env?.VIBE_MCP_SERVERS).toBeUndefined()
  })

  it('reads a home that declares its servers as one inline table, and leaves out one without a name', async () => {
    const w = await workspace([{ stdout: '{"type":"assistant","content":"fine"}\n' }])
    await mkdir(join(w.home, '.vibe'), { recursive: true })
    await writeFile(join(w.home, '.vibe', 'config.toml'), 'mcp_servers = { command = "hindsight", name = "hindsight" }\n')

    await w.run.launch(w.request({ extras: { disable_home_mcp: true } }))

    const servers = JSON.parse(String(w.process.commands[0]?.env?.VIBE_MCP_SERVERS)) as readonly { name: string }[]
    expect(servers.map((server) => server.name)).toEqual(['hindsight'])
  })

  it('switches nothing off when the home’s configuration declares no servers', async () => {
    const w = await workspace([{ stdout: '{"type":"assistant","content":"fine"}\n' }])
    await mkdir(join(w.home, '.vibe'), { recursive: true })
    await writeFile(join(w.home, '.vibe', 'config.toml'), 'theme = "dark"\n')

    await w.run.launch(w.request({ extras: { disable_home_mcp: true } }))

    expect(w.process.commands[0]?.env?.VIBE_MCP_SERVERS).toBeUndefined()
  })

  it('switches nothing off when no server the home declares has a name', async () => {
    const w = await workspace([{ stdout: '{"type":"assistant","content":"fine"}\n' }])
    await mkdir(join(w.home, '.vibe'), { recursive: true })
    await writeFile(join(w.home, '.vibe', 'config.toml'), 'mcp_servers = ["hindsight", { command = "hindsight" }]\n')

    await w.run.launch(w.request({ extras: { disable_home_mcp: true } }))

    expect(w.process.commands[0]?.env?.VIBE_MCP_SERVERS).toBeUndefined()
  })

  it('runs Antigravity in the read-only sandbox profile: a stream, the schema, and the tools it may use', async () => {
    const w = await workspace([{ stdout: agyStream(agyResult('looks fine')) }])

    const outcome = await w.run.launch(
      w.request({
        binary: 'agy',
        client: 'agy',
        extras: { sandbox: 'read-only', schema: '/assets/finding.schema.json', tools: ['view_file'] },
        model: 'gemini-3.8-flash-high',
      }),
    )

    expect(outcome.output.answer).toBe('looks fine')
    const [command] = w.process.commands
    expect(command?.args).toEqual([
      '-p',
      'read this',
      '--output-format',
      'stream-json',
      '--model',
      'gemini-3.8-flash-high',
      '--print-timeout',
      '900s',
      '--disable-slash-commands',
      '--sandbox',
      '--json-schema',
      '/assets/finding.schema.json',
      '--settings',
      JSON.stringify({
        permissions: {
          deny: ['mcp(*)', 'read_url(*)', 'execute_url(*)', 'command(*)', 'write_file(*)', 'list_dir(*)', 'grep_search(*)', 'find_by_name(*)'],
        },
      }),
    ])
  })

  it('denies only the tool families a searching Antigravity run does not name', async () => {
    const w = await workspace([{ stdout: agyResult('fine') }])

    await w.run.launch(
      w.request({
        binary: 'agy',
        client: 'agy',
        extras: { sandbox: 'read-only', tools: ['view_file', 'list_dir', 'grep_search', 'find_by_name'] },
        model: 'gemini-3.8-flash-high',
      }),
    )

    const settings = JSON.parse(String(flag(w.process.commands[0], '--settings'))) as { permissions: { deny: string[] } }
    expect(settings.permissions.deny).toEqual(['mcp(*)', 'read_url(*)', 'execute_url(*)', 'command(*)', 'write_file(*)'])
    expect(flag(w.process.commands[0], '--json-schema')).toBeUndefined()
  })

  it('refuses an extra the client cannot apply, naming the extra and the client', async () => {
    const w = await workspace()

    await expect(w.run.launch(w.request({ client: 'claude', extras: { sandbox: 'read-only' } }))).rejects.toMatchObject({
      code: 'extras-not-supported',
      message: 'claude cannot apply the sandbox extra',
    })
    await expect(w.run.launch(w.request({ client: 'agy', extras: { mcp_servers: [] } }))).rejects.toMatchObject({
      code: 'extras-not-supported',
      message: 'agy cannot apply the mcp_servers extra',
    })
    expect(w.process.commands).toEqual([])
  })

  it('refuses an extra its launcher would drop: a schema for Vibe, a home switch for Antigravity', async () => {
    const w = await workspace()

    await expect(w.run.launch(w.request({ extras: { schema: '/assets/finding.schema.json' } }))).rejects.toMatchObject({
      code: 'extras-not-supported',
      message: 'vibe cannot apply the schema extra',
    })
    await expect(w.run.launch(w.request({ client: 'agy', extras: { disable_home_mcp: true } }))).rejects.toMatchObject({
      code: 'extras-not-supported',
      message: 'agy cannot apply the disable_home_mcp extra',
    })
    expect(w.process.commands).toEqual([])
  })

  it('refuses a Vibe run that needs a web tool before launching anything, naming the tool', async () => {
    const w = await workspace()

    await expect(w.run.launch(w.request({ extras: { tools: ['read_file', 'web_fetch'] } }))).rejects.toMatchObject({
      code: 'extras-not-supported',
      message: expect.stringContaining('vibe cannot enable the web_fetch tool') as string,
    })
    expect(w.process.commands).toEqual([])
  })

  it('refuses the tools and the schema an Antigravity run applies only in the read-only sandbox', async () => {
    const w = await workspace()

    await expect(w.run.launch(w.request({ client: 'agy', extras: { tools: ['view_file'] } }))).rejects.toMatchObject({
      code: 'extras-not-supported',
      message: 'agy applies the tools extra only in the read-only sandbox, so it needs the sandbox extra too',
    })
    await expect(
      w.run.launch(w.request({ client: 'agy', extras: { schema: '/assets/finding.schema.json' } })),
    ).rejects.toMatchObject({
      code: 'extras-not-supported',
      message: 'agy applies the schema extra only in the read-only sandbox, so it needs the sandbox extra too',
    })
    expect(w.process.commands).toEqual([])
  })

  it('refuses an environment variable the launcher sets itself, naming the variable', async () => {
    const w = await workspace()

    await expect(w.run.launch(w.request({ extras: { env: { VIBE_OUTSIDE_WORKDIR: 'allow' } } }))).rejects.toMatchObject({
      code: 'extras-not-supported',
      message: 'vibe sets VIBE_OUTSIDE_WORKDIR itself, so extras.env may not name it',
    })
    await expect(w.run.launch(w.request({ extras: { env: { MISTRAL_API_KEY: 'sk-test' } } }))).rejects.toMatchObject({
      code: 'extras-not-supported',
      message: 'vibe sets MISTRAL_API_KEY itself, so extras.env may not name it',
    })
    expect(w.process.commands).toEqual([])
  })
})
