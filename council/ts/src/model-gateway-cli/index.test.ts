import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { GitRemote } from '../shared-kernel/index.js'
import { FakeEnv, FakeGit, FakeProcess, type FakeRepository } from '../../test/model-gateway/harness.js'
import { defaultModelGatewayRuntime, MODEL_GATEWAY_USAGE, runModelGatewayCli, type ModelGatewayCliResult } from './index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

/** The defaults agent-kit ships, which the bundle reads from its own folder. */
const SHIPPED_DEFAULTS = join(import.meta.dirname, '../../../../skills/workstation/model-gateway/defaults.json')

interface Workspace {
  readonly dir: string
  readonly home: string
  readonly defaults: string
  readonly override: string
  readonly allowlist: string
  readonly asked: readonly string[]
  run(...argv: string[]): Promise<ModelGatewayCliResult>
  writeAllowlist(value: unknown): Promise<void>
  writeDefaults(text: string): Promise<void>
  writeInput(name: string, text: string): Promise<string>
  writeOverride(value: unknown): Promise<void>
}

interface WorkspaceOptions {
  readonly installed?: readonly string[]
  readonly env?: Readonly<Record<string, string>>
  readonly repositories?: Readonly<Record<string, FakeRepository>>
}

async function workspace(options: WorkspaceOptions = {}): Promise<Workspace> {
  const dir = await mkdtemp(join(tmpdir(), 'model-gateway-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const home = join(dir, 'home')
  const defaults = join(dir, 'defaults.json')
  await writeFile(defaults, await readFile(SHIPPED_DEFAULTS, 'utf8'))
  const process = new FakeProcess(options.installed ?? ['claude', 'codex'])
  const runtime = defaultModelGatewayRuntime({
    defaultsPath: defaults,
    env: new FakeEnv({ HOME: home, ...options.env }),
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

/** One listed model, as the run printed it. */
function model(result: ModelGatewayCliResult, key: string): Record<string, unknown> | undefined {
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
      missing: [],
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

  it('asks PATH only about the clients whose models it declares', async () => {
    const w = await workspace()

    await w.run('models', '--json', '{}')

    expect(w.asked).toEqual(['claude', 'codex'])
  })

  it('leaves out a client that is not installed and names it with the reason', async () => {
    const w = await workspace({ installed: ['claude'] })

    const result = await w.run('models', '--json', '{}')

    expect(printed(result).missing).toEqual([{ client: 'codex', reason: 'not installed: codex is not on PATH' }])
    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
  })

  it('lists no model for a client whose models come from itself or its configuration, and does not report it missing', async () => {
    const w = await workspace({ installed: [] })

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual([])
    expect(printed(result).missing).toEqual([
      { client: 'claude', reason: 'not installed: claude is not on PATH' },
      { client: 'codex', reason: 'not installed: codex is not on PATH' },
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

    expect(model(result, 'claude:claude-opus-5-5')).toMatchObject({ inline_only: false, may_send: { code: true, history: false } })
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
    expect(model(result, 'codex:gpt-5.5')).toMatchObject({ inline_only: false, may_send: { code: true, history: true } })
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
    expect(model(result, 'hermes:hermes-4')).toMatchObject({ inline_only: true, may_send: { code: false, history: false } })
    expect(printed(result).policy).toBe('no repository: only inline content, to subsidised and metered-personal clients')
  })

  it('treats a path outside any repository as outside', async () => {
    const w = await workspace()

    const result = await w.run('models', '--json', JSON.stringify({ repo: '/tmp/nowhere' }))

    expect(listed(result)).toEqual(['claude:claude-haiku-5-5', 'claude:claude-opus-5-5', 'claude:claude-sonnet-5-5'])
    expect(model(result, 'claude:claude-opus-5-5')).toMatchObject({ inline_only: true })
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
        hermes: { binary: 'hermes', cost_class: 'metered-personal', model_source: 'live' },
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
          tier: 'routine',
        },
        {
          client: 'vibe',
          family: 'zhipu',
          model: 'zai-glm-5-3',
          notes: 'Strong code review and integration reasoning.',
          tier: 'hard',
        },
        {
          client: 'vibe',
          family: 'mistral',
          model: 'mistral-large-4-0',
          notes: 'Broad general model.',
          tier: 'hard',
        },
        {
          client: 'vibe',
          family: 'mistral',
          model: 'mistral-small-latest',
          notes: 'Very cheap; short summaries and classification.',
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
      `${JSON.stringify({ status: 'error', code: 'unknown-command', message: 'unknown subcommand "delegate"; the gateway has models' })}\n`,
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
