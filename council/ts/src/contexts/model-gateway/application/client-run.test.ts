import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { ProcessCommand } from '../../../ports/index.js'
import { FakeEnv, FakeFiles, FakeVibeProcess, agyResult, agyStream, type VibeScript } from '../../../../test/model-gateway/harness.js'
import { ClientRun, type ClientRunRequest } from './client-run.js'
import { LAUNCHERS, piProvider } from './launchers.js'

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

/** The values of every occurrence of one flag of a launch, in the order it names them. */
function flags(command: ProcessCommand | undefined, name: string): readonly string[] {
  const args = command?.args ?? []
  return args.flatMap((arg, i) => (args[i - 1] === name ? [arg] : []))
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

  it('refuses web_search for Vibe too, and lets other tools through', async () => {
    const w = await workspace([{ stdout: 'x' }])

    await expect(w.run.launch(w.request({ extras: { tools: ['web_search'] } }))).rejects.toMatchObject({
      message: expect.stringContaining('vibe cannot enable the web_search tool') as string,
    })
    expect(w.process.commands).toEqual([])
    await w.run.launch(w.request({ extras: { tools: ['read_file'] } })).catch(() => undefined)
    expect(w.process.commands).toHaveLength(1)
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

describe('the git directories a Codex writing run may write', () => {
  /** The git directories a writing run names: the worktree's own, and the repository's shared ones. */
  const GIT_WRITABLE: readonly string[] = ['/repo/.git/worktrees/run-1', '/repo/.git/objects', '/repo/.git/refs', '/repo/.git/logs']

  /** A launch of the workspace's work directory, with the git directories a test names. */
  function withGitWritable(request: ClientRunRequest, gitWritable: readonly string[]): ClientRunRequest {
    return { ...request, paths: { ...request.paths, gitWritable } }
  }

  it('adds each of them right after the work directory, so the sandbox can commit in the worktree', async () => {
    const w = await workspace([{ stdout: '' }])
    const request = w.request({ binary: 'codex', client: 'codex', mode: 'write' })

    await w.run.launch(withGitWritable(request, GIT_WRITABLE))

    const [command] = w.process.commands
    const args = command?.args ?? []
    const at = args.indexOf('-C')
    expect(args[at + 1]).toBe(join(w.dir, 'work'))
    expect(args.slice(at + 2)).toEqual(GIT_WRITABLE.flatMap((dir) => ['--add-dir', dir]))
    expect(flags(command, '--add-dir')).toEqual(GIT_WRITABLE)
    expect(flag(command, '-s')).toBe('workspace-write')
  })

  it('adds none for a read-only run, which may not write them', async () => {
    const w = await workspace([{ stdout: '' }])
    const request = w.request({ binary: 'codex', client: 'codex' })

    await w.run.launch(withGitWritable(request, GIT_WRITABLE))

    expect(flags(w.process.commands[0], '--add-dir')).toEqual([])
    expect(flag(w.process.commands[0], '-s')).toBe('read-only')
  })

  it('adds none for a writing run that names no git directory', async () => {
    const w = await workspace([{ stdout: '' }])

    await w.run.launch(w.request({ binary: 'codex', client: 'codex', mode: 'write' }))

    expect(flags(w.process.commands[0], '--add-dir')).toEqual([])
    expect(flag(w.process.commands[0], '-s')).toBe('workspace-write')
  })

  it('leaves another client’s launch alone, since only Codex has a sandbox to widen', async () => {
    const w = await workspace([{ stdout: '{"type":"result","result":"fine"}' }])
    const request = w.request({ binary: 'claude', client: 'claude', mode: 'write' })

    await w.run.launch(withGitWritable(request, GIT_WRITABLE))

    const [command] = w.process.commands
    expect(command?.args).not.toContain('--add-dir')
    expect(command?.args).not.toContain(GIT_WRITABLE[0])
  })
})

describe('a Pi launch', () => {
  const STREAM = JSON.stringify({ message: { content: [{ text: 'done\nSUMMARY: ok', type: 'text' }], role: 'assistant', usage: { input: 3, output: 2 } }, type: 'message_end' })

  /** A home with the route's provider, a launcher on PATH, and a process that answers the secret store and Pi. */
  async function piWorkspace(apiKey: string, answer: (command: ProcessCommand) => Promise<{ exitCode: number; stderr: string; stdout: string }>): Promise<{
    readonly agentDir: string
    readonly asked: ProcessCommand[]
    readonly request: (overrides?: Partial<ClientRunRequest>) => ClientRunRequest
    readonly run: ClientRun
    readonly dir: string
  }> {
    const dir = await mkdtemp(join(tmpdir(), 'model-gateway-pi-run-'))
    cleanups.push(() => rm(dir, { force: true, recursive: true }))
    const home = join(dir, 'home')
    await mkdir(join(home, '.pi', 'agent'), { recursive: true })
    await writeFile(
      join(home, '.pi', 'agent', 'models.json'),
      JSON.stringify({ providers: { 'litellm-vibe': { api: 'openai-completions', apiKey, baseUrl: 'http://127.0.0.1:4000/v1', models: [{ id: 'glm-5.3' }] } } }),
    )
    await mkdir(join(dir, 'bin'), { recursive: true })
    await writeFile(join(dir, 'bin', 'agent-launcher'), '')
    await mkdir(join(dir, 'work'), { recursive: true })
    const asked: ProcessCommand[] = []
    const run = new ClientRun({
      env: new FakeEnv({ HOME: home, PATH: join(dir, 'bin') }),
      files: new FakeFiles(),
      process: {
        exec: (command) => {
          asked.push(command)
          return answer(command).then((result) => ({ ...result, timedOut: false }))
        },
      },
    })
    const agentDir = join(dir, 'export', 'pi-agent')
    return {
      agentDir,
      asked,
      dir,
      request: (overrides = {}) => ({
        binary: 'pi',
        client: 'pi',
        extras: { env: { AGENT_KIT_DELEGATION: '1' } },
        mode: 'read-only',
        model: 'glm-5.3',
        paths: { exportDir: join(dir, 'export'), promptPath: join(dir, 'prompt.md'), workDir: join(dir, 'work') },
        prompt: 'read this',
        provider: 'litellm-vibe',
        time_limit_s: 120,
        ...overrides,
      }),
      run,
    }
  }

  const piAnswer = (command: ProcessCommand): Promise<{ exitCode: number; stderr: string; stdout: string }> =>
    Promise.resolve(command.command === 'agent-kit-secret' ? { exitCode: 0, stderr: '', stdout: 'the-key\n' } : { exitCode: 0, stderr: '', stdout: STREAM })

  it('runs Pi through the launcher, confined to its working directory, with the prompt on stdin, and removes the agent directory with the key', async () => {
    const w = await piWorkspace('!agent-kit-secret get LITELLM_VIBE_KEY', async (command) => {
      if (command.command !== 'agent-kit-secret') {
        expect((JSON.parse(await readFile(join(w.agentDir, 'models.json'), 'utf8')) as { providers: Record<string, { apiKey: string }> }).providers['litellm-vibe']?.apiKey).toBe('the-key')
      }
      return piAnswer(command)
    })

    const outcome = await w.run.launch(w.request())

    const launch = w.asked.at(-1)
    expect(launch).toMatchObject({
      command: 'agent-launcher',
      cwd: join(w.dir, 'work'),
      env: { AGENT_KIT_DELEGATION: '1', PATH: join(w.dir, 'bin'), PI_CODING_AGENT_DIR: w.agentDir, TMPDIR: w.agentDir },
      stdin: 'read this',
      timeoutMs: 120_000,
    })
    expect(launch?.args.slice(0, 11)).toEqual(['confine', '--read', join(w.dir, 'work'), '--run-from', 'pi', '--write', w.agentDir, '--net-connect', '4000', '--', 'pi'])
    expect(launch?.args).toContain('litellm-vibe')
    expect(launch?.args[launch.args.indexOf('--tools') + 1]).toBe('read,grep,find,ls')
    expect(outcome.output).toEqual({ answer: 'done\nSUMMARY: ok', charged: undefined, tokens: { cached: 0, input: 3, output: 2 } })
    await expect(readFile(join(w.agentDir, 'models.json'), 'utf8')).rejects.toThrow()
  })

  it('starts Pi with only the skills and plugins of its scope: a skill outside it is not in the run home during the run, an enforced one is, and both are gone after', async () => {
    const seen: { skills: string[]; extensions: string[] } = { extensions: [], skills: [] }
    const w = await piWorkspace('literal', async (command) => {
      if (command.command !== 'agent-kit-secret') {
        seen.skills = await readdir(join(w.agentDir, 'skills'))
        seen.extensions = await readdir(join(w.agentDir, 'extensions'))
      }
      return piAnswer(command)
    })
    const agent = join(w.dir, 'home', '.pi', 'agent')
    for (const name of ['enforced', 'outside']) {
      await mkdir(join(agent, 'skills', name), { recursive: true })
      await writeFile(join(agent, 'skills', name, 'SKILL.md'), `---\nname: ${name}\ndescription: d\n---\n`)
    }
    await mkdir(join(agent, 'extensions'), { recursive: true })
    await writeFile(join(agent, 'extensions', 'hello.ts'), 'export default () => {}\n')
    await writeFile(join(agent, 'extensions', 'other.ts'), 'export default () => {}\n')

    await w.run.launch(w.request(), { mcp: [], plugins: ['hello'], skills: ['enforced'] })

    expect(seen).toEqual({ extensions: ['hello.ts'], skills: ['enforced'] })
    const args = w.asked.at(-1)?.args ?? []
    expect(args).toEqual(expect.arrayContaining(['--no-skills', '--no-extensions', '--no-mcp']))
    expect(args[args.indexOf('--skill') + 1]).toBe(join(w.agentDir, 'skills', 'enforced'))
    expect(args[args.indexOf('-e') + 1]).toBe(join(w.agentDir, 'extensions', 'hello.ts'))
    expect(args).not.toContain(join(w.agentDir, 'skills', 'outside'))
    await expect(readdir(w.agentDir)).rejects.toThrow()
  })

  it('refuses to start a Pi run whose scope lists a skill the home does not have', async () => {
    const w = await piWorkspace('literal', piAnswer)

    await expect(w.run.launch(w.request(), { mcp: [], plugins: [], skills: ['ghost'] })).rejects.toMatchObject({ code: 'unconfined', message: expect.stringContaining("skill ghost is not in Pi's home") as string })
  })

  it('lets a writing run write its working directory and the git directories it names, and gives Pi the edit tools', async () => {
    const w = await piWorkspace('literal', piAnswer)
    const git = join(w.dir, 'git')

    await w.run.launch(w.request({ mode: 'write', paths: { ...w.request().paths, gitWritable: [git] } }))

    const launch = w.asked.at(-1)
    expect(launch?.args.slice(0, 6)).toEqual(['confine', '--write', join(w.dir, 'work'), '--write', git, '--run-from'])
    expect(launch?.args[launch.args.indexOf('--tools') + 1]).toBe('read,grep,find,ls,edit,write')
  })

  it('finds the home’s bin on PATH for the find tool, and removes the agent directory when the launch itself fails', async () => {
    const w = await piWorkspace('literal', (command) => (command.command === 'agent-launcher' ? Promise.reject(new Error('spawn failed')) : piAnswer(command)))
    await writeFile(join(w.dir, 'home', '.pi', 'agent', 'bin'), '')

    await expect(w.run.launch(w.request())).rejects.toThrow('spawn failed')

    const launch = w.asked.at(-1)
    expect(launch?.env?.PATH).toBe(`${join(w.dir, 'home', '.pi', 'agent', 'bin')}:${join(w.dir, 'bin')}`)
    expect(launch?.args).toContain(join(w.dir, 'home', '.pi', 'agent', 'bin'))
    await expect(readFile(join(w.agentDir, 'models.json'), 'utf8')).rejects.toThrow()
  })

  it('never starts Pi unconfined: no provider named, or none the home can give', async () => {
    const w = await piWorkspace('literal', piAnswer)

    await expect(w.run.launch(w.request({ provider: undefined as never }))).rejects.toMatchObject({ code: 'unconfined', message: 'a pi launch must name its provider' })
    await expect(w.run.launch(w.request({ provider: 'litellm-local' }))).rejects.toMatchObject({ code: 'unconfined' })
    expect(w.asked).toEqual([])
  })

  it('refuses an environment variable Pi’s launch sets itself', async () => {
    const w = await piWorkspace('literal', piAnswer)

    await expect(w.run.launch(w.request({ extras: { env: { PI_CODING_AGENT_DIR: '/elsewhere' } } }))).rejects.toMatchObject({
      message: 'pi sets PI_CODING_AGENT_DIR itself, so extras.env may not name it',
    })
  })

  it('has a bare command, started where it is told, when nothing confines it: the launcher’s own builder, never the run’s', () => {
    const command = LAUNCHERS.pi.command('pi', { model: 'glm-5.3', provider: 'litellm-vibe', time_limit_s: 30 }, { exportDir: '/e', promptPath: '/p', workDir: '/w' }, 'ask')
    expect(command).toMatchObject({ command: 'pi', cwd: '/w', env: {}, stdin: 'ask', timeoutMs: 30_000 })
    expect(command.args.slice(0, 7)).toEqual(['-p', '--mode', 'json', '--no-session', '--offline', '--provider', 'litellm-vibe'])
    expect(LAUNCHERS.pi.command('pi', { model: 'm', time_limit_s: 1 }, { exportDir: '/e', promptPath: '/p', workDir: '/w' }, 'x').args).toContain('undefined')
    expect(piProvider('pi-opencode-go')).toBe('litellm-opencode-go')
  })
})
