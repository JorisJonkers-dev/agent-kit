import { chmod, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, dirname, join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { LedgerRecord } from '../contexts/model-gateway/index.js'
import type { ProcessCommand, ProcessResult } from '../ports/index.js'
import type { GitRemote } from '../shared-kernel/index.js'
import {
  FAKE_CHECKOUT_FILE,
  FakeEnv,
  FakeGit,
  FakeProcess,
  FakeVibeProcess,
  FixedClock,
  agyResult,
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
  /** The machine-wide ledger, in this workspace's state directory. */
  readonly ledger: string
  readonly asked: readonly string[]
  readonly ran: readonly ProcessCommand[]
  run(...argv: string[]): Promise<ModelGatewayCliResult>
  writeAllowlist(value: unknown): Promise<void>
  writeDefaults(text: string): Promise<void>
  writeInput(name: string, text: string): Promise<string>
  writeLedger(records: readonly LedgerRecord[]): Promise<void>
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
    clock: new FixedClock(),
    defaultsPath: defaults,
    env: new FakeEnv(options.withoutHome === true ? { ...options.env } : { HOME: home, ...options.env }),
    git: new FakeGit(options.repositories ?? {}),
    process,
  })
  const ledger = join(home, '.local', 'state', 'agent-kit', 'model-gateway', 'ledger.jsonl')
  return {
    allowlist: join(home, '.config', 'agent-kit', 'critique-allowlist.json'),
    asked: process.asked,
    defaults,
    dir,
    home,
    ledger,
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
    writeLedger: async (records) => {
      await mkdir(dirname(ledger), { recursive: true })
      await writeFile(ledger, records.map((record) => `${JSON.stringify(record)}\n`).join(''))
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
        hermes: { binary: 'hermes', cost_class: 'metered-personal', model_source: 'live' },
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
      `${JSON.stringify({ status: 'error', code: 'unknown-command', message: 'unknown subcommand "delegate"; the gateway has models, run and usage' })}\n`,
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

  it('lists no model for an installed client whose listing the gateway does not read yet, and does not report it missing', async () => {
    const w = await workspace({ installed: ['hermes'] })

    const result = await w.run('models', '--json', '{}')

    expect(listed(result)).toEqual([])
    expect(printed(result).missing).toEqual([
      { client: 'agy', reason: 'not installed: agy is not on PATH' },
      { client: 'claude', reason: 'not installed: claude is not on PATH' },
      { client: 'codex', reason: 'not installed: codex is not on PATH' },
      { client: 'opencode', reason: 'not installed: opencode is not on PATH' },
      { client: 'vibe', reason: 'not installed: vibe is not on PATH' },
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
  /** The fake git, which records the worktrees a run added and removed. */
  readonly git: FakeGit
  readonly ledger: string
  /** The input a run test sends unless it says otherwise: one read-only task in the test repository. */
  input(overrides?: Readonly<Record<string, unknown>>): unknown
  run(input: unknown): Promise<ModelGatewayCliResult>
  writeAllowlist(value: unknown): Promise<void>
  writeLedger(records: readonly LedgerRecord[]): Promise<void>
  writeOverride(value: unknown): Promise<void>
  writeRoutingOverride(value: unknown): Promise<void>
  writeSetup(value: unknown): Promise<void>
}

interface RunWorkspaceOptions {
  readonly scripts?: readonly VibeScript[]
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
  const git = new FakeGit(repositories)
  const runtime = defaultModelGatewayRuntime({
    clock: new FixedClock(),
    defaultsPath: defaults,
    env: new FakeEnv({ HOME: home, XDG_STATE_HOME: state, ...options.env }),
    git,
    process: vibe,
    routingPath,
  })
  const write = async (path: string, value: unknown): Promise<void> => {
    await mkdir(dirname(path), { recursive: true })
    await writeFile(path, JSON.stringify(value))
  }
  const w: RunWorkspace = {
    dir,
    git,
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
    ledger: join(state, 'agent-kit', 'model-gateway', 'ledger.jsonl'),
    repo,
    run: (input) => runModelGatewayCli(['run', '--json', JSON.stringify(input)], runtime),
    state,
    vibe,
    writeAllowlist: (value) => write(join(home, '.config', 'agent-kit', 'critique-allowlist.json'), value),
    writeLedger: async (records) => {
      const path = join(state, 'agent-kit', 'model-gateway', 'ledger.jsonl')
      await mkdir(dirname(path), { recursive: true })
      await writeFile(path, records.map((record) => `${JSON.stringify(record)}\n`).join(''))
    },
    writeOverride: (value) => write(join(home, '.config', 'agent-kit', 'model-gateway.json'), value),
    writeRoutingOverride: (value) => write(join(home, '.config', 'agent-kit', 'critique-routing.json'), value),
    writeSetup: (value) => write(join(home, '.local', 'share', 'agent-kit', 'critique', 'machine-setup.json'), value),
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

/** Asserts that a Vibe run worked in a throwaway worktree of the repository, which is gone afterwards. */
async function expectThrowawayVibeRun(w: RunWorkspace, result: ModelGatewayCliResult): Promise<void> {
  expect(printed(result).status).toBe('ok')
  const [command] = w.vibe.commands
  const workDir = String(command?.cwd)
  expect(workDir).not.toBe(w.repo)
  expect(flag(command, '--workdir')).toBe(workDir)
  expect(w.git.worktrees).toEqual([
    { action: 'add', path: workDir, repo: w.repo },
    { action: 'remove', path: workDir, repo: w.repo },
  ])
  expect(await readdir(dirname(workDir))).not.toContain(basename(workDir))
}

describe('model-gateway run: the read-only proof', () => {
  it('runs a model with no passing proof in a throwaway worktree, removed afterwards', async () => {
    const w = await readyRun({ setupResults: setupResults('fail') })

    await expectThrowawayVibeRun(w, await w.run(w.input()))
  })

  it('runs in a throwaway worktree when setup recorded no proof at all', async () => {
    const w = await readyRun({ setupResults: undefined })

    await expectThrowawayVibeRun(w, await w.run(w.input()))
  })

  it('runs in one a model no Vibe critic client runs, even when another model’s proof passed', async () => {
    const w = await readyRun({
      allowlist: allowlist(['zhipu', 'mistral'], true, true),
      routing: { clients: { 'vibe-glm': { kind: 'vibe', model: 'zai-glm-5-3' } } },
    })

    await expectThrowawayVibeRun(w, await w.run(w.input({ model: 'mistral-large-4-0' })))
  })

  it('follows this machine’s routing override, which may move a critic client to another model', async () => {
    const w = await readyRun()
    await w.writeRoutingOverride({ clients: { 'vibe-glm': { model: 'zai-glm-6-0' } } })

    await expectThrowawayVibeRun(w, await w.run(w.input()))
  })

  it('proves the alias a run names by the model id the routing names, and launches it in place', async () => {
    const w = await readyRun()

    const result = await w.run(w.input())

    expect(printed(result).status).toBe('ok')
    expect(flag(w.vibe.commands[0], '--model')).toBe('glm-5-3')
    expect(w.vibe.commands[0]?.cwd).toBe(w.repo)
    expect(w.git.worktrees).toEqual([])
  })

  it('runs in a throwaway worktree when the routing runs no client on any of the alias’s names', async () => {
    const w = await readyRun({ routing: { clients: { 'vibe-glm': { kind: 'vibe', model: 'zai-glm-6-0' } } } })

    await expectThrowawayVibeRun(w, await w.run(w.input()))
  })

  it('runs an unproven model without a repository in a fresh empty directory, removed afterwards', async () => {
    const w = await readyRun({ setupResults: undefined })

    const result = await w.run(w.input({ repo: undefined }))

    expect(printed(result).status).toBe('ok')
    const workDir = String(w.vibe.commands[0]?.cwd)
    expect(workDir.startsWith(tmpdir())).toBe(true)
    expect(w.git.worktrees).toEqual([])
    expect(await readdir(dirname(workDir))).not.toContain(basename(workDir))
  })
})

describe('model-gateway run: the launch', () => {
  it('launches Vibe non-interactively, with the model, the read-only agent and its own limits', async () => {
    const w = await readyRun()

    const result = await w.run(w.input({ max_price_usd: 0.5, time_limit_s: 120 }))

    expect(result.exitCode).toBe(0)
    const [command] = w.vibe.commands
    expect(command?.command).toBe('env')
    expect(command?.cwd).toBe(w.repo)
    expect(command?.args).toEqual([
      '-u',
      'MISTRAL_API_KEY',
      '-u',
      'VIBE_HOME',
      'vibe',
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

describe('model-gateway run: extras', () => {
  it('refuses an extras field, which only a caller inside this process may send', async () => {
    const w = await readyRun()

    const result = await w.run(w.input({ extras: { sandbox: 'read-only' } }))

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toEqual({ code: 'invalid-input', message: 'extras are for in-process callers only', status: 'error' })
    expect(w.vibe.commands).toEqual([])
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
    expect(await readdir(dirname(w.ledger))).toEqual(['ledger.jsonl'])
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

/** What `claude -p --output-format json` prints: one result object, with the cost it charged. */
function claudeResult(result: string, extra: Readonly<Record<string, unknown>> = {}): string {
  return JSON.stringify({
    is_error: false,
    result,
    subtype: 'success',
    total_cost_usd: 0.042,
    type: 'result',
    usage: { cache_creation_input_tokens: 20, cache_read_input_tokens: 300, input_tokens: 10, output_tokens: 40 },
    ...extra,
  })
}

/** What `opencode run --format json` prints: one event per line. */
function openCodeEvents(...events: readonly unknown[]): string {
  return `${events.map((event) => JSON.stringify(event)).join('\n')}\n`
}

const OPENCODE_STEP_START = { part: { type: 'step-start' }, type: 'step_start' }

function openCodeText(text: string): Record<string, unknown> {
  return { part: { text, type: 'text' }, type: 'text' }
}

function openCodeStepFinish(
  cost: number,
  tokens: { input: number; output: number; reasoning?: number; read?: number; write?: number },
): Record<string, unknown> {
  return {
    part: {
      cost,
      reason: 'stop',
      tokens: {
        cache: { read: tokens.read ?? 0, write: tokens.write ?? 0 },
        input: tokens.input,
        output: tokens.output,
        reasoning: tokens.reasoning ?? 0,
      },
      type: 'step-finish',
    },
    type: 'step_finish',
  }
}

/** A gateway whose run is permitted for every family the clients here serve, with the proofs and scripts the test names. */
function clientRun(options: RunWorkspaceOptions = {}): Promise<RunWorkspace> {
  return runWorkspace({ allowlist: allowlist(['anthropic', 'google', 'zhipu'], true, true), ...options })
}

/** The proof of the Antigravity critic client, as setup records it. */
const AGY_PROVEN = setupResults('pass', 'agy-gemini-flash-high')

/** The prompt a delegate receives for the input `runWorkspace` sends. */
const WRAPPED_PROMPT =
  'read this\n\nEnd your answer with a line "SUMMARY:" followed by at most 1500 characters that summarise the result.'

describe('model-gateway run: Antigravity', () => {
  const MODEL = 'gemini-3.8-flash-high'

  it('runs a read-only delegation in print mode in plan mode, with its model, effort, JSON output and time limit', async () => {
    const w = await clientRun({ scripts: [{ stdout: agyResult('looks fine\nSUMMARY: all good') }], setupResults: AGY_PROVEN })

    const result = await w.run(w.input({ client: 'agy', model: MODEL, time_limit_s: 120 }))

    expect(result.exitCode).toBe(0)
    const [command] = w.vibe.commands
    expect(command?.command).toBe('agy')
    expect(command?.cwd).toBe(w.repo)
    expect(command?.args).toEqual([
      '--print',
      WRAPPED_PROMPT,
      '--model',
      MODEL,
      '--output-format',
      'json',
      '--effort',
      'high',
      '--mode',
      'plan',
      '--print-timeout',
      '120s',
    ])
    expect(command?.timeoutMs).toBe(180_000)
    expect(command?.env).toEqual({ AGENT_KIT_DELEGATION: '1' })
    expect(command?.stdin).toBeUndefined()
  })

  it('reads the effort off the model id, and takes medium for an id that names none', async () => {
    const w = await clientRun({
      scripts: [{ stdout: agyResult('a') }, { stdout: agyResult('b') }, { stdout: agyResult('c') }],
      setupResults: AGY_PROVEN,
    })

    await w.run(w.input({ client: 'agy', model: 'gemini-3.8-flash-low' }))
    await w.run(w.input({ client: 'agy', model: 'gemini-3.8-flash-medium' }))
    await w.run(w.input({ client: 'agy', model: 'gemini-4-pro' }))

    expect(w.vibe.commands.map((command) => flag(command, '--effort'))).toEqual(['low', 'medium', 'medium'])
  })

  it('reports the result and the ledger line in the shape a Vibe run does', async () => {
    const w = await clientRun({ scripts: [{ stdout: agyResult('looks fine\nSUMMARY: all good') }], setupResults: AGY_PROVEN })

    const result = await w.run(w.input({ client: 'agy', model: MODEL }))

    expect(printed(result)).toEqual({
      client: 'agy',
      cost_usd: 0,
      family: 'google',
      mode: 'read-only',
      model: MODEL,
      output_path: join(runDirOf(w, result), 'output.json'),
      run_id: printed(result).run_id,
      status: 'ok',
      summary: 'all good',
      tokens: { cached: 100, input: 7661, output: 29 },
      workflow: 'spec-293',
    })
    expect(await readFile(join(runDirOf(w, result), 'output.json'), 'utf8')).toBe(agyResult('looks fine\nSUMMARY: all good'))
    expect(JSON.parse((await readFile(w.ledger, 'utf8')).trimEnd())).toEqual({
      at: expect.any(String) as unknown,
      client: 'agy',
      cost_usd: 0,
      duration_s: expect.any(Number) as unknown,
      family: 'google',
      mode: 'read-only',
      model: MODEL,
      outcome: 'ok',
      repository: KEY,
      run_id: printed(result).run_id,
      tokens: { cached: 100, input: 7661, output: 29 },
      workflow: 'spec-293',
    })
  })

  it('counts the dollars at the catalogue price when the model has one, as agy prices nothing', async () => {
    const w = await clientRun({
      override: { models: [{ client: 'agy', model: MODEL, prices: { cached_input: 0, input: 1, output: 2 } }] },
      scripts: [{ stdout: agyResult('x', { usage: { input_tokens: 1_000_000, output_tokens: 1_000_000 } }) }],
      setupResults: AGY_PROVEN,
    })

    expect(printed(await w.run(w.input({ client: 'agy', model: MODEL }))).cost_usd).toBeCloseTo(3, 10)
  })

  it('runs in place when its read-only proof as the Gemini critic passed', async () => {
    const w = await clientRun({ scripts: [{ stdout: agyResult('x') }], setupResults: AGY_PROVEN })

    await w.run(w.input({ client: 'agy', model: MODEL }))

    expect(w.git.worktrees).toEqual([])
    expect(w.vibe.commands[0]?.cwd).toBe(w.repo)
  })

  it('runs in a throwaway worktree when that proof failed or was never recorded', async () => {
    for (const setup of [setupResults('fail', 'agy-gemini-flash-high'), undefined]) {
      const w = await clientRun({ scripts: [{ stdout: agyResult('x') }], setupResults: setup })

      expect(printed(await w.run(w.input({ client: 'agy', model: MODEL }))).status).toBe('ok')

      const [command] = w.vibe.commands
      expect(command?.cwd).not.toBe(w.repo)
      expect(w.git.worktrees.map((call) => call.action)).toEqual(['add', 'remove'])
    }
  })

  it('fails a run that stopped at its print timeout, was denied an action or was not a success, and one with no answer', async () => {
    const w = await clientRun({
      scripts: [
        { stdout: agyResult('partial', { finished_reason: 'print_timeout' }) },
        { stdout: agyResult('tried', { denied_actions: [{ action: 'write_file', display_name: 'Write' }] }) },
        { stdout: agyResult('boom', { status: 'ERROR' }) },
        { stdout: agyResult('  ') },
        { stdout: 'not json' },
        { stdout: JSON.stringify(['an', 'array']) },
      ],
      setupResults: AGY_PROVEN,
    })

    const reasons: unknown[] = []
    for (let index = 0; index < 6; index += 1) {
      reasons.push(printed(await w.run(w.input({ client: 'agy', model: MODEL }))).reason)
    }

    expect(reasons).toEqual(['timeout', 'denied-action', 'client-error', 'parse-error', 'parse-error', 'parse-error'])
  })

  it('reports no tokens for a result that carries no usage, and ignores a negative or non-numeric count', async () => {
    const w = await clientRun({
      scripts: [
        { stdout: JSON.stringify({ response: 'x', status: 'SUCCESS' }) },
        { stdout: agyResult('x', { usage: { input_tokens: -1, output_tokens: 'many' } }) },
      ],
      setupResults: AGY_PROVEN,
    })

    expect(printed(await w.run(w.input({ client: 'agy', model: MODEL }))).tokens).toEqual({ cached: 0, input: 0, output: 0 })
    expect(printed(await w.run(w.input({ client: 'agy', model: MODEL }))).tokens).toEqual({ cached: 0, input: 0, output: 0 })
  })

  it('refuses a family the repository’s entry does not allow, before anything is launched', async () => {
    const w = await clientRun({ allowlist: allowlist(['zhipu'], true, true), scripts: [{ stdout: agyResult('x') }], setupResults: AGY_PROVEN })

    expect(printed(await w.run(w.input({ client: 'agy', model: MODEL }))).code).toBe('policy-refused')
    expect(w.vibe.commands).toEqual([])
    expect(w.git.worktrees).toEqual([])
  })
})

describe('model-gateway run: Claude Code', () => {
  const MODEL = 'claude-sonnet-5-5'

  it('runs a read-only delegation headless with the prompt on stdin, in plan mode with read tools only', async () => {
    const w = await clientRun({ scripts: [{ stdout: claudeResult('fine\nSUMMARY: ok') }] })

    const result = await w.run(w.input({ client: 'claude', model: MODEL, time_limit_s: 120 }))

    expect(result.exitCode).toBe(0)
    const [command] = w.vibe.commands
    expect(command?.command).toBe('claude')
    expect(command?.args).toEqual([
      '-p',
      '--model',
      MODEL,
      '--output-format',
      'json',
      '--permission-mode',
      'plan',
      '--allowedTools',
      'Read,Grep,Glob',
    ])
    expect(command?.stdin).toBe(WRAPPED_PROMPT)
    expect(command?.env).toEqual({ AGENT_KIT_DELEGATION: '1' })
    expect(command?.timeoutMs).toBe(120_000)
  })

  it('reports the result and the ledger line in the shape a Vibe run does, with the cost Claude Code charged', async () => {
    const w = await clientRun({ scripts: [{ stdout: claudeResult('fine\nSUMMARY: ok') }] })

    const result = await w.run(w.input({ client: 'claude', model: MODEL }))

    expect(printed(result)).toEqual({
      client: 'claude',
      cost_usd: 0.042,
      family: 'anthropic',
      mode: 'read-only',
      model: MODEL,
      output_path: join(runDirOf(w, result), 'output.json'),
      run_id: printed(result).run_id,
      status: 'ok',
      summary: 'ok',
      tokens: { cached: 300, input: 330, output: 40 },
      workflow: 'spec-293',
    })
    expect(JSON.parse((await readFile(w.ledger, 'utf8')).trimEnd())).toMatchObject({
      client: 'claude',
      cost_usd: 0.042,
      family: 'anthropic',
      outcome: 'ok',
      repository: KEY,
      run_id: printed(result).run_id,
      tokens: { cached: 300, input: 330, output: 40 },
    })
  })

  it('counts the dollars at the catalogue price times the tokens when Claude Code charged nothing', async () => {
    const w = await clientRun({
      override: { models: [{ client: 'claude', model: MODEL, prices: { cached_input: 0.5, input: 3, output: 15 } }] },
      scripts: [
        {
          stdout: claudeResult('x', {
            total_cost_usd: undefined,
            usage: { cache_read_input_tokens: 1_000_000, input_tokens: 1_000_000, output_tokens: 1_000_000 },
          }),
        },
      ],
    })

    expect(printed(await w.run(w.input({ client: 'claude', model: MODEL }))).cost_usd).toBeCloseTo(3 + 0.5 + 15, 10)
  })

  it('has no passing read-only proof, so it runs in a throwaway worktree of the repository', async () => {
    const seen: string[][] = []
    const w = await clientRun({
      scripts: [
        {
          effect: async (command) => {
            seen.push(await readdir(command.cwd ?? ''))
          },
          stdout: claudeResult('x'),
        },
      ],
      setupResults: AGY_PROVEN,
    })

    expect(printed(await w.run(w.input({ client: 'claude', model: MODEL }))).status).toBe('ok')

    const [command] = w.vibe.commands
    expect(seen).toEqual([[FAKE_CHECKOUT_FILE]])
    expect(command?.cwd).not.toBe(w.repo)
    expect(String(command?.cwd).startsWith(tmpdir())).toBe(true)
    expect(w.git.worktrees).toEqual([
      { action: 'add', path: command?.cwd, repo: w.repo },
      { action: 'remove', path: command?.cwd, repo: w.repo },
    ])
  })

  it('reads a result that comes as a list of events, and fails one Claude Code reports as an error', async () => {
    const w = await clientRun({
      scripts: [
        { stdout: JSON.stringify([{ type: 'system' }, JSON.parse(claudeResult('from a list')) as unknown]) },
        { stdout: claudeResult('Credit balance is too low', { is_error: true }) },
        { stdout: JSON.stringify([{ type: 'system' }]) },
        { stdout: claudeResult('') },
        { stdout: JSON.stringify({ usage: {} }) },
      ],
    })

    const runs: Record<string, unknown>[] = []
    for (let index = 0; index < 5; index += 1) runs.push(printed(await w.run(w.input({ client: 'claude', model: MODEL }))))

    expect(runs.map((run) => run.status)).toEqual(['ok', 'failed', 'failed', 'failed', 'failed'])
    expect(runs.map((run) => run.reason)).toEqual([undefined, 'client-error', 'parse-error', 'parse-error', 'parse-error'])
    expect(runs[0]?.summary).toBe('from a list')
  })
})

describe('model-gateway run: OpenCode', () => {
  const MODEL = 'opencode-go/glm-5-3'
  const STEPS = openCodeEvents(
    OPENCODE_STEP_START,
    openCodeText('thinking about it'),
    openCodeStepFinish(0.01, { input: 100, output: 10 }),
    OPENCODE_STEP_START,
    openCodeText('the answer\nSUMMARY: fine'),
    openCodeStepFinish(0.02, { input: 50, output: 5, read: 20, reasoning: 3, write: 7 }),
  )

  it('runs a read-only delegation headless with its provider/model and JSON events, in the workdir', async () => {
    const w = await clientRun({ scripts: [{ stdout: STEPS }] })

    const result = await w.run(w.input({ client: 'opencode', model: MODEL }))

    expect(result.exitCode).toBe(0)
    const [command] = w.vibe.commands
    expect(command?.command).toBe('opencode')
    expect(command?.args).toEqual(['run', '--model', MODEL, '--format', 'json', WRAPPED_PROMPT])
    expect(command?.cwd).not.toBe(w.repo)
    expect(command?.env).toEqual({ AGENT_KIT_DELEGATION: '1' })
    expect(command?.timeoutMs).toBe(900_000)
  })

  it('reports the result and the ledger line in the shape a Vibe run does, from the events the run printed', async () => {
    const w = await clientRun({ scripts: [{ stdout: STEPS }] })

    const result = await w.run(w.input({ client: 'opencode', model: MODEL }))

    expect(printed(result)).toEqual({
      client: 'opencode',
      cost_usd: expect.closeTo(0.03, 10) as unknown,
      family: 'zhipu',
      mode: 'read-only',
      model: MODEL,
      output_path: join(runDirOf(w, result), 'output.json'),
      run_id: printed(result).run_id,
      status: 'ok',
      summary: 'fine',
      tokens: { cached: 20, input: 177, output: 18 },
      workflow: 'spec-293',
    })
    expect(JSON.parse((await readFile(w.ledger, 'utf8')).trimEnd())).toMatchObject({
      client: 'opencode',
      model: MODEL,
      outcome: 'ok',
      tokens: { cached: 20, input: 177, output: 18 },
    })
  })

  it('counts the dollars at the catalogue price when the events charged nothing', async () => {
    const w = await clientRun({
      override: {
        models: [{ client: 'opencode', family: 'zhipu', model: MODEL, notes: 'n', prices: { cached_input: 0, input: 1, output: 2 }, tier: 'hard' }],
      },
      scripts: [
        { stdout: openCodeEvents(OPENCODE_STEP_START, openCodeText('x'), openCodeStepFinish(0, { input: 1_000_000, output: 1_000_000 })) },
      ],
    })

    expect(printed(await w.run(w.input({ client: 'opencode', model: MODEL }))).cost_usd).toBeCloseTo(3, 10)
  })

  it('works out the family of a model from the part after its provider', async () => {
    const w = await clientRun({ allowlist: allowlist(['zhipu'], true, true), scripts: [{ stdout: STEPS }] })

    expect(printed(await w.run(w.input({ client: 'opencode', model: 'opencode-go/kimi-k3' }))).code).toBe('policy-refused')
    expect(printed(await w.run(w.input({ client: 'opencode', model: 'opencode-go/mystery-1' }))).code).toBe('unknown-model')
    expect(w.vibe.commands).toEqual([])
  })

  it('refuses OpenCode’s pay-per-token models, and every provider the machine did not name, before anything is launched', async () => {
    const w = await clientRun({ scripts: [{ stdout: STEPS }] })

    for (const model of ['opencode/glm-5-3', 'openrouter/glm-5-3', 'glm-5-3', '/glm-5-3']) {
      const result = await w.run(w.input({ client: 'opencode', model }))
      expect(result.exitCode).toBe(1)
      expect(printed(result)).toMatchObject({ code: 'provider-not-allowed', status: 'error' })
    }
    expect(w.vibe.commands).toEqual([])
    expect(w.git.worktrees).toEqual([])
  })

  it('names the provider and the allowed ones in the refusal', async () => {
    const w = await clientRun()

    expect(printed(await w.run(w.input({ client: 'opencode', model: 'opencode/glm-5-3' })))).toEqual({
      code: 'provider-not-allowed',
      message:
        'opencode/glm-5-3: the provider opencode is not allowed (clients.opencode.providers: opencode-go), and OpenCode’s pay-per-token models are never reached',
      status: 'error',
    })
    expect(printed(await w.run(w.input({ client: 'opencode', model: 'glm-5-3' })))).toMatchObject({
      message: 'glm-5-3: an OpenCode model is named <provider>/<model>, and the allowed providers are opencode-go',
    })
  })

  it('allows a provider the machine names in clients.opencode.providers, but never the pay-per-token one', async () => {
    const w = await clientRun({
      override: { clients: { opencode: { providers: ['opencode-go', 'opencode', 'zai'] } } },
      scripts: [{ stdout: STEPS }],
    })

    expect(printed(await w.run(w.input({ client: 'opencode', model: 'opencode/glm-5-3' }))).code).toBe('provider-not-allowed')
    expect(printed(await w.run(w.input({ client: 'opencode', model: 'zai/glm-5-3' }))).status).toBe('ok')
    expect(w.vibe.commands).toHaveLength(1)
  })

  it('refuses a provider list that is not a list of names, naming the setting', async () => {
    const w = await clientRun({ override: { clients: { opencode: { providers: 'opencode-go' } } } })

    expect(printed(await w.run(w.input({ client: 'opencode', model: MODEL })))).toMatchObject({
      code: 'invalid-config',
      message: expect.stringContaining('clients.opencode.providers') as unknown,
    })
  })

  it('fails a run whose events hold an error, or no answer, and reads the answer past events it does not know', async () => {
    const w = await clientRun({
      scripts: [
        { stdout: openCodeEvents(OPENCODE_STEP_START, openCodeText('partial'), { error: { name: 'APIError' }, type: 'error' }) },
        { stdout: openCodeEvents(OPENCODE_STEP_START, openCodeStepFinish(0, { input: 1, output: 1 })) },
        {
          stdout: `not json\n${openCodeEvents(OPENCODE_STEP_START, openCodeText(''), openCodeText('a'), openCodeText('b'), { type: 'tool_use' }, 'a string', null)}`,
        },
        { stdout: openCodeEvents({ part: { text: 3 }, type: 'text' }, { part: {}, type: 'step_finish' }, { type: 'text' }) },
      ],
    })

    const runs: Record<string, unknown>[] = []
    for (let index = 0; index < 4; index += 1) runs.push(printed(await w.run(w.input({ client: 'opencode', model: MODEL }))))

    expect(runs.map((run) => run.reason)).toEqual(['client-error', 'parse-error', undefined, 'parse-error'])
    expect(runs[2]?.summary).toBe('ab')
  })
})

describe('model-gateway run: a throwaway worktree', () => {
  const MODEL = 'claude-sonnet-5-5'

  it('is removed after a run that timed out, exited non-zero or could not start', async () => {
    const w = await clientRun({ scripts: [{ stdout: '', timedOut: true }, { exitCode: 2, stdout: '' }] })

    const reasons = [
      printed(await w.run(w.input({ client: 'claude', model: MODEL }))).reason,
      printed(await w.run(w.input({ client: 'claude', model: MODEL }))).reason,
      printed(await w.run(w.input({ client: 'claude', model: MODEL }))).reason,
    ]

    expect(reasons).toEqual(['timeout', 'exit-2', 'launch-failed'])
    expect(w.git.worktrees.map((call) => call.action)).toEqual(['add', 'remove', 'add', 'remove', 'add', 'remove'])
    for (const call of w.git.worktrees) expect(call.repo).toBe(w.repo)
    for (const command of w.vibe.commands) {
      const folder = String(command.cwd)
      expect(await readdir(dirname(folder))).not.toContain(basename(folder))
    }
  })

  it('is removed when the run itself fails after the launch', async () => {
    const w = await clientRun({ scripts: [{ stdout: claudeResult('x') }] })
    const blocked = join(w.repo, '.git', 'model-gateway', 'runs')
    await mkdir(join(w.repo, '.git', 'model-gateway'), { recursive: true })
    await writeFile(blocked, 'a file where the runs directory should be')

    const result = await w.run(w.input({ client: 'claude', model: MODEL }))

    expect(result.exitCode).toBe(1)
    expect(printed(result).code).toBe('unexpected')
    expect(w.git.worktrees.map((call) => call.action)).toEqual(['add', 'remove'])
  })

  it('stops the run when git cannot add it, and leaves no directory behind', async () => {
    const w = await clientRun({
      repositories: (repo) => ({
        [repo]: { commonDir: join(repo, '.git'), remotes: [AGENT_KIT_REMOTE], root: repo, worktreeError: 'fatal: invalid reference: HEAD' },
      }),
      scripts: [{ stdout: claudeResult('x') }],
    })

    const result = await w.run(w.input({ client: 'claude', model: MODEL }))

    expect(printed(result)).toEqual({
      code: 'worktree-failed',
      message: `${w.repo}: cannot add a throwaway worktree: fatal: invalid reference: HEAD`,
      status: 'error',
    })
    expect(w.vibe.commands).toEqual([])
    const [added] = w.git.worktrees
    expect(w.git.worktrees.map((call) => call.action)).toEqual(['add'])
    expect(await readdir(dirname(String(added?.path)))).not.toContain(basename(String(added?.path)))
  })

  it('never loses the result of a run that succeeded when git cannot remove its worktree: it warns', async () => {
    const w = await clientRun({
      repositories: (repo) => ({
        [repo]: { commonDir: join(repo, '.git'), remotes: [AGENT_KIT_REMOTE], removeError: 'fatal: cannot lock', root: repo },
      }),
      scripts: [{ stdout: claudeResult('x') }],
    })

    const result = await w.run(w.input({ client: 'claude', model: MODEL }))

    expect(result.exitCode).toBe(0)
    const workDir = String(w.vibe.commands[0]?.cwd)
    expect(printed(result)).toMatchObject({
      cleanup_warning: `cannot remove the throwaway worktree ${workDir}: fatal: cannot lock`,
      status: 'ok',
    })
    expect(JSON.parse(await readFile(join(runDirOf(w, result), 'result.json'), 'utf8'))).toEqual(printed(result))
    const ledger = JSON.parse((await readFile(w.ledger, 'utf8')).trimEnd()) as Record<string, unknown>
    expect(ledger).toMatchObject({ client: 'claude', outcome: 'ok' })
    expect(ledger).not.toHaveProperty('cleanup_warning')
  })

  it('keeps the failed result of a run when git cannot remove its worktree either', async () => {
    const w = await clientRun({
      repositories: (repo) => ({
        [repo]: { commonDir: join(repo, '.git'), remotes: [AGENT_KIT_REMOTE], removeError: 'fatal: cannot lock', root: repo },
      }),
      scripts: [{ exitCode: 3, stdout: '' }],
    })

    const result = await w.run(w.input({ client: 'claude', model: MODEL }))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({ cleanup_warning: expect.stringContaining('fatal: cannot lock') as unknown, reason: 'exit-3', status: 'failed' })
  })

  it('keeps the error of a run that threw when git cannot remove its worktree either', async () => {
    const w = await clientRun({
      repositories: (repo) => ({
        [repo]: { commonDir: join(repo, '.git'), remotes: [AGENT_KIT_REMOTE], removeError: 'fatal: cannot lock', root: repo },
      }),
      scripts: [{ stdout: claudeResult('x') }],
    })
    await mkdir(join(w.repo, '.git', 'model-gateway'), { recursive: true })
    await writeFile(join(w.repo, '.git', 'model-gateway', 'runs'), 'a file where the runs directory should be')

    const result = await w.run(w.input({ client: 'claude', model: MODEL }))

    expect(result.exitCode).toBe(1)
    expect(printed(result).code).toBe('unexpected')
    expect(printed(result).message).not.toContain('cannot lock')
    expect(w.git.worktrees.map((call) => call.action)).toEqual(['add', 'remove'])
  })

  it('is a fresh empty directory outside every repository for a run without one, and is removed after it', async () => {
    const seen: string[][] = []
    const w = await clientRun({
      scripts: [
        {
          effect: async (command) => {
            seen.push(await readdir(command.cwd ?? ''))
          },
          stdout: claudeResult('x'),
        },
      ],
    })

    const result = await w.run(w.input({ client: 'claude', model: MODEL, repo: undefined }))

    expect(printed(result).status).toBe('ok')
    const workDir = String(w.vibe.commands[0]?.cwd)
    expect(seen).toEqual([[]])
    expect(workDir.startsWith(tmpdir())).toBe(true)
    expect(workDir.startsWith(w.repo)).toBe(false)
    expect(w.git.worktrees).toEqual([])
    expect(await readdir(dirname(workDir))).not.toContain(basename(workDir))
  })

  it('is not made for a Vibe model with a passing proof, which runs in place as before', async () => {
    const w = await clientRun({ scripts: [{ stdout: journal('x') }], setupResults: setupResults('pass') })

    await w.run(w.input())

    expect(w.git.worktrees).toEqual([])
    expect(w.vibe.commands[0]?.cwd).toBe(w.repo)
  })

  it('is made for a Vibe model without one, as for every other client', async () => {
    const w = await clientRun({ scripts: [{ stdout: journal('x') }] })

    await expectThrowawayVibeRun(w, await w.run(w.input()))
  })
})

/** One ledger line, as a run of `client` at `at` left it. */
function ledgerRecord(at: string, client: string, overrides: Partial<LedgerRecord> = {}): LedgerRecord {
  return {
    at,
    client,
    cost_usd: 0,
    duration_s: 1,
    family: 'mistral',
    mode: 'read-only',
    model: 'glm-5-3',
    outcome: 'ok',
    repository: null,
    run_id: `${client}-${at}`,
    tokens: { cached: 0, input: 0, output: 0 },
    workflow: 'spec-293',
    ...overrides,
  }
}

/** One client's entry in a usage report. */
function usageClient(result: ModelGatewayCliResult, id: string): Record<string, unknown> {
  const clients = printed(result).clients as Record<string, unknown>[]
  const found = clients.find((entry) => entry.client === id)
  if (found === undefined) throw new Error(`${id} is not reported`)
  return found
}

/** One client's availability in a usage report. */
function availability(result: ModelGatewayCliResult, id: string): Record<string, unknown> {
  return usageClient(result, id).availability as Record<string, unknown>
}

/** One client's plan windows in a usage report. */
function planLimits(result: ModelGatewayCliResult, id: string): Record<string, unknown>[] {
  return usageClient(result, id).plan_limits as Record<string, unknown>[]
}

/** The clients a usage report named, in the order it printed them. */
function usageClients(result: ModelGatewayCliResult): string[] {
  return (printed(result).clients as { client: string }[]).map((entry) => entry.client)
}

/** What a client's own status command answers, as the fake process plays it. */
function status(stdout: string, exitCode = 0, stderr = ''): ProcessResult {
  return { exitCode, stderr, stdout }
}

describe('model-gateway usage: availability', () => {
  it('reports every client it knows, with what it found out about each', async () => {
    const w = await workspace({ installed: ['vibe'] })

    const result = await w.run('usage', '--json', '{}')

    expect(result.exitCode).toBe(0)
    expect(result.stderr).toBe('')
    expect(printed(result)).toMatchObject({ days: 7, status: 'ok' })
    expect(usageClients(result)).toEqual(['agy', 'claude', 'codex', 'hermes', 'opencode', 'vibe'])
    expect(usageClient(result, 'vibe')).toEqual({
      availability: {
        installed: true,
        reason: 'vibe has no status command, so the gateway cannot tell whether it is signed in',
        signed_in: 'unknown',
        status: 'available',
      },
      by_day: [],
      by_workflow: [],
      client: 'vibe',
      cost_class: 'subsidised',
      plan_limits: [],
      quota: { reason: 'vibe has no status command, so the gateway cannot read its quota', status: 'unknown' },
      usage: { cost_usd: 0, runs: 0, tokens: { cached: 0, input: 0, output: 0 } },
      warnings: [],
    })
  })

  it('reports a client that is not installed as unavailable, and asks it nothing', async () => {
    const w = await workspace({ installed: ['codex'], listings: { codex: status('Logged in using ChatGPT\n') } })

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'vibe').availability).toEqual({
      installed: false,
      reason: 'not installed: vibe is not on PATH',
      signed_in: 'unknown',
      status: 'unavailable',
    })
    expect(w.ran.map((command) => command.command)).toEqual(['codex'])
  })

  it('reads availability from each client’s own status command', async () => {
    const w = await workspace({
      installed: ['codex', 'hermes', 'opencode'],
      listings: {
        codex: status('Logged in using ChatGPT\n'),
        hermes: status('copilot (1 credentials):\n  #1  gh auth token\n'),
        opencode: status('OpenCode Console  Personal  stored\n'),
      },
    })

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'codex').availability).toEqual({
      installed: true,
      reason: 'codex login status: Logged in using ChatGPT',
      signed_in: 'yes',
      status: 'available',
    })
    expect(usageClient(result, 'hermes').availability).toEqual({
      installed: true,
      reason: 'hermes auth list: it lists credentials',
      signed_in: 'yes',
      status: 'available',
    })
    expect(usageClient(result, 'opencode').availability).toEqual({
      installed: true,
      reason: 'opencode auth list: it lists credentials',
      signed_in: 'yes',
      status: 'available',
    })
    expect(w.ran.map((command) => [command.command, ...command.args].join(' '))).toEqual([
      'codex login status',
      'hermes auth list',
      'opencode auth list',
    ])
  })

  it('reports a client whose status command says it is signed out as unavailable', async () => {
    const w = await workspace({
      installed: ['codex', 'hermes', 'opencode'],
      listings: {
        codex: status('', 1, 'Not logged in\n'),
        hermes: status('\n'),
        opencode: status('', 2, 'unknown command\n'),
      },
    })

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'codex').availability).toEqual({
      installed: true,
      reason: 'codex login status: Not logged in',
      signed_in: 'no',
      status: 'unavailable',
    })
    expect(usageClient(result, 'hermes').availability).toMatchObject({
      reason: 'hermes auth list: it lists no credentials',
      signed_in: 'no',
      status: 'unavailable',
    })
    expect(usageClient(result, 'opencode').availability).toEqual({
      installed: true,
      reason: 'opencode auth list: could not be read: exited 2: unknown command',
      signed_in: 'unknown',
      status: 'available',
    })
  })

  it('reports a client whose status command cannot be run as unknown, not signed out', async () => {
    const w = await workspace({ installed: ['codex', 'hermes', 'opencode'] })

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'codex').availability).toEqual({
      installed: true,
      reason: 'codex login status: could not be read: cannot run codex',
      signed_in: 'unknown',
      status: 'available',
    })
    expect(usageClient(result, 'hermes').availability).toMatchObject({ signed_in: 'unknown', status: 'available' })
    expect(usageClient(result, 'opencode').availability).toMatchObject({ signed_in: 'unknown', status: 'available' })
  })

  it('reports a client rate-limited in the last hour as unavailable, unless a later run succeeded', async () => {
    const limited = await workspace({ installed: ['vibe'] })
    await limited.writeLedger([ledgerRecord('2026-11-04T10:40:00.000Z', 'vibe', { outcome: 'failed', reason: 'rate-limited' })])

    expect(usageClient(await limited.run('usage', '--json', '{}'), 'vibe').availability).toEqual({
      installed: true,
      reason: 'rate-limited at 2026-11-04T10:40:00.000Z, with no later run of it that succeeded',
      signed_in: 'unknown',
      status: 'unavailable',
    })

    const recovered = await workspace({ installed: ['vibe'] })
    await recovered.writeLedger([
      ledgerRecord('2026-11-04T10:40:00.000Z', 'vibe', { outcome: 'failed', reason: 'rate-limited' }),
      ledgerRecord('2026-11-04T10:45:00.000Z', 'vibe'),
    ])

    expect(availability(await recovered.run('usage', '--json', '{}'), 'vibe').status).toBe('available')
  })

  it('ignores an error older than the availability window, and takes the window from the machine override', async () => {
    const old = await workspace({ installed: ['vibe'] })
    await old.writeLedger([ledgerRecord('2026-11-04T08:00:00.000Z', 'vibe', { outcome: 'failed', reason: 'auth' })])

    expect(availability(await old.run('usage', '--json', '{}'), 'vibe').status).toBe('available')

    const wide = await workspace({ installed: ['vibe'] })
    await wide.writeOverride({ settings: { availability_window_minutes: 180 } })
    await wide.writeLedger([ledgerRecord('2026-11-04T08:00:00.000Z', 'vibe', { outcome: 'failed', reason: 'auth' })])

    expect(usageClient(await wide.run('usage', '--json', '{}'), 'vibe').availability).toMatchObject({
      reason: 'auth at 2026-11-04T08:00:00.000Z, with no later run of it that succeeded',
      status: 'unavailable',
    })
  })

  it('gives every status command a five second limit, and reads a timeout as unknown', async () => {
    const w = await workspace({
      installed: ['codex', 'hermes', 'opencode'],
      listings: {
        codex: { exitCode: 124, stderr: '', stdout: '', timedOut: true },
        hermes: status('copilot (1 credentials):\n'),
        opencode: status('OpenCode Console  Personal  stored\n'),
      },
    })

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'codex').availability).toEqual({
      installed: true,
      reason: 'codex login status: status command did not answer in time',
      signed_in: 'unknown',
      status: 'available',
    })
    expect(w.ran.map((command) => command.timeoutMs)).toEqual([5000, 5000, 5000])
  })

  it('shows a client’s own quota as unknown, never zero', async () => {
    const w = await workspace({ installed: ['codex'], listings: { codex: status('Logged in using ChatGPT\n') } })

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'codex').quota).toEqual({ reason: 'codex login status reports no quota', status: 'unknown' })
  })
})

describe('model-gateway usage: the ledger', () => {
  it('counts each client’s runs, tokens and dollars by day and by workflow', async () => {
    const w = await workspace({ installed: ['vibe'] })
    await w.writeLedger([
      ledgerRecord('2026-11-03T09:00:00.000Z', 'vibe', { cost_usd: 0.25, tokens: { cached: 0, input: 100, output: 10 } }),
      ledgerRecord('2026-11-04T09:00:00.000Z', 'vibe', { cost_usd: 0.5, tokens: { cached: 5, input: 200, output: 20 } }),
      ledgerRecord('2026-11-04T10:00:00.000Z', 'vibe', { cost_usd: 0.125, tokens: { cached: 0, input: 50, output: 5 }, workflow: 'review' }),
      ledgerRecord('2026-11-04T10:00:00.000Z', 'codex', { cost_usd: 9, tokens: { cached: 0, input: 1, output: 1 } }),
    ])

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'vibe').usage).toEqual({ cost_usd: 0.875, runs: 3, tokens: { cached: 5, input: 350, output: 35 } })
    expect(usageClient(result, 'vibe').by_day).toEqual([
      { cost_usd: 0.25, day: '2026-11-03', runs: 1, tokens: { cached: 0, input: 100, output: 10 } },
      { cost_usd: 0.625, day: '2026-11-04', runs: 2, tokens: { cached: 5, input: 250, output: 25 } },
    ])
    expect(usageClient(result, 'vibe').by_workflow).toEqual([
      { cost_usd: 0.125, runs: 1, tokens: { cached: 0, input: 50, output: 5 }, workflow: 'review' },
      { cost_usd: 0.75, runs: 2, tokens: { cached: 5, input: 300, output: 30 }, workflow: 'spec-293' },
    ])
    expect(usageClient(result, 'codex').usage).toEqual({ cost_usd: 9, runs: 1, tokens: { cached: 0, input: 1, output: 1 } })
  })

  it('narrows the usage to one workflow when the caller names one', async () => {
    const w = await workspace({ installed: ['vibe'] })
    await w.writeLedger([
      ledgerRecord('2026-11-04T09:00:00.000Z', 'vibe', { cost_usd: 0.5, tokens: { cached: 0, input: 200, output: 20 } }),
      ledgerRecord('2026-11-04T10:00:00.000Z', 'vibe', { cost_usd: 0.125, tokens: { cached: 0, input: 50, output: 5 }, workflow: 'review' }),
    ])

    const result = await w.run('usage', '--json', JSON.stringify({ workflow: 'review' }))

    expect(printed(result)).toMatchObject({ workflow: 'review' })
    expect(usageClient(result, 'vibe').usage).toEqual({ cost_usd: 0.125, runs: 1, tokens: { cached: 0, input: 50, output: 5 } })
    expect(usageClient(result, 'vibe').by_workflow).toEqual([
      { cost_usd: 0.125, runs: 1, tokens: { cached: 0, input: 50, output: 5 }, workflow: 'review' },
    ])
  })

  it('covers the last days days, ending today', async () => {
    const w = await workspace({ installed: ['vibe'] })
    await w.writeLedger([
      ledgerRecord('2026-10-28T09:00:00.000Z', 'vibe', { cost_usd: 1 }),
      ledgerRecord('2026-10-29T00:00:00.000Z', 'vibe', { cost_usd: 2 }),
    ])

    expect(usageClient(await w.run('usage', '--json', '{}'), 'vibe').usage).toEqual({
      cost_usd: 2,
      runs: 1,
      tokens: { cached: 0, input: 0, output: 0 },
    })
    expect(usageClient(await w.run('usage', '--json', JSON.stringify({ days: 30 })), 'vibe').usage).toEqual({
      cost_usd: 3,
      runs: 2,
      tokens: { cached: 0, input: 0, output: 0 },
    })
  })

  it('refuses days that is not a whole number above 0, and a workflow that is not a name', async () => {
    const w = await workspace()

    expect(printed(await w.run('usage', '--json', JSON.stringify({ days: 0 })))).toMatchObject({ code: 'invalid-input', status: 'error' })
    expect(printed(await w.run('usage', '--json', JSON.stringify({ days: 1.5 })))).toMatchObject({ code: 'invalid-input', status: 'error' })
    expect(printed(await w.run('usage', '--json', JSON.stringify({ workflow: '' })))).toMatchObject({ code: 'invalid-input', status: 'error' })
  })
})

describe('model-gateway usage: plan limits', () => {
  /** The owner's OpenCode Go plan, as the machine file carries it. */
  const OPENCODE_GO = [
    { limit_usd: 12, window: '5h' },
    { limit_usd: 30, window: 'week' },
    { limit_usd: 60, window: 'month' },
  ]

  it('watches each window of a plan limit, and warns at the warning ratio', async () => {
    const w = await workspace({ installed: ['opencode'] })
    await w.writeOverride({ clients: { opencode: { plan_limits: OPENCODE_GO } } })
    await w.writeLedger([
      ledgerRecord('2026-11-02T10:00:00.000Z', 'opencode', { cost_usd: 25 }),
      ledgerRecord('2026-11-04T09:00:00.000Z', 'opencode', { cost_usd: 3 }),
    ])

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'opencode').plan_limits).toEqual([
      { limit_usd: 12, ratio: 0.25, spent_usd: 3, state: 'ok', window: '5h' },
      { limit_usd: 30, ratio: 0.9333, spent_usd: 28, state: 'warning', window: 'week' },
      { limit_usd: 60, ratio: 0.4667, spent_usd: 28, state: 'ok', window: 'month' },
    ])
    expect(usageClient(result, 'opencode').warnings).toEqual(['opencode has used 93% of its week plan limit ($28.00 of $30.00)'])
    expect(availability(result, 'opencode').status).toBe('available')
  })

  it('reports a client at 100% of a window as unable to start', async () => {
    const w = await workspace({ installed: ['opencode'] })
    await w.writeOverride({ clients: { opencode: { plan_limits: OPENCODE_GO } } })
    await w.writeLedger([ledgerRecord('2026-11-04T09:00:00.000Z', 'opencode', { cost_usd: 12 })])

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'opencode').availability).toEqual({
      installed: true,
      reason: 'at 100% of its 5h plan limit ($12.00 of $12.00)',
      signed_in: 'unknown',
      status: 'unable_to_start',
    })
    expect(planLimits(result, 'opencode')[0]).toEqual({
      limit_usd: 12,
      ratio: 1,
      spent_usd: 12,
      state: 'exhausted',
      window: '5h',    })
  })

  it('counts a window’s runs when the plan limit names runs instead of dollars', async () => {
    const w = await workspace({ installed: ['vibe'] })
    await w.writeOverride({ clients: { vibe: { plan_limits: [{ limit_runs: 10, window: 'day' }] } } })
    await w.writeLedger(Array.from({ length: 9 }, (_, index) => ledgerRecord(`2026-11-04T0${String(index)}:00:00.000Z`, 'vibe')))

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'vibe').plan_limits).toEqual([{ limit_runs: 10, ratio: 0.9, runs: 9, state: 'warning', window: 'day' }])
    expect(usageClient(result, 'vibe').warnings).toEqual(['vibe has used 90% of its day plan limit (9 of 10 runs)'])
  })

  it('leaves a client without plan limits unwatched', async () => {
    const w = await workspace({ installed: ['vibe'] })
    await w.writeLedger([ledgerRecord('2026-11-04T09:00:00.000Z', 'vibe', { cost_usd: 100 })])

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'vibe').plan_limits).toEqual([])
    expect(usageClient(result, 'vibe').warnings).toEqual([])
  })

  it('refuses a plan limit it cannot read, naming what is wrong with it', async () => {
    const cases: readonly unknown[] = [
      {},
      [5],
      [{ limit_usd: 5, window: 'fortnight' }],
      [{ limit_runs: 5, limit_usd: 5, window: 'day' }],
      [{ window: 'day' }],
      [{ limit_usd: 0, window: 'day' }],
      [{ limit_runs: 1.5, window: 'day' }],
    ]
    for (const planLimits of cases) {
      const w = await workspace()
      await w.writeOverride({ clients: { vibe: { plan_limits: planLimits } } })

      expect(printed(await w.run('usage', '--json', '{}'))).toMatchObject({ code: 'invalid-config', status: 'error' })
    }
  })
})

describe('model-gateway run: a client that refuses', () => {
  it('records a rate-limited run’s reason, so availability can see it', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 1, stderr: '429 Too Many Requests', stdout: '' }] })

    const result = await w.run(w.input())

    expect(printed(result)).toMatchObject({ reason: 'rate-limited', status: 'failed' })
    expect(JSON.parse((await readFile(w.ledger, 'utf8')).trimEnd())).toMatchObject({ outcome: 'failed', reason: 'rate-limited' })
  })

  it('records an auth failure’s reason', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 1, stderr: 'Not logged in. Run `vibe login`.', stdout: '' }] })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'auth', status: 'failed' })
  })

  it('reads the exit code when the client says nothing', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 429, stdout: '' }] })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'rate-limited', status: 'failed' })
  })
})

describe('model-gateway run: what refused it', () => {
  it('does not read a refusal out of the delegate’s own answer', async () => {
    const w = await readyRun({
      scripts: [{ exitCode: 1, stdout: journal('the log says rate limit exceeded, 429, 401 and 4.429'), stderr: '' }],
    })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'exit-1', status: 'failed' })
  })

  it('reads a rate limit from the client’s own error text', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 1, stderr: 'Error: rate limit exceeded, retry later', stdout: '' }] })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'rate-limited', status: 'failed' })
  })

  it('reads a rate limit from the HTTP status of the client’s structured error', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 1, exportJson: { error: { message: 'nope', status: 429 } }, stdout: '' }] })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'rate-limited', status: 'failed' })
  })

  it('reads a rate limit from the stop reason the client exported', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 1, exportJson: { stop_reason: 'rate_limited' }, stdout: '' }] })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'rate-limited', status: 'failed' })
  })

  it('reads a refused credential from the error result the client exported', async () => {
    const w = await readyRun({
      scripts: [{ exitCode: 1, exportJson: { is_error: true, result: 'unauthorized: invalid api key' }, stdout: '' }],
    })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'auth', status: 'failed' })
  })

  it('reads a rate limit from the HTTP status the client exported beside no error', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 1, exportJson: { status: 429 }, stdout: '' }] })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'rate-limited', status: 'failed' })
  })

  it('reads a rate limit from the error text the client exported', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 1, exportJson: { error: 'rate limit exceeded' }, stdout: '' }] })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'rate-limited', status: 'failed' })
  })

  it('falls back to the exit code when the export carries no error', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 1, exportJson: { session_id: 'abc' }, stdout: '' }] })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'exit-1', status: 'failed' })
  })

  it('ignores a status that is not a whole number', async () => {
    const w = await readyRun({
      scripts: [{ exitCode: 1, exportJson: { error: { message: 'boom', status: 'nope' } }, stdout: '' }],
    })

    expect(printed(await w.run(w.input()))).toMatchObject({ reason: 'exit-1', status: 'failed' })
  })
})

describe('model-gateway run: the plan limit', () => {
  it('warns in the result when the client is near its plan limit', async () => {
    const w = await readyRun({ override: { clients: { vibe: { plan_limits: [{ limit_usd: 1, window: 'day' }] } } } })
    await w.writeLedger([ledgerRecord('2026-11-04T09:00:00.000Z', 'vibe', { cost_usd: 0.9 })])

    const result = await w.run(w.input())

    expect(printed(result).warnings).toEqual(['vibe has used 91% of its day plan limit ($0.91 of $1.00)'])
  })

  it('leaves the warning out when the client has no plan limit', async () => {
    const w = await readyRun()

    expect(printed(await w.run(w.input()))).not.toHaveProperty('warnings')
  })
})
