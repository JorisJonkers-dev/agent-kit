import { chmod, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { ProcessCommand, ProcessResult } from '../ports/index.js'
import type { GitRemote } from '../shared-kernel/index.js'
import {
  FakeEnv,
  FakeGit,
  FakeHermesProcess,
  FakeProcess,
  FakeVibeProcess,
  type FakeRepository,
  type VibeScript,
} from '../../test/model-gateway/harness.js'
import { defaultModelGatewayRuntime, MODEL_GATEWAY_USAGE, runModelGatewayCli, type ModelGatewayCliResult } from './index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

/** The defaults agent-kit ships, which the bundle reads from its own folder. */
const SHIPPED_DEFAULTS = join(import.meta.dirname, '../../../../skills/workstation/model-gateway/defaults.json')

/** What `agy models` prints: a fetching line, then one `<id>\t<display name>` per model. */
const AGY_LISTING: ProcessResult = {
  exitCode: 0,
  stderr: '',
  stdout: [
    'Fetching available models...',
    'gemini-3.8-flash-high\tGemini 3.8 Flash (High)',
    'gemini-4-pro\tGemini 4 Pro',
    'claude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)',
    'gpt-oss-120b-medium\tGPT-OSS 120B (Medium)',
    '',
  ].join('\n'),
}

/** What `opencode models` prints: one `<provider>/<model>` per line. */
const OPENCODE_LISTING: ProcessResult = {
  exitCode: 0,
  stderr: '',
  stdout: [
    'opencode-go/gemini-4-argon',
    'opencode-go/deepseek-v4.1-flash',
    'opencode-go/glm-5.3',
    'opencode-go/kimi-k3',
    'other/gemini-4-argon',
    'opencode-go/hy3',
    '',
  ].join('\n'),
}

/** A Vibe configuration: three models, two of them in the shipped catalogue, declared with their aliases and names. */
const VIBE_CONFIG = `[models.mistral-large-4-0]
name = "mistral-large-4-0"
provider = "mistral"
alias = "mistral-large-latest"

[models.zai-glm-5-3]
name = "zai-glm-5-3"
provider = "mistral"
alias = "glm-5-3"

[models.mistral-medium-4-1]
name = "mistral-medium-4-1"
provider = "mistral"
alias = "mistral-medium-4-1"
`

interface Workspace {
  readonly dir: string
  readonly home: string
  readonly defaults: string
  readonly override: string
  readonly allowlist: string
  readonly asked: readonly string[]
  readonly ran: readonly ProcessCommand[]
  run(...argv: string[]): Promise<ModelGatewayCliResult>
  writeAllowlist(value: unknown): Promise<void>
  writeDefaults(text: string): Promise<void>
  writeInput(name: string, text: string): Promise<string>
  writeOverride(value: unknown): Promise<void>
  writeVibeConfig(text: string): Promise<string>
}

interface WorkspaceOptions {
  readonly installed?: readonly string[]
  readonly env?: Readonly<Record<string, string>>
  readonly listings?: Readonly<Record<string, ProcessResult>>
  readonly repositories?: Readonly<Record<string, FakeRepository>>
  /** Leaves HOME out of the environment, as a session that never set it has none. */
  readonly withoutHome?: boolean
}

async function workspace(options: WorkspaceOptions = {}): Promise<Workspace> {
  const dir = await mkdtemp(join(tmpdir(), 'model-gateway-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const home = join(dir, 'home')
  const defaults = join(dir, 'defaults.json')
  await writeFile(defaults, await readFile(SHIPPED_DEFAULTS, 'utf8'))
  const process = new FakeProcess(options.installed ?? ['claude', 'codex'], options.listings ?? {})
  const runtime = defaultModelGatewayRuntime({
    defaultsPath: defaults,
    env: new FakeEnv(options.withoutHome === true ? { ...options.env } : { HOME: home, ...options.env }),
    git: new FakeGit(options.repositories ?? {}),
    process,
  })
  return {
    allowlist: join(home, '.config', 'agent-kit', 'critique-allowlist.json'),
    asked: process.asked,
    defaults,
    dir,
    home,
    override: join(home, '.config', 'agent-kit', 'model-gateway.json'),
    ran: process.ran,
    run: (...argv) => runModelGatewayCli(argv, runtime),
    writeAllowlist: async (value) => {
      await mkdir(join(home, '.config', 'agent-kit'), { recursive: true })
      await writeFile(join(home, '.config', 'agent-kit', 'critique-allowlist.json'), JSON.stringify(value))
    },
    writeDefaults: (text) => writeFile(defaults, text),
    writeInput: async (name, text) => {
      const path = join(dir, name)
      await writeFile(path, text)
      return path
    },
    writeOverride: async (value) => {
      await mkdir(join(home, '.config', 'agent-kit'), { recursive: true })
      await writeFile(join(home, '.config', 'agent-kit', 'model-gateway.json'), JSON.stringify(value))
    },
    writeVibeConfig: async (text) => {
      const path = join(home, '.vibe', 'config.toml')
      await mkdir(join(home, '.vibe'), { recursive: true })
      await writeFile(path, text)
      return path
    },
  }
}

/** The one JSON object a run printed. */
function printed(result: ModelGatewayCliResult): Record<string, unknown> {
  return JSON.parse(result.stdout) as Record<string, unknown>
}

/** The models a run listed, as `<client>:<model>` keys in the order it printed them. */
function listed(result: ModelGatewayCliResult): string[] {
  const models = printed(result).models as { client: string; model: string }[]
  return models.map((entry) => `${entry.client}:${entry.model}`)
}

/** One listed model, by its `<client>:<model>` key. */
function model(result: ModelGatewayCliResult, key: string): Record<string, unknown> {
  const models = printed(result).models as Record<string, unknown>[]
  const found = models.find((entry) => `${String(entry.client)}:${String(entry.model)}` === key)
  if (found === undefined) throw new Error(`${key} is not listed`)
  return found
}
/** One listed model, as the run printed it, or undefined when the run left it out. */
function maybeModel(result: ModelGatewayCliResult, key: string): Record<string, unknown> | undefined {
  const models = printed(result).models as { client: string; model: string }[]
  return models.find((entry) => `${entry.client}:${entry.model}` === key)
}

/** The repository the tests work in, and the remote that keys it. */
const REPO = '/work/agent-kit'
const KEY = 'github.com/jorisjonkers-dev/agent-kit'
const AGENT_KIT_REMOTE: GitRemote = { name: 'origin', url: 'git@github.com:JorisJonkers-dev/agent-kit.git' }

function repository(remotes: readonly GitRemote[] = [AGENT_KIT_REMOTE]): FakeRepository {
  return { remotes, root: REPO }
}

/** An allowlist file with one entry for the test repository. */
function allowlist(families: readonly string[], code = false, history = false): unknown {
  return { repositories: { [KEY]: { code, families, history, updated_at: '2026-11-04T00:00:00.000Z' } }, version: 1 }
}

describe('model-gateway models', () => {
  it('lists the declared models of every installed client, sorted by client then model', async () => {
    const w = await workspace({ repositories: { [REPO]: repository() } })
    await w.writeAllowlist(allowlist(['anthropic', 'openai'], true, true))

    const result = await w.run('models', '--json', JSON.stringify({ repo: REPO }))

    expect(result.exitCode).toBe(0)
    expect(result.stderr).toBe('')
    expect(printed(result)).toEqual({
      missing: [
        { client: 'agy', reason: 'not installed: agy is not on PATH' },
        { client: 'hermes', reason: 'not installed: hermes is not on PATH' },
        { client: 'opencode', reason: 'not installed: opencode is not on PATH' },
        { client: 'vibe', reason: 'not installed: vibe is not on PATH' },
      ],
      models: [
        {
          client: 'claude',
          cost_class: 'subsidised',
          family: 'anthropic',
          inline_only: false,
          may_send: { code: true, history: true },
          model: 'claude-haiku-5-5',
          notes: 'Fast and cheap; mechanical edits, summaries, lookups.',
          source: 'declared',
          tier: 'routine',
        },
        {
          client: 'claude',
          cost_class: 'subsidised',
          family: 'anthropic',
          inline_only: false,
          may_send: { code: true, history: true },
          model: 'claude-opus-5-5',
          notes: 'Strongest Claude; design, hard debugging, review of risky changes.',
          source: 'declared',
          tier: 'frontier',
        },
        {
          client: 'claude',
          cost_class: 'subsidised',
          family: 'anthropic',
          inline_only: false,
          may_send: { code: true, history: true },
          model: 'claude-sonnet-5-5',
          notes: 'Strong general coding and refactoring.',
          source: 'declared',
          tier: 'hard',
        },
        {
          client: 'codex',
          cost_class: 'metered-employer',
          family: 'openai',
          inline_only: false,
          may_send: { code: true, history: true },
          model: 'gpt-5.5',
          notes: 'Strong implementation and review; metered on the nedap account, grant needed.',
          source: 'declared',
          tier: 'frontier',
        },
      ],
      policy: `${KEY}: the entry allows anthropic, openai`,
      skipped: [],
      status: 'ok',
    })
  })

  it('reads the same input from a file with --input', async () => {
    const w = await workspace({ repositories: { [REPO]: repository() } })
    await w.writeAllowlist(allowlist(['anthropic', 'openai']))
    const input = await w.writeInput('input.json', JSON.stringify({ repo: REPO }))

    const result = await w.run('models', '--input', input)

    expect(result.exitCode).toBe(0)
    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5', 'codex:gpt-5.5'])
  })

  it('asks PATH about every client it knows, and runs no client it does not have to', async () => {
    const w = await workspace()

    await w.run('models', '--json', '{}')

    expect(w.asked).toEqual(['agy', 'claude', 'codex', 'hermes', 'opencode', 'vibe'])
    expect(w.ran).toEqual([])
  })

  it('leaves out a client that is not installed and names it with the reason', async () => {
    const w = await workspace({ installed: ['claude'] })

    const result = await w.run('models', '--json', '{}')

    expect(printed(result).missing).toEqual([
      { client: 'agy', reason: 'not installed: agy is not on PATH' },
      { client: 'codex', reason: 'not installed: codex is not on PATH' },
      { client: 'hermes', reason: 'not installed: hermes is not on PATH' },
      { client: 'opencode', reason: 'not installed: opencode is not on PATH' },
      { client: 'vibe', reason: 'not installed: vibe is not on PATH' },
    ])
    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
  })

  it('lists no model when no client is installed, and names every client it knows', async () => {
    const w = await workspace({ installed: [] })

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual([])
    expect(printed(result).missing).toEqual([
      { client: 'agy', reason: 'not installed: agy is not on PATH' },
      { client: 'claude', reason: 'not installed: claude is not on PATH' },
      { client: 'codex', reason: 'not installed: codex is not on PATH' },
      { client: 'hermes', reason: 'not installed: hermes is not on PATH' },
      { client: 'opencode', reason: 'not installed: opencode is not on PATH' },
      { client: 'vibe', reason: 'not installed: vibe is not on PATH' },
    ])
  })

  it('applies the machine override over the shipped defaults', async () => {
    const w = await workspace({ repositories: { [REPO]: repository() } })
    await w.writeAllowlist(allowlist(['anthropic', 'openai']))
    await w.writeOverride({
      clients: { codex: { cost_class: 'subsidised' } },
      disabled: ['claude:claude-haiku-5-5'],
      models: [
        { client: 'claude', model: 'claude-sonnet-5-5', notes: 'the one to use', tier: 'frontier' },
        { client: 'claude', family: 'anthropic', model: 'claude-opus-5-6', notes: 'newer', tier: 'frontier' },
      ],
      settings: { summary_max_chars: 500 },
    })

    const result = await w.run('models', '--json', JSON.stringify({ repo: REPO }))

    expect(listed(result)).toEqual(['claude:claude-opus-5-5', 'claude:claude-opus-5-6', 'claude:claude-sonnet-5-5', 'codex:gpt-5.5'])
    const models = printed(result).models as { client: string; cost_class: string; model: string; notes: string; tier: string }[]
    expect(models.find((entry) => entry.model === 'gpt-5.5')?.cost_class).toBe('subsidised')
    expect(models.find((entry) => entry.model === 'claude-sonnet-5-5')).toMatchObject({ notes: 'the one to use', tier: 'frontier' })
  })

  it('reads the override from XDG_CONFIG_HOME when it is set', async () => {
    const w = await workspace({ env: { XDG_CONFIG_HOME: join(tmpdir(), 'model-gateway-xdg') } })
    const xdg = join(tmpdir(), 'model-gateway-xdg', 'agent-kit')
    cleanups.push(() => rm(join(tmpdir(), 'model-gateway-xdg'), { force: true, recursive: true }))
    await mkdir(xdg, { recursive: true })
    await writeFile(join(xdg, 'model-gateway.json'), JSON.stringify({ disabled: ['codex:gpt-5.5'] }))

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
  })

  it('reports an override that is not JSON as invalid-config, naming the file', async () => {
    const w = await workspace()
    await mkdir(join(w.home, '.config', 'agent-kit'), { recursive: true })
    await writeFile(w.override, '{ not json')

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.override)
  })

  it('reports an override that does not fit the defaults as invalid-config, naming the file', async () => {
    const w = await workspace()
    await w.writeOverride({ clients: { codex: { cost_class: 'free' } } })

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toBe(
      `${w.override}: clients.codex.cost_class must be one of subsidised, metered-personal, metered-employer`,
    )
  })

  it('reports an override it cannot read as invalid-config, naming the file', async () => {
    const w = await workspace()
    await mkdir(w.override, { recursive: true })

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.override)
  })

  it('reports an override behind a directory it may not enter as invalid-config, naming the file', async () => {
    const w = await workspace()
    await w.writeOverride({ disabled: ['codex:gpt-5.5'] })
    const folder = join(w.home, '.config', 'agent-kit')
    await chmod(folder, 0o000)
    cleanups.push(() => chmod(folder, 0o755))

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.override)
  })

  it('reports shipped defaults that are not JSON as invalid-config, naming the file', async () => {
    const w = await workspace()
    await w.writeDefaults('nope')

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.defaults)
  })

  it('reports shipped defaults that do not validate as invalid-config, naming the file', async () => {
    const w = await workspace()
    await w.writeDefaults(JSON.stringify({ version: 2 }))

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toBe(`${w.defaults}: version must be 1`)
  })
})

describe('the data policy', () => {
  it('lists only the client models of the families the repository entry allows', async () => {
    const w = await workspace({ repositories: { [REPO]: repository() } })
    await w.writeAllowlist(allowlist(['anthropic']))

    const result = await w.run('models', '--json', JSON.stringify({ repo: REPO }))

    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
    expect(printed(result).policy).toBe(`${KEY}: the entry allows anthropic`)
  })

  it('reports what the entry lets a delegation send, code and history', async () => {
    const w = await workspace({ repositories: { [REPO]: repository() } })
    await w.writeAllowlist(allowlist(['anthropic'], true, false))

    const result = await w.run('models', '--json', JSON.stringify({ repo: REPO }))

    expect(maybeModel(result, 'claude:claude-opus-5-5')).toMatchObject({ inline_only: false, may_send: { code: true, history: false } })
  })

  it('lists nothing for a repository with a remote but no entry, and says why', async () => {
    const w = await workspace({ repositories: { [REPO]: repository() } })

    const result = await w.run('models', '--json', JSON.stringify({ repo: REPO }))

    expect(result.exitCode).toBe(0)
    expect(listed(result)).toEqual([])
    expect(printed(result).policy).toBe(`${KEY} has no allowlist entry, so nothing is sent`)
  })

  it('lists nothing for a repository with no remote, and says why', async () => {
    const w = await workspace({ repositories: { [REPO]: repository([]) } })
    await w.writeAllowlist(allowlist(['anthropic']))

    const result = await w.run('models', '--json', JSON.stringify({ repo: REPO }))

    expect(listed(result)).toEqual([])
    expect(printed(result).policy).toBe('the repository has no remote, so nothing is sent')
  })

  it('lists only Codex and Claude Code for a nedap repository, whatever its entry says', async () => {
    const w = await workspace({
      repositories: { [REPO]: repository([{ name: 'origin', url: 'git@github.com:Nedap/widgets.git' }]) },
    })
    await w.writeAllowlist(allowlist(['mistral']))

    const result = await w.run('models', '--json', JSON.stringify({ repo: REPO }))

    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5', 'codex:gpt-5.5'])
    expect(maybeModel(result, 'codex:gpt-5.5')).toMatchObject({ inline_only: false, may_send: { code: true, history: true } })
    expect(printed(result).policy).toBe('a nedap repository: only Codex and Claude Code, with code and history')
  })

  it('lists only subsidised and metered-personal client models outside any repository, marked inline-only', async () => {
    const w = await workspace({ installed: ['claude', 'codex', 'hermes'] })
    await w.writeOverride({
      clients: { hermes: { model_source: 'declared' } },
      models: [{ client: 'hermes', family: 'mistral', model: 'hermes-4', notes: 'Metered on the owner account.', tier: 'routine' }],
    })

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5', 'hermes:hermes-4'])
    expect(maybeModel(result, 'hermes:hermes-4')).toMatchObject({ inline_only: true, may_send: { code: false, history: false } })
    expect(printed(result).policy).toBe('no repository: only inline content, to subsidised and metered-personal clients')
  })

  it('treats a path outside any repository as outside', async () => {
    const w = await workspace()

    const result = await w.run('models', '--json', JSON.stringify({ repo: '/tmp/nowhere' }))

    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
    expect(maybeModel(result, 'claude:claude-opus-5-5')).toMatchObject({ inline_only: true })
  })

  it('reads the allowlist from XDG_CONFIG_HOME when it is set', async () => {
    const xdg = join(tmpdir(), 'model-gateway-allowlist-xdg')
    cleanups.push(() => rm(xdg, { force: true, recursive: true }))
    await mkdir(join(xdg, 'agent-kit'), { recursive: true })
    await writeFile(join(xdg, 'agent-kit', 'critique-allowlist.json'), JSON.stringify(allowlist(['anthropic'])))
    const w = await workspace({ env: { XDG_CONFIG_HOME: xdg }, repositories: { [REPO]: repository() } })

    const result = await w.run('models', '--json', JSON.stringify({ repo: REPO }))

    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
  })

  it('reports an allowlist that is not JSON as invalid-config, naming the file', async () => {
    const w = await workspace({ repositories: { [REPO]: repository() } })
    await mkdir(join(w.home, '.config', 'agent-kit'), { recursive: true })
    await writeFile(w.allowlist, '{ not json')

    const result = await w.run('models', '--json', JSON.stringify({ repo: REPO }))

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.allowlist)
  })

  it('reports an allowlist entry that does not fit as invalid-config, naming the file', async () => {
    const w = await workspace({ repositories: { [REPO]: repository() } })
    await w.writeAllowlist({ repositories: { [KEY]: { code: 'yes', families: ['anthropic'], updated_at: 'x' } }, version: 1 })

    const result = await w.run('models', '--json', JSON.stringify({ repo: REPO }))

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toBe(`${w.allowlist}: repositories.${KEY}.code must be true or false`)
  })

  it('fails with repository-unreadable when git cannot read the path, naming it and git\'s message', async () => {
    const w = await workspace({
      repositories: { [REPO]: { error: "fatal: detected dubious ownership in repository at '/work/agent-kit'" } },
    })

    const result = await w.run('models', '--json', JSON.stringify({ repo: REPO }))

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'repository-unreadable', status: 'error' })
    expect(String(printed(result).message)).toBe(
      `${REPO}: fatal: detected dubious ownership in repository at '/work/agent-kit'`,
    )
  })

  it('refuses a repo that is not a non-empty string', async () => {
    const w = await workspace()

    const result = await w.run('models', '--json', JSON.stringify({ repo: 7 }))

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-input', message: 'repo must be a non-empty string', status: 'error' })
  })
})

describe('the shipped defaults', () => {
  it('names every client, model and setting the contract ships', async () => {
    const shipped: unknown = JSON.parse(await readFile(SHIPPED_DEFAULTS, 'utf8'))

    expect(shipped).toEqual({
      clients: {
        agy: { binary: 'agy', cost_class: 'subsidised', families: ['google'], model_source: 'live' },
        claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'declared' },
        codex: { binary: 'codex', cost_class: 'metered-employer', model_source: 'declared' },
        hermes: { binary: 'hermes', cost_class: 'metered-personal', model_source: 'config' },
        opencode: {
          binary: 'opencode',
          cost_class: 'metered-personal',
          model_source: 'live',
          provider_cost_classes: { 'opencode-go': 'subsidised' },
        },
        vibe: { binary: 'vibe', cost_class: 'subsidised', model_source: 'config' },
      },
      models: [
        {
          client: 'claude',
          family: 'anthropic',
          model: 'claude-opus-5-5',
          notes: 'Strongest Claude; design, hard debugging, review of risky changes.',
          tier: 'frontier',
        },
        {
          client: 'claude',
          family: 'anthropic',
          model: 'claude-sonnet-5-5',
          notes: 'Strong general coding and refactoring.',
          tier: 'hard',
        },
        {
          client: 'claude',
          family: 'anthropic',
          model: 'claude-haiku-5-5',
          notes: 'Fast and cheap; mechanical edits, summaries, lookups.',
          tier: 'routine',
        },
        {
          client: 'codex',
          family: 'openai',
          model: 'gpt-5.5',
          notes: 'Strong implementation and review; metered on the nedap account, grant needed.',
          tier: 'frontier',
        },
        {
          client: 'vibe',
          family: 'deepseek',
          model: 'deepseek-v4-1-flash-1017',
          notes: 'Cheap and quick; good at well-specified code changes and lint-like review.',
          prices: { cached_input: 0.03, input: 0.3, output: 1.2 },
          tier: 'routine',
        },
        {
          client: 'vibe',
          family: 'zhipu',
          model: 'glm-5-3',
          names: ['zai-glm-5-3'],
          notes: 'Strong code review and integration reasoning.',
          prices: { cached_input: 0.14, input: 1.4, output: 4.4 },
          tier: 'hard',
        },
        {
          client: 'vibe',
          family: 'mistral',
          model: 'mistral-large-latest',
          names: ['mistral-large-4-0'],
          notes: 'Broad general model.',
          prices: { cached_input: 0.07, input: 0.7, output: 2.1 },
          tier: 'hard',
        },
        {
          client: 'vibe',
          family: 'mistral',
          model: 'mistral-small-latest',
          names: ['mistral-small-latest'],
          notes: 'Very cheap; short summaries and classification.',
          prices: { cached_input: 0.015, input: 0.15, output: 0.6 },
          tier: 'trivial',
        },
        {
          client: 'agy',
          family: 'google',
          model: 'gemini-3.8-flash-high',
          notes: 'Long context; reading large codebases and documents.',
          tier: 'hard',
        },
      ],
      settings: {
        availability_window_minutes: 60,
        max_parallel_per_client: 3,
        metered_personal_caps: { per_day_usd: 5, per_workflow_usd: 2 },
        plan_warning_ratio: 0.8,
        summary_max_chars: 1500,
      },
      version: 1,
    })
  })
})

describe('model-gateway command line', () => {
  it('prints one JSON object and exits 1 for a subcommand it does not have', async () => {
    const w = await workspace()

    const result = await w.run('delegate', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(result.stdout).toBe(
      `${JSON.stringify({ status: 'error', code: 'unknown-command', message: 'unknown subcommand "delegate"; the gateway has models, run, workflow start, workflow end and workflow list' })}\n`,
    )
  })

  it('names a missing subcommand', async () => {
    const w = await workspace()

    const result = await w.run()

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'unknown-command', status: 'error' })
  })

  it('refuses a command without --json or --input', async () => {
    const w = await workspace()

    const result = await w.run('models')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-input', status: 'error' })
  })

  it('refuses a command with more than one input', async () => {
    const w = await workspace()

    const result = await w.run('models', '--json', '{}', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-input', status: 'error' })
  })

  it('refuses a flag it does not know', async () => {
    const w = await workspace()

    const result = await w.run('models', '--repo', 'agent-kit')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-input', status: 'error' })
  })

  it('refuses input that is not JSON', async () => {
    const w = await workspace()

    const result = await w.run('models', '--json', '{')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-input', status: 'error' })
  })

  it('refuses input that is not a JSON object', async () => {
    const w = await workspace()

    const result = await w.run('models', '--json', '[]')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-input', message: 'the input must be a JSON object', status: 'error' })
  })

  it('refuses an input file it cannot read, naming the file', async () => {
    const w = await workspace()
    const missing = join(w.dir, 'nowhere.json')

    const result = await w.run('models', '--input', missing)

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-input', status: 'error' })
    expect(String(printed(result).message)).toContain(missing)
  })

  it('prints its usage for --help', async () => {
    const w = await workspace()

    const result = await w.run('--help')

    expect(result).toEqual({ exitCode: 0, stderr: '', stdout: MODEL_GATEWAY_USAGE })
  })
})

describe('models: the clients whose models the gateway declares', () => {
  it('lists the declared models of every installed client the policy permits, sorted by client then model', async () => {
    const w = await workspace()

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(0)
    expect(result.stderr).toBe('')
    expect(printed(result)).toEqual({
      missing: [
        { client: 'agy', reason: 'not installed: agy is not on PATH' },
        { client: 'hermes', reason: 'not installed: hermes is not on PATH' },
        { client: 'opencode', reason: 'not installed: opencode is not on PATH' },
        { client: 'vibe', reason: 'not installed: vibe is not on PATH' },
      ],
      models: [
        {
          client: 'claude',
          cost_class: 'subsidised',
          family: 'anthropic',
          inline_only: true,
          may_send: { code: false, history: false },
          model: 'claude-haiku-5-5',
          notes: 'Fast and cheap; mechanical edits, summaries, lookups.',
          source: 'declared',
          tier: 'routine',
        },
        {
          client: 'claude',
          cost_class: 'subsidised',
          family: 'anthropic',
          inline_only: true,
          may_send: { code: false, history: false },
          model: 'claude-opus-5-5',
          notes: 'Strongest Claude; design, hard debugging, review of risky changes.',
          source: 'declared',
          tier: 'frontier',
        },
        {
          client: 'claude',
          cost_class: 'subsidised',
          family: 'anthropic',
          inline_only: true,
          may_send: { code: false, history: false },
          model: 'claude-sonnet-5-5',
          notes: 'Strong general coding and refactoring.',
          source: 'declared',
          tier: 'hard',
        },
      ],
      policy: 'no repository: only inline content, to subsidised and metered-personal clients',
      skipped: [],
      status: 'ok',
    })
  })

  it('reads the same input from a file with --input', async () => {
    const w = await workspace()
    const input = await w.writeInput('input.json', '{}')

    const result = await w.run('models', '--input', input)

    expect(result.exitCode).toBe(0)
    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
  })

  it('asks PATH about every client it knows, and runs no client it does not have to', async () => {
    const w = await workspace()

    await w.run('models', '--json', '{}')

    expect(w.asked).toEqual(['agy', 'claude', 'codex', 'hermes', 'opencode', 'vibe'])
    expect(w.ran).toEqual([])
  })

  it('leaves out a client that is not installed and names it with the reason', async () => {
    const w = await workspace({ installed: ['claude'] })

    const result = await w.run('models', '--json', '{}')

    expect(printed(result).missing).toEqual([
      { client: 'agy', reason: 'not installed: agy is not on PATH' },
      { client: 'codex', reason: 'not installed: codex is not on PATH' },
      { client: 'hermes', reason: 'not installed: hermes is not on PATH' },
      { client: 'opencode', reason: 'not installed: opencode is not on PATH' },
      { client: 'vibe', reason: 'not installed: vibe is not on PATH' },
    ])
    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
  })

  it('leaves out an installed client whose listing cannot be run, and names it with the reason', async () => {
    const w = await workspace({ installed: ['hermes'] })

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual([])
    expect(printed(result).missing).toContainEqual({
      client: 'hermes',
      reason: 'listing failed: hermes config get model --json: cannot run hermes',
    })
  })

  it('applies the machine override over the shipped defaults', async () => {
    const w = await workspace()
    await w.writeOverride({
      clients: { codex: { cost_class: 'subsidised' } },
      disabled: ['claude:claude-haiku-5-5'],
      models: [
        { client: 'claude', model: 'claude-sonnet-5-5', notes: 'the one to use', tier: 'frontier' },
        { client: 'claude', family: 'anthropic', model: 'claude-opus-5-6', notes: 'newer', tier: 'frontier' },
      ],
      settings: { summary_max_chars: 500 },
    })

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual(['claude:claude-opus-5-5', 'claude:claude-opus-5-6', 'claude:claude-sonnet-5-5', 'codex:gpt-5.5'])
    const models = printed(result).models as { client: string; cost_class: string; model: string; notes: string; tier: string }[]
    expect(models.find((entry) => entry.model === 'gpt-5.5')?.cost_class).toBe('subsidised')
    expect(models.find((entry) => entry.model === 'claude-sonnet-5-5')).toMatchObject({ notes: 'the one to use', tier: 'frontier' })
  })

  it('reads the override from XDG_CONFIG_HOME when it is set', async () => {
    const w = await workspace({ env: { XDG_CONFIG_HOME: join(tmpdir(), 'model-gateway-xdg') } })
    const xdg = join(tmpdir(), 'model-gateway-xdg', 'agent-kit')
    cleanups.push(() => rm(join(tmpdir(), 'model-gateway-xdg'), { force: true, recursive: true }))
    await mkdir(xdg, { recursive: true })
    await writeFile(join(xdg, 'model-gateway.json'), JSON.stringify({ disabled: ['codex:gpt-5.5'] }))

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
  })

  it('reports an override that is not JSON as invalid-config, naming the file', async () => {
    const w = await workspace()
    await mkdir(join(w.home, '.config', 'agent-kit'), { recursive: true })
    await writeFile(w.override, '{ not json')

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.override)
  })

  it('reports an override that does not fit the defaults as invalid-config, naming the file', async () => {
    const w = await workspace()
    await w.writeOverride({ clients: { codex: { cost_class: 'free' } } })

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toBe(
      `${w.override}: clients.codex.cost_class must be one of subsidised, metered-personal, metered-employer`,
    )
  })

  it('reports an override it cannot read as invalid-config, naming the file', async () => {
    const w = await workspace()
    await mkdir(w.override, { recursive: true })

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.override)
  })

  it('reports an override behind a directory it may not enter as invalid-config, naming the file', async () => {
    const w = await workspace()
    await w.writeOverride({ disabled: ['codex:gpt-5.5'] })
    const folder = join(w.home, '.config', 'agent-kit')
    await chmod(folder, 0o000)
    cleanups.push(() => chmod(folder, 0o755))

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.override)
  })

  it('reports shipped defaults that are not JSON as invalid-config, naming the file', async () => {
    const w = await workspace()
    await w.writeDefaults('nope')

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.defaults)
  })

  it('reports shipped defaults that do not validate as invalid-config, naming the file', async () => {
    const w = await workspace()
    await w.writeDefaults(JSON.stringify({ version: 2 }))

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toBe(`${w.defaults}: version must be 1`)
  })
})

describe('models: Antigravity, read from its own listing', () => {
  it('lists the Google models it names, marked live, and leaves the other families out', async () => {
    const w = await workspace({ installed: ['agy'], listings: { agy: AGY_LISTING } })

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual(['agy:gemini-3.8-flash-high', 'agy:gemini-4-pro'])
    expect(model(result, 'agy:gemini-3.8-flash-high')).toEqual({
      client: 'agy',
      cost_class: 'subsidised',
      family: 'google',
      inline_only: true,
      may_send: { code: false, history: false },
      model: 'gemini-3.8-flash-high',
      notes: 'Long context; reading large codebases and documents.',
      source: 'live',
      tier: 'hard',
    })
    expect(w.ran).toEqual([{ args: ['models'], command: 'agy' }])
  })

  it('lists a model the catalogue does not know with no tier and no notes', async () => {
    const w = await workspace({ installed: ['agy'], listings: { agy: AGY_LISTING } })

    const result = await w.run('models', '--json', '{}')

    expect(model(result, 'agy:gemini-4-pro')).toEqual({
      client: 'agy',
      cost_class: 'subsidised',
      family: 'google',
      inline_only: true,
      may_send: { code: false, history: false },
      model: 'gemini-4-pro',
      notes: '',
      source: 'live',
      tier: null,
    })
  })

  it('leaves out a model the machine override disabled', async () => {
    const w = await workspace({ installed: ['agy'], listings: { agy: AGY_LISTING } })
    await w.writeOverride({ disabled: ['agy:gemini-4-pro'] })

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual(['agy:gemini-3.8-flash-high'])
  })

  it('leaves the client out with the reason when its listing fails, and still lists the others', async () => {
    const w = await workspace({
      installed: ['agy', 'claude'],
      listings: { agy: { exitCode: 1, stderr: 'not signed in\n', stdout: '' } },
    })

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(0)
    expect(printed(result).missing).toContainEqual({ client: 'agy', reason: 'listing failed: agy models exited 1: not signed in' })
    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
  })

  it('leaves the client out with the reason when its listing cannot be run at all', async () => {
    const w = await workspace({ installed: ['agy'] })

    const result = await w.run('models', '--json', '{}')

    expect(printed(result).missing).toContainEqual({ client: 'agy', reason: 'listing failed: agy models: cannot run agy' })
  })
})

describe('models: Vibe, read from its own configuration', () => {
  it('lists the aliases its configuration declares, marked config, joined to the catalogue', async () => {
    const w = await workspace({ installed: ['vibe'] })
    await w.writeVibeConfig(VIBE_CONFIG)

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual(['vibe:glm-5-3', 'vibe:mistral-large-latest', 'vibe:mistral-medium-4-1'])
    expect(model(result, 'vibe:glm-5-3')).toEqual({
      client: 'vibe',
      cost_class: 'subsidised',
      family: 'zhipu',
      inline_only: true,
      may_send: { code: false, history: false },
      model: 'glm-5-3',
      notes: 'Strong code review and integration reasoning.',
      source: 'config',
      tier: 'hard',
    })
    expect(model(result, 'vibe:mistral-medium-4-1')).toMatchObject({ family: 'mistral', notes: '', tier: null })
  })

  it('lists a model once, by its alias, when the catalogue knows it by the name its configuration gives it', async () => {
    const w = await workspace({ installed: ['vibe'] })
    await w.writeVibeConfig('[models.glm]\nname = "zai-glm-5-3"\nprovider = "mistral"\nalias = "glm-5-3"\n')

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual(['vibe:glm-5-3'])
    expect(model(result, 'vibe:glm-5-3')).toMatchObject({
      family: 'zhipu',
      notes: 'Strong code review and integration reasoning.',
      tier: 'hard',
    })
  })

  it('lists a model whose alias and name both miss the catalogue with no tier', async () => {
    const w = await workspace({ installed: ['vibe'] })
    await w.writeVibeConfig('[models.new]\nname = "mistral-medium-4-1"\nprovider = "mistral"\nalias = "medium"\n')

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual(['vibe:medium'])
    expect(model(result, 'vibe:medium')).toMatchObject({ family: 'mistral', notes: '', tier: null })
  })

  it('reads the configuration from VIBE_HOME when it is set', async () => {
    const w = await workspace({ env: { VIBE_HOME: join(tmpdir(), 'model-gateway-vibe-home') }, installed: ['vibe'] })
    const home = join(tmpdir(), 'model-gateway-vibe-home')
    cleanups.push(() => rm(home, { force: true, recursive: true }))
    await mkdir(home, { recursive: true })
    await writeFile(join(home, 'config.toml'), VIBE_CONFIG)

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual(['vibe:glm-5-3', 'vibe:mistral-large-latest', 'vibe:mistral-medium-4-1'])
  })

  it('leaves the client out with the reason when it has no configuration, and still lists the others', async () => {
    const w = await workspace({ installed: ['claude', 'vibe'] })

    const result = await w.run('models', '--json', '{}')

    expect(printed(result).missing).toContainEqual({
      client: 'vibe',
      reason: `no configuration: ${join(w.home, '.vibe', 'config.toml')} is not there`,
    })
    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
  })

  it('leaves the client out with the reason when it cannot tell where its configuration is, and still lists the others', async () => {
    const w = await workspace({
      env: { XDG_CONFIG_HOME: join(tmpdir(), 'model-gateway-no-home') },
      installed: ['claude', 'vibe'],
      withoutHome: true,
    })

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(0)
    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
    expect(printed(result).missing).toContainEqual({
      client: 'vibe',
      reason: 'configuration unreadable: ~/.vibe/config.toml: required environment variable is missing: HOME',
    })
  })

  it('leaves the client out with the reason when its configuration cannot be read', async () => {
    const w = await workspace({ installed: ['vibe'] })
    const path = await w.writeVibeConfig('this is not TOML\n')

    const result = await w.run('models', '--json', '{}')

    expect(printed(result).missing).toContainEqual({
      client: 'vibe',
      reason: `configuration unreadable: ${path}: line 1: expected '.'`,
    })
  })
})

describe('models: OpenCode, read from its own listing', () => {
  it('lists each provider model, marked live, with the family its model names', async () => {
    const w = await workspace({ installed: ['opencode'], listings: { opencode: OPENCODE_LISTING } })

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual([
      'opencode:opencode-go/deepseek-v4.1-flash',
      'opencode:opencode-go/gemini-4-argon',
      'opencode:opencode-go/glm-5.3',
      'opencode:opencode-go/kimi-k3',
      'opencode:other/gemini-4-argon',
    ])
    expect(model(result, 'opencode:opencode-go/glm-5.3')).toEqual({
      client: 'opencode',
      cost_class: 'subsidised',
      family: 'zhipu',
      inline_only: true,
      may_send: { code: false, history: false },
      model: 'opencode-go/glm-5.3',
      notes: '',
      source: 'live',
      tier: null,
    })
    expect(model(result, 'opencode:opencode-go/kimi-k3').family).toBe('moonshot')
    expect(w.ran).toEqual([{ args: ['models'], command: 'opencode' }])
  })

  it('takes the cost class the shipped defaults set for OpenCode Go, and the client cost class for another provider', async () => {
    const w = await workspace({ installed: ['opencode'], listings: { opencode: OPENCODE_LISTING } })

    const result = await w.run('models', '--json', '{}')

    expect(model(result, 'opencode:opencode-go/gemini-4-argon').cost_class).toBe('subsidised')
    expect(model(result, 'opencode:other/gemini-4-argon').cost_class).toBe('metered-personal')
  })

  it('takes the cost class the machine file sets for a provider over the shipped one', async () => {
    const w = await workspace({ installed: ['opencode'], listings: { opencode: OPENCODE_LISTING } })
    await w.writeOverride({ clients: { opencode: { provider_cost_classes: { 'opencode-go': 'metered-employer' } } } })

    const result = await w.run('models', '--json', '{}')

    expect(model(result, 'opencode:opencode-go/glm-5.3').cost_class).toBe('metered-employer')
  })

  it('reports a model whose family it cannot work out in skipped, with the reason', async () => {
    const w = await workspace({ installed: ['opencode'], listings: { opencode: OPENCODE_LISTING } })

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).not.toContain('opencode:opencode-go/hy3')
    expect(printed(result).skipped).toEqual([
      {
        client: 'opencode',
        model: 'opencode-go/hy3',
        reason: 'family unknown: add it to the catalogue in defaults.json or the machine override',
      },
    ])
  })

  it('leaves the client out with the reason when its listing cannot be run at all', async () => {
    const w = await workspace({ installed: ['opencode'] })

    const result = await w.run('models', '--json', '{}')

    expect(printed(result).missing).toContainEqual({ client: 'opencode', reason: 'listing failed: opencode models: cannot run opencode' })
  })

  it('leaves the client out with the reason when its listing fails, and still lists the others', async () => {
    const w = await workspace({
      installed: ['claude', 'opencode'],
      listings: { opencode: { exitCode: 2, stderr: '', stdout: '' } },
    })

    const result = await w.run('models', '--json', '{}')

    expect(printed(result).missing).toContainEqual({ client: 'opencode', reason: 'listing failed: opencode models exited 2' })
    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
  })
})

/** A Vibe journal: one assistant message whose text is `text`. */
function journal(text: string): string {
  return JSON.stringify([{ content: [{ text }], role: 'assistant', type: 'message' }])
}

/** What Vibe writes to `export.json`: its token counts, and its own figure when it priced the run. */
function exported(options: { cost?: number; cached?: number; input?: number; output?: number } = {}): unknown {
  return {
    ...(options.cost === undefined ? {} : { cost_usd: options.cost }),
    usage: {
      cached_input_tokens: options.cached ?? 0,
      input_tokens: options.input ?? 0,
      output_tokens: options.output ?? 0,
    },
  }
}

/** Critique's routing, as the shipped file holds it: the Vibe critic clients and the model each runs. */
const ROUTING = {
  clients: {
    'agy-gemini-flash-high': { kind: 'agy', model: 'gemini-3.8-flash-high' },
    'vibe-glm': { kind: 'vibe', model: 'zai-glm-5-3' },
    'vibe-mistral-large': { kind: 'vibe', model: 'mistral-large-4-0' },
  },
}

/** Setup's results, with the read-only proof of one critic client recorded as `result`. */
function setupResults(result: string, client = 'vibe-glm'): unknown {
  return { checks: { [`read-only:${client}`]: { result } }, version: 1 }
}

interface RunWorkspace {
  readonly dir: string
  readonly home: string
  readonly state: string
  /** The repository the run works in: a real directory, so its run directory can be written. */
  readonly repo: string
  readonly vibe: FakeVibeProcess
  readonly ledger: string
  readonly workflows: string
  /** Any gateway command, for the tests that are not about `run`. */
  call(...argv: string[]): Promise<ModelGatewayCliResult>
  /** The input a run test sends unless it says otherwise: one read-only task in the test repository. */
  input(overrides?: Readonly<Record<string, unknown>>): unknown
  run(input: unknown): Promise<ModelGatewayCliResult>
  writeAllowlist(value: unknown): Promise<void>
  writeOverride(value: unknown): Promise<void>
  writeRoutingOverride(value: unknown): Promise<void>
  writeSetup(value: unknown): Promise<void>
  writeWorkflows(value: unknown): Promise<void>
}

interface RunWorkspaceOptions {
  readonly scripts?: readonly VibeScript[]
  /** The client the gateway launches, when a test runs something other than Vibe. */
  readonly process?: FakeHermesProcess
  /** The repositories the fake git knows, keyed by path; the test repository by default. */
  readonly repositories?: (repo: string) => Readonly<Record<string, FakeRepository>>
  readonly env?: Readonly<Record<string, string>>
  readonly routing?: unknown
  readonly setupResults?: unknown
  readonly allowlist?: unknown
  readonly override?: unknown
  /** The shipped defaults the gateway reads, when a test needs a broken catalogue. */
  readonly defaults?: unknown
}

/** A gateway wired to a fake Vibe, a temporary home and state directory, and a routing file of its own. */
async function runWorkspace(options: RunWorkspaceOptions = {}): Promise<RunWorkspace> {
  const dir = await mkdtemp(join(tmpdir(), 'model-gateway-run-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const home = join(dir, 'home')
  const state = join(dir, 'state')
  const repo = join(dir, 'repo')
  await mkdir(repo, { recursive: true })
  const defaults = join(dir, 'defaults.json')
  await writeFile(
    defaults,
    options.defaults === undefined ? await readFile(SHIPPED_DEFAULTS, 'utf8') : JSON.stringify(options.defaults),
  )
  const routingPath = join(dir, 'routing.json')
  await writeFile(routingPath, JSON.stringify(options.routing ?? ROUTING))
  const vibe = new FakeVibeProcess([...(options.scripts ?? [])])
  const repositories = options.repositories?.(repo) ?? {
    [repo]: { commonDir: join(repo, '.git'), remotes: [AGENT_KIT_REMOTE], root: repo },
  }
  const runtime = defaultModelGatewayRuntime({
    defaultsPath: defaults,
    env: new FakeEnv({ HOME: home, XDG_STATE_HOME: state, ...options.env }),
    git: new FakeGit(repositories),
    process: options.process ?? vibe,
    routingPath,
  })
  const write = async (path: string, value: unknown): Promise<void> => {
    await mkdir(dirname(path), { recursive: true })
    await writeFile(path, JSON.stringify(value))
  }
  const w: RunWorkspace = {
    dir,
    home,
    input: (overrides = {}) => ({
      client: 'vibe',
      mode: 'read-only',
      model: 'glm-5-3',
      prompt: 'read this',
      repo,
      workflow: 'spec-293',
      ...overrides,
    }),
    call: (...argv) => runModelGatewayCli(argv, runtime),
    ledger: join(state, 'agent-kit', 'model-gateway', 'ledger.jsonl'),
    repo,
    run: (input) => runModelGatewayCli(['run', '--json', JSON.stringify(input)], runtime),
    state,
    vibe,
    workflows: join(state, 'agent-kit', 'model-gateway', 'workflows.json'),
    writeAllowlist: (value) => write(join(home, '.config', 'agent-kit', 'critique-allowlist.json'), value),
    writeOverride: (value) => write(join(home, '.config', 'agent-kit', 'model-gateway.json'), value),
    writeRoutingOverride: (value) => write(join(home, '.config', 'agent-kit', 'critique-routing.json'), value),
    writeSetup: (value) => write(join(home, '.local', 'share', 'agent-kit', 'critique', 'machine-setup.json'), value),
    writeWorkflows: (value) => write(join(state, 'agent-kit', 'model-gateway', 'workflows.json'), value),
  }
  if (options.allowlist !== undefined) await w.writeAllowlist(options.allowlist)
  if (options.override !== undefined) await w.writeOverride(options.override)
  if (options.setupResults !== undefined) await w.writeSetup(options.setupResults)
  return w
}

/** A workspace whose one run is permitted, proven and scripted to answer `text`. */
async function readyRun(options: RunWorkspaceOptions & { readonly text?: string } = {}): Promise<RunWorkspace> {
  return runWorkspace({
    allowlist: allowlist(['zhipu'], true, true),
    scripts: [{ exportJson: exported({ cost: 0.0123, input: 100 }), stdout: journal(options.text ?? 'the answer\nSUMMARY: it is fine') }],
    setupResults: setupResults('pass'),
    ...options,
  })
}

/** The run directory of a result, inside the test repository's common directory. */
function runDirOf(w: RunWorkspace, result: ModelGatewayCliResult): string {
  return join(w.repo, '.git', 'model-gateway', 'runs', String(printed(result).run_id))
}

/** The value of one flag of a launch. */
function flag(command: ProcessCommand | undefined, name: string): string | undefined {
  const at = command?.args.indexOf(name) ?? -1
  return at === -1 ? undefined : command?.args[at + 1]
}

describe('model-gateway run: the input', () => {
  it('refuses a call that names no workflow, before anything is launched', async () => {
    const w = await readyRun()

    const result = await w.run(w.input({ workflow: undefined }))

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toEqual({ code: 'invalid-input', message: 'a run must name its workflow', status: 'error' })
    expect(w.vibe.commands).toEqual([])
  })

  it('refuses a client or a mode a later ticket builds', async () => {
    const w = await readyRun()

    expect(printed(await w.run(w.input({ client: 'codex' }))).code).toBe('not-supported-yet')
    expect(printed(await w.run(w.input({ mode: 'writing' }))).code).toBe('not-supported-yet')
    expect(w.vibe.commands).toEqual([])
  })

  it('refuses a client the configuration does not declare', async () => {
    const shipped = JSON.parse(await readFile(SHIPPED_DEFAULTS, 'utf8')) as { clients: Record<string, unknown> }
    delete shipped.clients.vibe
    const w = await readyRun({ defaults: shipped })

    expect(printed(await w.run(w.input()))).toEqual({
      code: 'unknown-model',
      message: 'the gateway knows no client vibe',
      status: 'error',
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('refuses a model the gateway cannot place, and one the machine disabled', async () => {
    const w = await readyRun({ override: { disabled: ['vibe:mistral-large-4-0'] } })

    expect(printed(await w.run(w.input({ model: 'unknown-model-1' })))).toMatchObject({
      code: 'unknown-model',
      message: 'vibe:unknown-model-1 is not a model the gateway can place: add it to the catalogue in defaults.json or the machine override',
    })
    expect(printed(await w.run(w.input({ model: 'mistral-large-4-0' })))).toMatchObject({
      code: 'unknown-model',
      message: "vibe:mistral-large-4-0 is disabled in this machine's configuration",
    })
    expect(w.vibe.commands).toEqual([])
  })
})

describe('model-gateway run: the data policy, before anything is launched', () => {
  it('refuses a client model the repository’s entry does not permit, naming the rule', async () => {
    const w = await readyRun({ allowlist: allowlist(['mistral'], true, true) })

    expect(printed(await w.run(w.input()))).toEqual({
      code: 'policy-refused',
      message: `vibe:glm-5-3 is not permitted: ${KEY}: the entry allows mistral`,
      status: 'error',
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('refuses a run in a repository whose entry does not allow code, whatever the caller marks', async () => {
    const w = await readyRun({ allowlist: allowlist(['zhipu'], false, true) })

    expect(printed(await w.run(w.input()))).toMatchObject({
      code: 'policy-refused',
      message: `vibe:glm-5-3 may not carry code: ${KEY}: the entry allows zhipu`,
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('refuses history the entry does not allow, and runs when it does', async () => {
    const refused = await readyRun({ allowlist: allowlist(['zhipu'], true, false) })

    expect(printed(await refused.run(refused.input({ carries: { code: true, history: true } })))).toMatchObject({
      code: 'policy-refused',
      message: `vibe:glm-5-3 may not carry history: ${KEY}: the entry allows zhipu`,
    })
    expect(refused.vibe.commands).toEqual([])

    const allowed = await readyRun({ allowlist: allowlist(['zhipu'], true, true) })
    expect(printed(await allowed.run(allowed.input({ carries: { code: true, history: true } }))).status).toBe('ok')
  })

  it('refuses an inline run that claims code, and runs one that claims nothing', async () => {
    const w = await readyRun()

    expect(printed(await w.run(w.input({ carries: { code: true, history: false }, repo: undefined })))).toMatchObject({
      code: 'policy-refused',
      message: 'vibe:glm-5-3 may not carry code: no repository: only inline content, to subsidised and metered-personal clients',
    })
    expect(w.vibe.commands).toEqual([])

    expect(printed(await w.run(w.input({ repo: undefined }))).status).toBe('ok')
  })

  it('refuses a nedap repository, which may reach only Codex and Claude Code', async () => {
    const w = await readyRun({
      repositories: (repo) => ({
        [repo]: { commonDir: join(repo, '.git'), remotes: [{ name: 'origin', url: 'git@github.com:nedap/agent-kit.git' }], root: repo },
      }),
    })

    expect(printed(await w.run(w.input()))).toMatchObject({
      code: 'policy-refused',
      message: 'vibe:glm-5-3 is not permitted: a nedap repository: only Codex and Claude Code, with code and history',
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('refuses a path in no repository at all, rather than running inline', async () => {
    const w = await readyRun({ repositories: () => ({}) })

    expect(printed(await w.run(w.input()))).toEqual({
      code: 'repository-unreadable',
      message: `${w.repo}: not a git repository`,
      status: 'error',
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('refuses a path git cannot read, naming the path and git’s message', async () => {
    const w = await readyRun({ repositories: (repo) => ({ [repo]: { error: 'fatal: detected dubious ownership' } }) })

    expect(printed(await w.run(w.input()))).toEqual({
      code: 'repository-unreadable',
      message: `${w.repo}: fatal: detected dubious ownership`,
      status: 'error',
    })
    expect(w.vibe.commands).toEqual([])
  })
})

describe('model-gateway run: the read-only proof', () => {
  it('refuses a model with no passing proof, and launches nothing', async () => {
    const w = await readyRun({ setupResults: setupResults('fail') })

    expect(printed(await w.run(w.input()))).toEqual({
      code: 'unproven-read-only',
      message: "vibe:glm-5-3 has no passing read-only proof: setup's read-only proof for vibe-glm is fail",
      status: 'error',
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('refuses when setup recorded no proof at all', async () => {
    const w = await readyRun({ setupResults: undefined })

    expect(printed(await w.run(w.input()))).toMatchObject({
      code: 'unproven-read-only',
      message: "vibe:glm-5-3 has no passing read-only proof: setup's read-only proof for vibe-glm is not recorded",
    })
  })

  it('refuses a model no Vibe critic client runs, even when another model’s proof passed', async () => {
    const w = await readyRun({
      allowlist: allowlist(['zhipu', 'mistral'], true, true),
      routing: { clients: { 'vibe-glm': { kind: 'vibe', model: 'zai-glm-5-3' } } },
    })

    expect(printed(await w.run(w.input({ model: 'mistral-large-4-0' })))).toMatchObject({
      code: 'unproven-read-only',
      message: "vibe:mistral-large-4-0 has no passing read-only proof: no Vibe critic client in critique's routing runs mistral-large-4-0",
    })
  })

  it('follows this machine’s routing override, which may move a critic client to another model', async () => {
    const w = await readyRun()
    await w.writeRoutingOverride({ clients: { 'vibe-glm': { model: 'zai-glm-6-0' } } })

    expect(printed(await w.run(w.input()))).toMatchObject({ code: 'unproven-read-only' })
  })

  it('proves the alias a run names by the model id the routing names, and launches it', async () => {
    const w = await readyRun()

    const result = await w.run(w.input())

    expect(printed(result).status).toBe('ok')
    expect(flag(w.vibe.commands[0], '--model')).toBe('glm-5-3')
  })

  it('refuses the alias when the routing runs no client on any of its names', async () => {
    const w = await readyRun({ routing: { clients: { 'vibe-glm': { kind: 'vibe', model: 'zai-glm-6-0' } } } })

    expect(printed(await w.run(w.input()))).toEqual({
      code: 'unproven-read-only',
      message: "vibe:glm-5-3 has no passing read-only proof: no Vibe critic client in critique's routing runs glm-5-3",
      status: 'error',
    })
    expect(w.vibe.commands).toEqual([])
  })
})

describe('model-gateway run: the launch', () => {
  it('launches Vibe non-interactively, with the model, the read-only agent and its own limits', async () => {
    const w = await readyRun()

    const result = await w.run(w.input({ max_price_usd: 0.5, time_limit_s: 120 }))

    expect(result.exitCode).toBe(0)
    const [command] = w.vibe.commands
    expect(command?.command).toBe('vibe')
    expect(command?.cwd).toBe(w.repo)
    expect(command?.args).toEqual([
      '--prompt-file',
      join(runDirOf(w, result), 'prompt.md'),
      '--model',
      'glm-5-3',
      '--agent',
      'delegate-readonly',
      '--trust',
      '--workdir',
      w.repo,
      '--output',
      'json',
      '--time-limit',
      '120',
      '--max-price',
      '0.5',
      '--output-dir',
      join(runDirOf(w, result), 'vibe'),
    ])
    expect(command?.timeoutMs).toBe(180_000)
  })

  it('leaves the price cap out when the caller sets none, and defaults the time limit to 900 seconds', async () => {
    const w = await readyRun()

    const result = await w.run(w.input())

    expect(flag(w.vibe.commands[0], '--max-price')).toBeUndefined()
    expect(flag(w.vibe.commands[0], '--time-limit')).toBe('900')
    expect(result.exitCode).toBe(0)
  })

  it('marks the delegate’s environment, and switches the home’s MCP servers and subagents off', async () => {
    const w = await readyRun()

    await w.run(w.input())

    expect(w.vibe.commands[0]?.env).toEqual({
      AGENT_KIT_DELEGATION: '1',
      VIBE_ENABLE_MCP: 'false',
      VIBE_ENABLE_SUBAGENTS: 'false',
      VIBE_INCLUDE_PROJECT_CONTEXT: 'false',
      VIBE_OUTSIDE_WORKDIR: 'deny',
    })
  })

  it('gives an inline run a fresh empty directory outside every repository as its workdir', async () => {
    const w = await readyRun()

    const result = await w.run(w.input({ repo: undefined }))

    const workDir = flag(w.vibe.commands[0], '--workdir') ?? ''
    expect(workDir.startsWith(tmpdir())).toBe(true)
    expect(workDir.startsWith(w.repo)).toBe(false)
    expect(await readdir(workDir)).toEqual([])
    expect(printed(result).status).toBe('ok')
  })
})

describe('model-gateway run: the result', () => {
  it('prints the short result: status, run id, client model, tokens, cost, output path and summary', async () => {
    const w = await readyRun({
      scripts: [{ exportJson: exported({ cached: 10, cost: 0.0123, input: 100, output: 20 }), stdout: journal('the answer\nSUMMARY: it is fine') }],
    })

    const result = await w.run(w.input())

    expect(result.exitCode).toBe(0)
    expect(result.stderr).toBe('')
    expect(printed(result)).toEqual({
      client: 'vibe',
      cost_usd: 0.0123,
      family: 'zhipu',
      mode: 'read-only',
      model: 'glm-5-3',
      output_path: join(runDirOf(w, result), 'output.json'),
      run_id: printed(result).run_id,
      status: 'ok',
      summary: 'it is fine',
      tokens: { cached: 10, input: 100, output: 20 },
      workflow: 'spec-293',
    })
    expect(String(printed(result).run_id)).toMatch(/^\d{8}T\d{6}Z-[0-9a-f]{6}$/)
  })

  it('cuts the summary at summary_max_chars, and falls back to the last characters of the answer', async () => {
    const cut = await readyRun({ override: { settings: { summary_max_chars: 5 } }, text: 'SUMMARY: 0123456789' })
    expect(printed(await cut.run(cut.input())).summary).toBe('01234')

    const fallback = await readyRun({ override: { settings: { summary_max_chars: 5 } }, text: '0123456789' })
    expect(printed(await fallback.run(fallback.input())).summary).toBe('56789')
  })

  it('counts the dollars at the catalogue price when the export does not parse', async () => {
    const w = await readyRun({
      scripts: [
        {
          effect: async (command) => {
            const dir = String(flag(command, '--output-dir'))
            await mkdir(dir, { recursive: true })
            await writeFile(join(dir, 'export.json'), '{"cost_usd":')
          },
          stdout: journal('done'),
        },
      ],
    })

    const result = await w.run(w.input())

    expect(printed(result)).toMatchObject({ cost_usd: 0, status: 'ok', tokens: { cached: 0, input: 0, output: 0 } })
  })

  it('counts the dollars at the catalogue price when Vibe priced nothing', async () => {
    const w = await readyRun({ scripts: [{ exportJson: exported({ input: 1_000_000, output: 1_000_000 }), stdout: journal('done') }] })

    expect(printed(await w.run(w.input())).cost_usd).toBeCloseTo(1.4 + 4.4, 10)
  })
})

describe('model-gateway run: the run directory', () => {
  it('keeps the prompt, the input, the full output and the result in the repository’s common directory', async () => {
    const w = await readyRun()

    const result = await w.run(w.input())

    const runDir = runDirOf(w, result)
    expect(await readFile(join(runDir, 'prompt.md'), 'utf8')).toBe(
      'read this\n\nEnd your answer with a line "SUMMARY:" followed by at most 1500 characters that summarise the result.',
    )
    expect(JSON.parse(await readFile(join(runDir, 'input.json'), 'utf8'))).toEqual({ ...(w.input() as object), time_limit_s: 900 })
    expect(await readFile(join(runDir, 'output.json'), 'utf8')).toBe(journal('the answer\nSUMMARY: it is fine'))
    expect(JSON.parse(await readFile(join(runDir, 'result.json'), 'utf8'))).toEqual(printed(result))
  })

  it('refuses a repository whose common directory git cannot name', async () => {
    const w = await readyRun({
      repositories: (repo) => ({
        [repo]: { commonDirError: 'fatal: not a git repository', remotes: [AGENT_KIT_REMOTE], root: repo },
      }),
    })

    expect(printed(await w.run(w.input()))).toEqual({
      code: 'repository-unreadable',
      message: `${w.repo}: fatal: not a git repository`,
      status: 'error',
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('keeps them in this machine’s state directory for an inline run', async () => {
    const w = await readyRun()

    const result = await w.run(w.input({ repo: undefined }))

    const runDir = join(w.state, 'agent-kit', 'model-gateway', 'runs', String(printed(result).run_id))
    expect(printed(result).output_path).toBe(join(runDir, 'output.json'))
    expect(await readFile(join(runDir, 'prompt.md'), 'utf8')).toContain('read this')
  })
})

describe('model-gateway run: the ledger', () => {
  it('appends one line per run: what it was, what it cost and how it ended', async () => {
    const w = await readyRun()

    const result = await w.run(w.input())

    const lines = (await readFile(w.ledger, 'utf8')).trimEnd().split('\n')
    expect(lines).toHaveLength(1)
    expect(JSON.parse(String(lines[0]))).toEqual({
      at: expect.any(String) as unknown,
      client: 'vibe',
      cost_usd: 0.0123,
      duration_s: expect.any(Number) as unknown,
      family: 'zhipu',
      mode: 'read-only',
      model: 'glm-5-3',
      outcome: 'ok',
      repository: KEY,
      run_id: printed(result).run_id,
      tokens: { cached: 0, input: 100, output: 0 },
      workflow: 'spec-293',
    })
  })

  it('appends a line for a failed run too, with its reason and no repository for an inline one', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 3, stdout: '' }] })

    await w.run(w.input({ repo: undefined }))

    expect(JSON.parse((await readFile(w.ledger, 'utf8')).trimEnd())).toMatchObject({
      outcome: 'failed',
      reason: 'exit-3',
      repository: null,
    })
  })

  it('never interleaves a line when two runs append at once', async () => {
    const w = await readyRun()
    w.vibe.queue(...Array.from({ length: 7 }, () => ({ exportJson: exported({ cost: 0.01 }), stdout: journal('done') })))

    const results = await Promise.all(Array.from({ length: 8 }, () => w.run(w.input())))

    const lines = (await readFile(w.ledger, 'utf8')).trimEnd().split('\n')
    expect(lines).toHaveLength(8)
    const ids = lines.map((line) => (JSON.parse(line) as { run_id: string }).run_id)
    expect(new Set(ids).size).toBe(8)
    expect(ids.sort()).toEqual(results.map((result) => String(printed(result).run_id)).sort())
    expect(await readdir(dirname(w.ledger))).toEqual(['ledger.jsonl', 'workflows.json'])
  })
})

describe('model-gateway run: a failed run', () => {
  it('reports a timeout', async () => {
    const w = await readyRun({ scripts: [{ stdout: '', timedOut: true }] })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'timeout', status: 'failed' })
  })

  it('reports the exit code Vibe ended with', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 3, stderr: 'stopped early', stdout: '' }] })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'exit-3', status: 'failed' })
  })

  it('reports a journal it cannot read, and keeps the raw output', async () => {
    const w = await readyRun({ scripts: [{ stdout: 'not a journal' }] })

    const result = await w.run(w.input())

    expect(printed(result)).toMatchObject({ reason: 'parse-error', status: 'failed', summary: '' })
    expect(await readFile(join(runDirOf(w, result), 'output.json'), 'utf8')).toBe('not a journal')
  })

  it('reports a launch that cannot start at all', async () => {
    const w = await readyRun({ scripts: [] })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'launch-failed', status: 'failed' })
  })
})

describe('model-gateway run: the nesting guard', () => {
  it('refuses a call from inside a delegation', async () => {
    const w = await readyRun({ env: { AGENT_KIT_DELEGATION: '1' } })

    expect(printed(await w.run(w.input()))).toEqual({
      code: 'nested-delegation',
      message: 'a delegate may not delegate on: AGENT_KIT_DELEGATION is set in this environment',
      status: 'error',
    })
    expect(w.vibe.commands).toEqual([])
  })
})

/** What `hermes config get model --json` prints: the default model and the provider it runs on. */
const HERMES_CONFIG: ProcessResult = {
  exitCode: 0,
  stderr: '',
  stdout: '{"default":"anthropic/claude-sonnet-4.6","provider":"anthropic"}\n',
}

describe('models: Hermes, read from its own configuration', () => {
  it('lists the model its configuration names as its default, marked as read from its configuration', async () => {
    const w = await workspace({ installed: ['hermes'], listings: { hermes: HERMES_CONFIG } })

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual(['hermes:anthropic/claude-sonnet-4.6'])
    expect(model(result, 'hermes:anthropic/claude-sonnet-4.6')).toEqual({
      client: 'hermes',
      cost_class: 'metered-personal',
      family: 'anthropic',
      inline_only: true,
      may_send: { code: false, history: false },
      model: 'anthropic/claude-sonnet-4.6',
      notes: '',
      source: 'config',
      tier: null,
    })
    expect(w.ran).toEqual([{ args: ['config', 'get', 'model', '--json'], command: 'hermes' }])
  })

  it('leaves the client out with the reason when its configuration names no model', async () => {
    const w = await workspace({
      installed: ['hermes'],
      listings: { hermes: { exitCode: 0, stderr: '', stdout: '{}\n' } },
    })

    const result = await w.run('models', '--json', '{}')

    expect(printed(result).missing).toContainEqual({
      client: 'hermes',
      reason: 'listing failed: hermes config get model --json named no model',
    })
  })

  it('leaves the client out with the reason when its configuration cannot be read', async () => {
    const w = await workspace({
      installed: ['hermes'],
      listings: { hermes: { exitCode: 1, stderr: 'Config key not set: model\n', stdout: '' } },
    })

    const result = await w.run('models', '--json', '{}')

    expect(printed(result).missing).toContainEqual({
      client: 'hermes',
      reason: 'listing failed: hermes config get model --json exited 1: Config key not set: model',
    })
  })
})

describe('model-gateway workflow', () => {
  it('starts a workflow and reports when it started', async () => {
    const w = await runWorkspace()

    const result = await w.call('workflow', 'start', '--json', '{"name":"spec-293"}')

    expect(result.exitCode).toBe(0)
    expect(result.stderr).toBe('')
    expect(printed(result)).toEqual({
      status: 'ok',
      workflow: { ended_at: null, name: 'spec-293', started_at: expect.any(String) as unknown, state: 'running' },
    })
    expect(JSON.parse(await readFile(w.workflows, 'utf8'))).toEqual({
      version: 1,
      workflows: [{ name: 'spec-293', started_at: printed(result).workflow instanceof Object ? (printed(result).workflow as { started_at: string }).started_at : '' }],
    })
  })

  it('leaves a workflow that is already running as it is', async () => {
    const w = await runWorkspace()
    await w.writeWorkflows({ version: 1, workflows: [{ name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }] })

    const result = await w.call('workflow', 'start', '--json', '{"name":"spec-293"}')

    expect(printed(result)).toMatchObject({ workflow: { started_at: '2026-11-04T10:00:00.000Z', state: 'running' } })
  })

  it('ends a workflow and reports when it ended', async () => {
    const w = await runWorkspace()
    await w.writeWorkflows({ version: 1, workflows: [{ name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }] })

    const result = await w.call('workflow', 'end', '--json', '{"name":"spec-293"}')

    expect(printed(result)).toEqual({
      status: 'ok',
      workflow: {
        ended_at: expect.any(String) as unknown,
        name: 'spec-293',
        started_at: '2026-11-04T10:00:00.000Z',
        state: 'ended',
      },
    })
  })

  it('lists every workflow, with the 24-hour end worked out', async () => {
    const w = await runWorkspace()
    await w.writeWorkflows({
      version: 1,
      workflows: [
        { name: 'abandoned', started_at: '2020-01-01T00:00:00.000Z' },
        { ended_at: '2026-11-04T11:00:00.000Z', name: 'done', started_at: '2026-11-04T10:00:00.000Z' },
      ],
    })

    const result = await w.call('workflow', 'list', '--json', '{}')

    expect(printed(result)).toEqual({
      status: 'ok',
      workflows: [
        { ended_at: '2020-01-02T00:00:00.000Z', name: 'abandoned', started_at: '2020-01-01T00:00:00.000Z', state: 'ended' },
        { ended_at: '2026-11-04T11:00:00.000Z', name: 'done', started_at: '2026-11-04T10:00:00.000Z', state: 'ended' },
      ],
    })
  })

  it('lists no workflow before one has started', async () => {
    const w = await runWorkspace()

    expect(printed(await w.call('workflow', 'list', '--json', '{}'))).toEqual({ status: 'ok', workflows: [] })
  })

  it('refuses a workflow command that names no workflow', async () => {
    const w = await runWorkspace()

    expect(printed(await w.call('workflow', 'start', '--json', '{}'))).toEqual({
      code: 'invalid-input',
      message: 'a workflow command must name its workflow',
      status: 'error',
    })
    expect(printed(await w.call('workflow', 'end', '--json', '{"name":""}'))).toMatchObject({ code: 'invalid-input' })
  })

  it('refuses to end a workflow this machine never started', async () => {
    const w = await runWorkspace()

    expect(printed(await w.call('workflow', 'end', '--json', '{"name":"nowhere"}'))).toEqual({
      code: 'unknown-workflow',
      message: 'no workflow named nowhere has started on this machine',
      status: 'error',
    })
  })

  it('refuses a workflow file it cannot read, naming the file', async () => {
    const w = await runWorkspace()
    await w.writeWorkflows({ version: 2, workflows: [] })

    const result = await w.call('workflow', 'list', '--json', '{}')

    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.workflows)
  })

  it('refuses a workflow subcommand it does not have', async () => {
    const w = await runWorkspace()

    expect(printed(await w.call('workflow', 'pause', '--json', '{}'))).toMatchObject({ code: 'unknown-command' })
  })
})

describe('model-gateway run: the workflow', () => {
  it('starts the workflow the first time a run names it', async () => {
    const w = await readyRun()

    await w.run(w.input())

    expect(JSON.parse(await readFile(w.workflows, 'utf8'))).toMatchObject({
      workflows: [{ name: 'spec-293', started_at: expect.any(String) as unknown }],
    })
  })

  it('refuses a run that names a workflow that ended, and launches nothing', async () => {
    const w = await readyRun()
    await w.writeWorkflows({
      version: 1,
      workflows: [{ ended_at: '2026-11-04T11:00:00.000Z', name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }],
    })

    expect(printed(await w.run(w.input()))).toEqual({
      code: 'workflow-ended',
      message: 'workflow spec-293 ended at 2026-11-04T11:00:00.000Z: a run may not join it, so start a new workflow',
      status: 'error',
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('refuses a run that names a workflow whose 24 hours ran out', async () => {
    const w = await readyRun()
    await w.writeWorkflows({ version: 1, workflows: [{ name: 'spec-293', started_at: '2020-01-01T00:00:00.000Z' }] })

    expect(printed(await w.run(w.input()))).toMatchObject({
      code: 'workflow-ended',
      message: 'workflow spec-293 ended at 2020-01-02T00:00:00.000Z: a run may not join it, so start a new workflow',
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('starts no workflow for a run refused for a client the configuration does not declare', async () => {
    const shipped = JSON.parse(await readFile(SHIPPED_DEFAULTS, 'utf8')) as { clients: Record<string, unknown> }
    delete shipped.clients.vibe
    const w = await readyRun({ defaults: shipped })

    expect(printed(await w.run(w.input()))).toMatchObject({ code: 'unknown-model' })
    expect(await listedWorkflows(w)).toEqual([])
  })

  it('starts no workflow for a run the data policy refuses', async () => {
    const w = await readyRun({ allowlist: allowlist(['mistral'], true, true) })

    expect(printed(await w.run(w.input()))).toMatchObject({ code: 'policy-refused' })
    expect(await listedWorkflows(w)).toEqual([])
  })

  it('starts no workflow for a run whose provider this machine does not allow', async () => {
    const { w } = await readyHermes({ override: {} })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({ code: 'provider-not-allowed' })
    expect(await listedWorkflows(w)).toEqual([])
  })

  it('starts no workflow for a run with no passing read-only proof', async () => {
    const { w } = await readyHermes({ setupResults: undefined })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({ code: 'unproven-read-only' })
    expect(await listedWorkflows(w)).toEqual([])
  })
})

/** The workflows `workflow list` reports, as the gateway itself reads them. */
async function listedWorkflows(w: RunWorkspace): Promise<readonly unknown[]> {
  return (printed(await w.call('workflow', 'list', '--json', '{}')) as { workflows: readonly unknown[] }).workflows
}

/** The machine file that lets Hermes run on Anthropic. */
const HERMES_MACHINE = {
  clients: { hermes: { providers: ['anthropic'] } },
}

/** What Hermes writes to its usage file: its own tokens, dollars and outcome. */
function hermesUsageFile(options: { cost?: number; failed?: boolean; input?: number; output?: number } = {}): unknown {
  return {
    api_calls: 2,
    cache_read_tokens: 0,
    estimated_cost_usd: options.cost ?? 0.0234,
    failed: options.failed ?? false,
    input_tokens: options.input ?? 1200,
    model: 'anthropic/claude-sonnet-4.6',
    output_tokens: options.output ?? 300,
    provider: 'anthropic',
  }
}

interface HermesWorkspace {
  readonly hermes: FakeHermesProcess
  readonly w: RunWorkspace
}

/** A Hermes run's input: the metered-personal client, on the provider the machine allows. */
function hermesInput(w: RunWorkspace, overrides: Readonly<Record<string, unknown>> = {}): unknown {
  return w.input({ client: 'hermes', model: 'anthropic/claude-sonnet-4.6', provider: 'anthropic', ...overrides })
}

/** A workspace whose one Hermes run is permitted, proven and scripted to answer `text`. */
async function readyHermes(
  options: RunWorkspaceOptions & { readonly text?: string; readonly usage?: unknown } = {},
): Promise<HermesWorkspace> {
  const hermes = new FakeHermesProcess([
    { stdout: options.text ?? 'the answer\nSUMMARY: it is fine', usage: options.usage ?? hermesUsageFile() },
  ])
  const w = await runWorkspace({
    allowlist: allowlist(['anthropic'], true, true),
    override: HERMES_MACHINE,
    process: hermes,
    setupResults: setupResults('pass', 'hermes'),
    ...options,
  })
  return { hermes, w }
}

describe('model-gateway run: Hermes, the metered-personal client', () => {
  it('refuses a hermes run that names no provider, naming the providers this machine allows', async () => {
    const { hermes, w } = await readyHermes()

    expect(printed(await w.run(hermesInput(w, { provider: undefined })))).toEqual({
      code: 'provider-not-allowed',
      message: 'a hermes run must name its provider: this machine allows anthropic',
      status: 'error',
    })
    expect(hermes.commands).toEqual([])
  })

  it('refuses a provider this machine does not list for hermes', async () => {
    const { hermes, w } = await readyHermes()

    expect(printed(await w.run(hermesInput(w, { provider: 'google' })))).toEqual({
      code: 'provider-not-allowed',
      message: 'hermes may not run on google: this machine allows anthropic',
      status: 'error',
    })
    expect(hermes.commands).toEqual([])
  })

  it('refuses a hermes run when the machine lists no provider at all', async () => {
    const { w } = await readyHermes({ override: {} })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({
      code: 'provider-not-allowed',
      message: 'hermes may not run on anthropic: this machine allows no provider',
    })
  })

  it('refuses openrouter, whatever the machine lists', async () => {
    const { hermes, w } = await readyHermes({ override: { clients: { hermes: { providers: ['openrouter'] } } } })

    expect(printed(await w.run(hermesInput(w, { provider: 'openrouter' })))).toEqual({
      code: 'provider-not-allowed',
      message: 'openrouter is not allowed: the owner forbade OpenRouter',
      status: 'error',
    })
    expect(printed(await w.run(hermesInput(w, { model: 'openrouter/pareto-code' })))).toMatchObject({
      code: 'provider-not-allowed',
    })
    expect(hermes.commands).toEqual([])
  })

  it('refuses a hermes run setup recorded no read-only proof for, and launches nothing', async () => {
    const { hermes, w } = await readyHermes({ setupResults: undefined })

    expect(printed(await w.run(hermesInput(w)))).toEqual({
      code: 'unproven-read-only',
      message:
        "hermes:anthropic/claude-sonnet-4.6 has no passing read-only proof: setup's read-only proof for hermes is not recorded",
      status: 'error',
    })
    expect(hermes.commands).toEqual([])
  })

  it('launches hermes one-shot when setup proves its read-only runs', async () => {
    const { hermes, w } = await readyHermes()

    const result = await w.run(hermesInput(w, { time_limit_s: 120 }))

    expect(result.exitCode).toBe(0)
    const [command] = hermes.commands
    expect(command?.command).toBe('hermes')
    expect(command?.cwd).toBe(w.repo)
    expect(command?.args).toEqual([
      '-z',
      'read this\n\nEnd your answer with a line "SUMMARY:" followed by at most 1500 characters that summarise the result.',
      '--usage-file',
      join(runDirOf(w, result), 'usage.json'),
      '-m',
      'anthropic/claude-sonnet-4.6',
      '--provider',
      'anthropic',
    ])
    expect(command?.env).toEqual({ AGENT_KIT_DELEGATION: '1' })
    expect(command?.timeoutMs).toBe(180_000)
  })

  it('prints the short result, with the spend the usage file reports', async () => {
    const { w } = await readyHermes()

    const result = await w.run(hermesInput(w))

    expect(printed(result)).toEqual({
      client: 'hermes',
      cost_usd: 0.0234,
      family: 'anthropic',
      mode: 'read-only',
      model: 'anthropic/claude-sonnet-4.6',
      output_path: join(runDirOf(w, result), 'output.json'),
      run_id: printed(result).run_id,
      status: 'ok',
      summary: 'it is fine',
      tokens: { cached: 0, input: 1200, output: 300 },
      workflow: 'spec-293',
    })
  })

  it('records the spend the usage file reports in the ledger, under its workflow', async () => {
    const { w } = await readyHermes()

    const result = await w.run(hermesInput(w))

    const lines = (await readFile(w.ledger, 'utf8')).trimEnd().split('\n')
    expect(lines).toHaveLength(1)
    expect(JSON.parse(String(lines[0]))).toEqual({
      at: expect.any(String) as unknown,
      client: 'hermes',
      cost_usd: 0.0234,
      duration_s: expect.any(Number) as unknown,
      family: 'anthropic',
      mode: 'read-only',
      model: 'anthropic/claude-sonnet-4.6',
      outcome: 'ok',
      repository: KEY,
      run_id: printed(result).run_id,
      tokens: { cached: 0, input: 1200, output: 300 },
      workflow: 'spec-293',
    })
  })

  it('counts a run the usage file calls failed as failed, and keeps its spend', async () => {
    const { w } = await readyHermes({ usage: hermesUsageFile({ cost: 0.5, failed: true }) })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({ cost_usd: 0.5, reason: 'failed', status: 'failed' })
  })

  it('counts no dollars when the usage file prices nothing', async () => {
    const { w } = await readyHermes({ usage: { failed: false, input_tokens: 10, output_tokens: 5 } })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({ cost_usd: 0, status: 'ok' })
  })

  it('reports a hermes run that printed nothing as a parse error', async () => {
    const { w } = await readyHermes({ text: '  \n' })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({ reason: 'parse-error', status: 'failed', summary: '' })
  })

  it('reports the exit code hermes ended with', async () => {
    const hermes = new FakeHermesProcess([{ exitCode: 2, stderr: 'no final response', stdout: '' }])
    const w = await runWorkspace({
      allowlist: allowlist(['anthropic'], true, true),
      override: HERMES_MACHINE,
      process: hermes,
      setupResults: setupResults('pass', 'hermes'),
    })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({
      reason: 'exit-2',
      status: 'failed',
    })
  })

  it('refuses a hermes run the data policy does not permit, before anything is launched', async () => {
    const { hermes, w } = await readyHermes({ allowlist: allowlist(['google'], true, true) })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({
      code: 'policy-refused',
      message: `hermes:anthropic/claude-sonnet-4.6 is not permitted: ${KEY}: the entry allows google`,
    })
    expect(hermes.commands).toEqual([])
  })
})
