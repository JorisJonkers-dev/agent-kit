import { mkdir, mkdtemp, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { ProcessCommand, ProcessResult } from '../../../ports/index.js'
import { FakeEnv, FakeFiles } from '../../../../test/model-gateway/harness.js'
import { homeAgentDir, NO_LAUNCHER, preparePi, resolveScope, urlPort, type PiLaunchPorts } from './pi-launch.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

type Answer = ProcessResult | Error

interface Machine {
  readonly dir: string
  readonly home: string
  readonly agentDir: string
  readonly asked: ProcessCommand[]
  ports(env?: Record<string, string>, answer?: Answer): PiLaunchPorts
  /** Writes the home's models.json with one provider, and a launcher on PATH. */
  setup(provider: Record<string, unknown> | undefined, launcher?: boolean): Promise<void>
}

async function machine(): Promise<Machine> {
  const dir = await mkdtemp(join(tmpdir(), 'model-gateway-pi-launch-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const home = join(dir, 'home')
  const asked: ProcessCommand[] = []
  return {
    agentDir: join(dir, 'launch', 'pi-agent'),
    asked,
    dir,
    home,
    ports: (env = {}, answer = { exitCode: 0, stderr: '', stdout: 'the-key\n' }) => ({
      env: new FakeEnv({ HOME: home, PATH: join(dir, 'bin'), ...env }),
      files: new FakeFiles(),
      process: {
        exec: (command) => {
          asked.push(command)
          return answer instanceof Error ? Promise.reject(answer) : Promise.resolve(answer)
        },
      },
    }),
    setup: async (provider, launcher = true) => {
      await mkdir(join(home, '.pi', 'agent'), { recursive: true })
      await writeFile(join(home, '.pi', 'agent', 'models.json'), JSON.stringify({ providers: provider === undefined ? {} : { 'litellm-vibe': provider } }))
      if (launcher) {
        await mkdir(join(dir, 'bin'), { recursive: true })
        await writeFile(join(dir, 'bin', 'agent-launcher'), '')
      }
    },
  }
}

const provider = (apiKey: string, extra: Record<string, unknown> = {}): Record<string, unknown> => ({
  api: 'openai-completions',
  apiKey,
  baseUrl: 'https://litellm.example/v1',
  models: [{ id: 'other' }, { id: 'glm-5.3', cost: { input: 1 } }],
  ...extra,
})

describe('the home Pi directory and the proxy port', () => {
  it('is PI_CODING_AGENT_DIR when set, else ~/.pi/agent, and takes the scheme’s own port without one', () => {
    expect(homeAgentDir({ env: new FakeEnv({ HOME: '/h', PI_CODING_AGENT_DIR: '/own' }), files: new FakeFiles() })).toBe('/own')
    expect(homeAgentDir({ env: new FakeEnv({ HOME: '/h', PI_CODING_AGENT_DIR: '' }), files: new FakeFiles() })).toBe('/h/.pi/agent')
    expect(homeAgentDir({ env: new FakeEnv({}), files: new FakeFiles() })).toBe('.pi/agent')
    expect(urlPort('https://litellm.example/v1')).toBe(443)
    expect(urlPort('http://litellm.example')).toBe(80)
    expect(urlPort('http://127.0.0.1:4000/v1')).toBe(4000)
    expect(urlPort('http://host:0/v1')).toBeUndefined()
    expect(urlPort('http://host:70000/v1')).toBeUndefined()
    expect(urlPort('http://host:x/v1')).toBeUndefined()
    expect(urlPort('ftp://host/v1')).toBeUndefined()
  })
})

describe('preparing a confined Pi launch', () => {
  it('writes the route’s provider with its key, read from the secret store, into the launch’s own agent directory', async () => {
    const m = await machine()
    await m.setup(provider('!agent-kit-secret get LITELLM_VIBE_KEY'))
    // The fake files port tells a file from nothing; the real one answers for a directory too.
    await writeFile(join(m.home, '.pi', 'agent', 'bin'), '')

    const prepared = await preparePi(m.ports(), 'litellm-vibe', 'glm-5.3', m.agentDir)

    expect(prepared).toEqual({
      agentDir: m.agentDir,
      bin: join(m.home, '.pi', 'agent', 'bin'),
      launcher: 'agent-launcher',
      path: join(m.dir, 'bin'),
      port: 443,
    })
    expect(m.asked).toEqual([{ args: ['get', 'LITELLM_VIBE_KEY'], command: '!agent-kit-secret'.slice(1), timeoutMs: 20_000 }])
    expect(JSON.parse(await readFile(join(m.agentDir, 'models.json'), 'utf8'))).toEqual({
      providers: { 'litellm-vibe': { api: 'openai-completions', apiKey: 'the-key', baseUrl: 'https://litellm.example/v1', models: [{ cost: { input: 1 }, id: 'glm-5.3' }] } },
    })
    expect(await readFile(join(m.agentDir, 'settings.json'), 'utf8')).toBe('{}\n')
    expect((await stat(join(m.agentDir, 'models.json'))).mode & 0o777).toBe(0o600)
  })

  it('takes a literal key as it is, a model the home does not list as a bare id, and the launcher the environment names', async () => {
    const m = await machine()
    await m.setup(provider('literal'), false)

    const prepared = await preparePi(m.ports({ AGENT_KIT_LAUNCHER_BIN: '/opt/launcher', PATH: '' }), 'litellm-vibe', 'unlisted', m.agentDir)

    expect(prepared).toEqual({ agentDir: m.agentDir, launcher: '/opt/launcher', path: '', port: 443 })
    expect(m.asked).toEqual([])
    expect((JSON.parse(await readFile(join(m.agentDir, 'models.json'), 'utf8')) as { providers: Record<string, { models: unknown }> }).providers['litellm-vibe']?.models).toEqual([{ id: 'unlisted' }])
  })

  it('reads a home that names no PATH and a provider with no model list', async () => {
    const m = await machine()
    await m.setup(provider('literal', { models: 'none' }), false)
    const ports = m.ports({ AGENT_KIT_LAUNCHER_BIN: '/opt/launcher' })

    const prepared = await preparePi({ ...ports, env: new FakeEnv({ AGENT_KIT_LAUNCHER_BIN: '/opt/launcher', HOME: m.home }) }, 'litellm-vibe', 'glm-5.3', m.agentDir)

    expect(prepared).toEqual({ agentDir: m.agentDir, launcher: '/opt/launcher', port: 443 })
  })

  it('says why the home cannot give the route: no file, no provider, a provider with a field missing, a base URL with no port', async () => {
    const m = await machine()
    const path = join(m.home, '.pi', 'agent', 'models.json')
    expect(await preparePi(m.ports(), 'litellm-vibe', 'glm-5.3', m.agentDir)).toBe(`pi: ${path} cannot be read; re-run agent-kit setup so it writes the LiteLLM providers into ~/.pi/agent/models.json`)
    await m.setup(undefined)
    expect(await preparePi(m.ports(), 'litellm-vibe', 'glm-5.3', m.agentDir)).toContain('has no litellm-vibe provider')
    await writeFile(path, '[]')
    expect(await preparePi(m.ports(), 'litellm-vibe', 'glm-5.3', m.agentDir)).toContain('has no litellm-vibe provider')
    await writeFile(path, JSON.stringify({ providers: { 'litellm-vibe': { api: 'x' } } }))
    expect(await preparePi(m.ports(), 'litellm-vibe', 'glm-5.3', m.agentDir)).toContain('lacks a baseUrl, api or apiKey')
    await writeFile(path, JSON.stringify({ providers: { 'litellm-vibe': provider('k', { baseUrl: 'ftp://x' }) } }))
    expect(await preparePi(m.ports(), 'litellm-vibe', 'glm-5.3', m.agentDir)).toContain('not an http or https URL with a port')
    await rm(m.agentDir, { force: true, recursive: true })
  })

  it('refuses to start without a launcher, naming what setup installs', async () => {
    const m = await machine()
    await m.setup(provider('literal'), false)

    expect(await preparePi(m.ports({ PATH: ':/nowhere' }), 'litellm-vibe', 'glm-5.3', m.agentDir)).toBe(NO_LAUNCHER)
    expect(await preparePi(m.ports({ AGENT_KIT_LAUNCHER_BIN: '' }), 'litellm-vibe', 'glm-5.3', m.agentDir)).toBe(NO_LAUNCHER)
    await expect(stat(m.agentDir)).rejects.toThrow()
  })

  it('refuses a key that is a shell command, and one the secret store cannot give, without showing any of the key', async () => {
    const m = await machine()
    await m.setup(provider('!echo a | cat'))
    expect(await preparePi(m.ports(), 'litellm-vibe', 'glm-5.3', m.agentDir)).toContain('is a shell command the gateway does not run')
    await m.setup(provider('!agent-kit-secret get NAME'))
    const failed = (answer: Answer): Promise<string | object> => preparePi(m.ports({}, answer), 'litellm-vibe', 'glm-5.3', m.agentDir)
    expect(await failed({ exitCode: 1, stderr: '\n  no such secret\nmore', stdout: 'leak' })).toBe('pi: the litellm-vibe key could not be read from the secret store (NAME): no such secret')
    expect(await failed({ exitCode: 3, stderr: '', stdout: '' })).toBe('pi: the litellm-vibe key could not be read from the secret store (NAME): exit 3')
    expect(await failed({ exitCode: 0, stderr: '', stdout: '  \n' })).toContain('exit 0')
    expect(await failed({ exitCode: 0, stderr: 'slow', stdout: 'late', timedOut: true })).toContain('slow')
    expect(await failed(new Error('spawn agent-kit-secret ENOENT'))).toContain('spawn agent-kit-secret ENOENT')
    expect(await failed(new Error('x'))).not.toContain('leak')
    await expect(stat(m.agentDir)).rejects.toThrow()
  })

  it('refuses when the files port cannot write privately', async () => {
    const m = await machine()
    await m.setup(provider('literal'))
    const ports = m.ports()

    const prepared = await preparePi({ ...ports, files: { exists: (path) => ports.files.exists(path), readText: (path) => ports.files.readText(path) } }, 'litellm-vibe', 'glm-5.3', m.agentDir)

    expect(prepared).toBe('pi: the launch has no way to write its private agent directory')
  })
})

describe('a launch home built from a tool scope', () => {
  const scope = (skills: readonly string[], plugins: readonly string[] = []): { mcp: readonly string[]; plugins: readonly string[]; skills: readonly string[] } => ({ mcp: [], plugins, skills })

  /** A home with three skills (one in Pi's folder, one shared), and two extensions. */
  async function stocked(): Promise<Machine> {
    const m = await machine()
    await m.setup(provider('literal'))
    const agent = join(m.home, '.pi', 'agent')
    for (const [root, name] of [[join(agent, 'skills'), 'inside'], [join(agent, 'skills'), 'outside'], [join(m.home, '.agents', 'skills'), 'shared']] as const) {
      await mkdir(join(root, name, 'references'), { recursive: true })
      await writeFile(join(root, name, 'SKILL.md'), `---\nname: ${name}\ndescription: d\n---\n`)
      await writeFile(join(root, name, 'references', 'notes.md'), name)
    }
    await mkdir(join(agent, 'extensions'), { recursive: true })
    await writeFile(join(agent, 'extensions', 'hello.ts'), 'export default () => {}\n')
    await writeFile(join(agent, 'extensions', 'unlisted.js'), 'export default () => {}\n')
    await writeFile(join(agent, 'extensions', 'plain'), 'x')
    return m
  }

  it('holds the listed skills and plugins and no other: a skill outside the scope is not in the run home, an enforced one is', async () => {
    const m = await stocked()

    const prepared = await preparePi(m.ports(), 'litellm-vibe', 'glm-5.3', m.agentDir, scope(['inside', 'shared'], ['hello']))

    expect(prepared).toMatchObject({ extensions: [join(m.agentDir, 'extensions', 'hello.ts')], skills: [join(m.agentDir, 'skills', 'inside'), join(m.agentDir, 'skills', 'shared')] })
    expect(await readFile(join(m.agentDir, 'skills', 'inside', 'references', 'notes.md'), 'utf8')).toBe('inside')
    expect(await readFile(join(m.agentDir, 'skills', 'shared', 'SKILL.md'), 'utf8')).toContain('name: shared')
    await expect(stat(join(m.agentDir, 'skills', 'outside'))).rejects.toThrow()
    await expect(stat(join(m.agentDir, 'extensions', 'unlisted.js'))).rejects.toThrow()
    expect(await readdir(join(m.agentDir, 'skills'))).toEqual(['inside', 'shared'])
    expect((await stat(m.agentDir)).mode & 0o777).toBe(0o700)
  })

  it('finds a plugin by its folder, its .ts file or its .js file, and holds the empty scope as no scope at all', async () => {
    const m = await stocked()
    const ports = m.ports()
    expect(await resolveScope(ports, scope([], ['plain', 'hello', 'unlisted']))).toEqual({
      plugins: ['plain', 'hello', 'unlisted'].map((name, at) => ({ from: join(m.home, '.pi', 'agent', 'extensions', ['plain', 'hello.ts', 'unlisted.js'][at] ?? ''), name })),
      skills: [],
    })
    const empty = await preparePi(m.ports(), 'litellm-vibe', 'glm-5.3', m.agentDir, scope([]))
    expect(empty).not.toHaveProperty('skills')
    expect(empty).not.toHaveProperty('extensions')
    expect(await resolveScope(ports, { mcp: 'any', plugins: 'any', skills: 'any' })).toEqual({ plugins: [], skills: [] })
  })

  it('refuses a skill or plugin that is not in the home, naming where it looked, before anything is written or read from the store', async () => {
    const m = await stocked()

    const skill = await preparePi(m.ports(), 'litellm-vibe', 'glm-5.3', m.agentDir, scope(['inside', 'ghost']))
    const plugin = await preparePi(m.ports(), 'litellm-vibe', 'glm-5.3', m.agentDir, scope([], ['ghost']))

    expect(skill).toBe(`skill ghost is not in Pi's home: looked for ${join(m.home, '.pi/agent/skills/ghost/SKILL.md')} and ${join(m.home, '.agents/skills/ghost/SKILL.md')}`)
    expect(plugin).toContain("plugin ghost is not in Pi's home: looked for ")
    expect(plugin).toContain('ghost.ts')
    expect(plugin).toContain('ghost.js')
    await expect(stat(m.agentDir)).rejects.toThrow()
  })

  it('leaves no key behind when a copy or a write fails, and says what failed', async () => {
    const m = await stocked()
    const ports = m.ports()
    const failing = (copyTree: ((from: string, to: string) => Promise<void>) | undefined, writeTextPrivate?: (path: string, content: string) => Promise<void>): PiLaunchPorts => ({
      ...ports,
      files: { exists: (path) => ports.files.exists(path), readText: (path) => ports.files.readText(path), writeTextPrivate: writeTextPrivate ?? ((path, content) => ports.files.writeTextPrivate?.(path, content) ?? Promise.resolve()), ...(copyTree === undefined ? {} : { copyTree }) },
    })

    expect(await preparePi(failing(undefined), 'litellm-vibe', 'glm-5.3', m.agentDir, scope(['inside']))).toBe('pi: the launch has no way to copy skill inside into its agent directory')
    expect(await preparePi(failing(() => Promise.reject(new Error('EACCES: denied'))), 'litellm-vibe', 'glm-5.3', m.agentDir, scope([], ['hello']))).toBe(
      `pi: cannot copy plugin hello into the launch's agent directory ${m.agentDir}: EACCES: denied`,
    )
    await expect(stat(join(m.agentDir, 'models.json'))).rejects.toThrow()
    const settings = await preparePi(failing(undefined, () => Promise.reject(new Error('EACCES: no settings'))), 'litellm-vibe', 'glm-5.3', m.agentDir)
    expect(settings).toBe(`pi: cannot write the launch's agent directory ${m.agentDir}: EACCES: no settings`)
    let writes = 0
    const models = await preparePi(failing(undefined, (path, content) => (++writes === 2 ? Promise.reject(new Error('EACCES: no models')) : ports.files.writeTextPrivate?.(path, content) ?? Promise.resolve())), 'litellm-vibe', 'glm-5.3', m.agentDir)
    expect(models).toBe(`pi: cannot write the launch's agent directory ${m.agentDir}: EACCES: no models`)
  })
})
