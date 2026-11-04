import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { ModelGatewayProcessPort } from '../contexts/model-gateway/index.js'
import type { EnvPort } from '../ports/index.js'
import { defaultModelGatewayRuntime, MODEL_GATEWAY_USAGE, runModelGatewayCli, type ModelGatewayCliResult } from './index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

/** The defaults agent-kit ships, which the bundle reads from its own folder. */
const SHIPPED_DEFAULTS = join(import.meta.dirname, '../../../../skills/workstation/model-gateway/defaults.json')

class FakeEnv implements EnvPort {
  constructor(private readonly values: Readonly<Record<string, string>>) {}

  get(name: string): string | undefined {
    return this.values[name]
  }

  require(name: string): string {
    const value = this.get(name)
    if (value === undefined) throw new Error(`required environment variable is missing: ${name}`)
    return value
  }

  all(): Readonly<Record<string, string>> {
    return this.values
  }
}

/** A PATH that holds exactly the programs the test names, and records what was asked. */
class FakeProcess implements ModelGatewayProcessPort {
  readonly asked: string[] = []

  constructor(private readonly installed: readonly string[]) {}

  which(binary: string): Promise<boolean> {
    this.asked.push(binary)
    return Promise.resolve(this.installed.includes(binary))
  }
}

interface Workspace {
  readonly dir: string
  readonly home: string
  readonly defaults: string
  readonly override: string
  readonly asked: readonly string[]
  run(...argv: string[]): Promise<ModelGatewayCliResult>
  writeDefaults(text: string): Promise<void>
  writeInput(name: string, text: string): Promise<string>
  writeOverride(value: unknown): Promise<void>
}

interface WorkspaceOptions {
  readonly installed?: readonly string[]
  readonly env?: Readonly<Record<string, string>>
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
    process,
  })
  return {
    asked: process.asked,
    defaults,
    dir,
    home,
    override: join(home, '.config', 'agent-kit', 'model-gateway.json'),
    run: (...argv) => runModelGatewayCli(argv, runtime),
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

describe('model-gateway models', () => {
  it('lists the declared models of every installed client, sorted by client then model', async () => {
    const w = await workspace()

    const result = await w.run('models', '--json', '{}')

    expect(result.exitCode).toBe(0)
    expect(result.stderr).toBe('')
    expect(printed(result)).toEqual({
      missing: [],
      models: [
        {
          client: 'claude',
          cost_class: 'subsidised',
          family: 'anthropic',
          model: 'claude-haiku-5-5',
          notes: 'Fast and cheap; mechanical edits, summaries, lookups.',
          source: 'declared',
          tier: 'routine',
        },
        {
          client: 'claude',
          cost_class: 'subsidised',
          family: 'anthropic',
          model: 'claude-opus-5-5',
          notes: 'Strongest Claude; design, hard debugging, review of risky changes.',
          source: 'declared',
          tier: 'frontier',
        },
        {
          client: 'claude',
          cost_class: 'subsidised',
          family: 'anthropic',
          model: 'claude-sonnet-5-5',
          notes: 'Strong general coding and refactoring.',
          source: 'declared',
          tier: 'hard',
        },
        {
          client: 'codex',
          cost_class: 'metered-employer',
          family: 'openai',
          model: 'gpt-5.5',
          notes: 'Strong implementation and review; metered on the nedap account, grant needed.',
          source: 'declared',
          tier: 'frontier',
        },
      ],
      status: 'ok',
    })
  })

  it('reads the same input from a file with --input', async () => {
    const w = await workspace()
    const input = await w.writeInput('input.json', '{}')

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
