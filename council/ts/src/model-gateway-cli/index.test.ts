import { chmod, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, dirname, join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { ModelGatewayError, NodeModelGatewayFiles } from '../contexts/model-gateway/index.js'
import type {
  LedgerEntry,
  LedgerRecord,
  ModelGatewayFilesPort,
  ModelGatewayKeptPort,
  ModelGatewayProcessPort,
  ModelGatewayWorkflowsPort,
  ReservationRecord,
  SettledRecord,
} from '../contexts/model-gateway/index.js'
import type { ProcessCommand, ProcessResult } from '../ports/index.js'
import type { GitRemote } from '../shared-kernel/index.js'
import {
  FAKE_CHECKOUT_FILE,
  FakeEnv,
  FakeGit,
  FakeHermesProcess,
  FakeKept,
  FakeProcess,
  FakeVibeProcess,
  FakeWorkflows,
  FixedClock,
  agyResult,
  type FakeRepository,
  type VibeScript,
} from '../../test/model-gateway/harness.js'
import {
  defaultModelGatewayRuntime,
  MODEL_GATEWAY_USAGE,
  runModelGatewayCli,
  type ModelGatewayCliResult,
  type ModelGatewayRuntime,
} from './index.js'

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
  writeLedger(records: readonly LedgerEntry[]): Promise<void>
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

/** What a run printed less the attempts of its delegation, which is what the run's own `result.json` holds. */
function withoutAttempts(result: ModelGatewayCliResult): Record<string, unknown> {
  return Object.fromEntries(Object.entries(printed(result)).filter(([key]) => key !== 'attempts'))
}

/** The ledger's lines, as the gateway wrote them: a run, a reservation or a settlement each. */
function ledgerLines(text: string): Record<string, unknown>[] {
  return text
    .trimEnd()
    .split('\n')
    .filter((line) => line !== '')
    .map((line) => JSON.parse(line) as Record<string, unknown>)
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
          prices: { cached_input: 0.125, input: 1.25, output: 10 },
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
        output_allowance_tokens: { frontier: 16000, hard: 6000, routine: 2000, trivial: 500 },
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
      `${JSON.stringify({ status: 'error', code: 'unknown-command', message: 'unknown subcommand "delegate"; the gateway has models, run, usage, grant write, grant list, workflow start, workflow end, workflow list and worktrees clean' })}\n`,
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
  /** The fake git, which records the worktrees a run added and removed. */
  readonly git: FakeGit
  readonly ledger: string
  /** The file that lists the writing worktrees the gateway kept. */
  readonly kept: string
  /** The file that holds the workflows every run names. */
  readonly workflows: string
  /** Any gateway command, by its words. */
  cli(...argv: string[]): Promise<ModelGatewayCliResult>
  /** A second gateway process over the same machine: its own runtime and its own fake client, the same ledger. */
  gateway(process: ModelGatewayProcessPort): ModelGatewayRuntime
  /** The input a run test sends unless it says otherwise: one read-only task in the test repository, with no escalation (a test of the escalation turns it on). */
  input(overrides?: Readonly<Record<string, unknown>>): unknown
  run(input: unknown): Promise<ModelGatewayCliResult>
  writeAllowlist(value: unknown): Promise<void>
  writeLedger(records: readonly LedgerEntry[]): Promise<void>
  writeOverride(value: unknown): Promise<void>
  writeRoutingOverride(value: unknown): Promise<void>
  writeSetup(value: unknown): Promise<void>
  writeWorkflows(value: unknown): Promise<void>
}

interface RunWorkspaceOptions {
  /** The clock the gateway reads, when a test moves it on while a delegate runs. */
  readonly clock?: FixedClock
  readonly scripts?: readonly VibeScript[]
  /** The client the gateway launches, when a test runs something other than Vibe. */
  readonly process?: ModelGatewayProcessPort
  /** The repositories the fake git knows, keyed by path; the test repository by default. */
  readonly repositories?: (repo: string) => Readonly<Record<string, FakeRepository>>
  readonly env?: Readonly<Record<string, string>>
  readonly routing?: unknown
  readonly setupResults?: unknown
  readonly allowlist?: unknown
  readonly override?: unknown
  /** The shipped defaults the gateway reads, when a test needs a broken catalogue. */
  readonly defaults?: unknown
  /** The kept-worktrees list, when a test needs one that fails; the real file in the state directory by default. */
  readonly kept?: ModelGatewayKeptPort
  /** The workflows port, when a test needs one that fails to start a workflow; the real file by default. */
  readonly workflows?: ModelGatewayWorkflowsPort
  /** The files port, when a test needs one that cannot look at a path; the real file system by default. */
  readonly files?: ModelGatewayFilesPort
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
    clock: options.clock ?? new FixedClock(),
    defaultsPath: defaults,
    env: new FakeEnv({ HOME: home, XDG_STATE_HOME: state, ...options.env }),
    git,
    process: options.process ?? vibe,
    routingPath,
    ...(options.kept === undefined ? {} : { kept: options.kept }),
    ...(options.workflows === undefined ? {} : { workflows: options.workflows }),
    ...(options.files === undefined ? {} : { files: options.files }),
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
      escalate: false,
      mode: 'read-only',
      model: 'glm-5-3',
      prompt: 'read this',
      repo,
      workflow: 'spec-293',
      ...overrides,
    }),
    cli: (...argv) => runModelGatewayCli(argv, runtime),
    gateway: (process) =>
      defaultModelGatewayRuntime({
        clock: new FixedClock(),
        defaultsPath: defaults,
        env: new FakeEnv({ HOME: home, XDG_STATE_HOME: state, ...options.env }),
        git,
        process,
        routingPath,
      }),
    kept: join(state, 'agent-kit', 'model-gateway', 'kept-worktrees.json'),
    ledger: join(state, 'agent-kit', 'model-gateway', 'ledger.jsonl'),
    repo,
    run: (input) => runModelGatewayCli(['run', '--json', JSON.stringify(input)], runtime),
    state,
    vibe,
    workflows: join(state, 'agent-kit', 'model-gateway', 'workflows.json'),
    writeAllowlist: (value) => write(join(home, '.config', 'agent-kit', 'critique-allowlist.json'), value),
    writeLedger: async (records) => {
      const path = join(state, 'agent-kit', 'model-gateway', 'ledger.jsonl')
      await mkdir(dirname(path), { recursive: true })
      await writeFile(path, records.map((record) => `${JSON.stringify(record)}\n`).join(''))
    },
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

    expect(printed(await w.run(w.input({ client: 'gemini' }))).code).toBe('not-supported-yet')
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
      attempts: [{ client: 'vibe', model: 'glm-5-3', run_id: printed(result).run_id, status: 'ok' }],
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
    expect(JSON.parse(await readFile(join(runDir, 'result.json'), 'utf8'))).toEqual(withoutAttempts(result))
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
      cost_class: 'subsidised',
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
      attempts: [{ client: 'agy', model: MODEL, run_id: printed(result).run_id, status: 'ok' }],
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
      cost_class: 'subsidised',
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
      attempts: [{ client: 'claude', model: MODEL, run_id: printed(result).run_id, status: 'ok' }],
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
      attempts: [{ client: 'opencode', model: MODEL, run_id: printed(result).run_id, status: 'ok' }],
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
      override: {
        clients: { opencode: { providers: ['opencode-go', 'opencode', 'zai'] } },
        models: [
          {
            client: 'opencode',
            family: 'zhipu',
            model: 'zai/glm-5-3',
            notes: 'Metered on the provider account.',
            prices: { cached_input: 0, input: 1, output: 2 },
            tier: 'hard',
          },
        ],
      },
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
    expect(JSON.parse(await readFile(join(runDirOf(w, result), 'result.json'), 'utf8'))).toEqual(withoutAttempts(result))
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

/** One ledger line, as a run of `client` at `at` left it, charged to the metered-personal caps unless it says otherwise. */
function ledgerRecord(at: string, client: string, overrides: Partial<LedgerRecord> = {}): LedgerRecord {
  return {
    at,
    client,
    cost_class: 'metered-personal',
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

/** A run's line as a client that reported no token counts leaves it: the gateway cannot read them, so the line carries none. */
function ledgerRecordWithoutTokens(at: string, client: string, overrides: Partial<LedgerRecord> = {}): LedgerRecord {
  const record = { ...ledgerRecord(at, client, overrides) }
  delete record.tokens
  return record
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
      reserved_runs: 0,
      reserved_usd: 0,
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

  it('leaves the token totals unknown for a run that reported none, and still counts the run', async () => {
    const w = await workspace({ installed: ['vibe'] })
    await w.writeLedger([ledgerRecordWithoutTokens('2026-11-04T09:00:00.000Z', 'vibe', { cost_usd: 0.5 })])

    const result = await w.run('usage', '--json', '{}')

    expect(usageClient(result, 'vibe').usage).toEqual({ cost_usd: 0.5, runs: 1 })
    expect(usageClient(result, 'vibe').by_day).toEqual([{ cost_usd: 0.5, day: '2026-11-04', runs: 1 }])
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

const EDIT_RULE = 'Edit only files inside the working directory. Do not commit.'

/** The git operations a run may make: it finds, creates, stages, lists and removes, and never commits, merges or pushes. */
const ALLOWED_GIT_CALLS = new Set([
  'commonDir',
  'createWorktree',
  'deleteBranch',
  'remotes',
  'removeWorktree',
  'root',
  'stageAll',
  'stagedFiles',
])

/** The input of a writing run: one task, on a base, with a check. */
function writeInput(w: RunWorkspace, overrides: Readonly<Record<string, unknown>> = {}): unknown {
  return w.input({ base: 'main', check: 'npm test', mode: 'write', ...overrides })
}

/** What a fake delegate does in its workdir: writes the files it names, as a client that edits would. */
function edits(...files: readonly string[]): (command: ProcessCommand) => Promise<void> {
  return async (command) => {
    for (const file of files) {
      await mkdir(dirname(join(command.cwd ?? '', file)), { recursive: true })
      await writeFile(join(command.cwd ?? '', file), 'edited')
    }
  }
}

/** The worktree a writing run was given, from its result. */
function worktreeOf(result: ModelGatewayCliResult): string {
  return String(printed(result).worktree)
}

describe('model-gateway run: writing, the input', () => {
  it('refuses a writing run without a repository, before anything is launched', async () => {
    const w = await readyRun({ scripts: [{ stdout: journal('x') }] })

    const result = await w.run(writeInput(w, { repo: undefined }))

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'write-needs-repository', status: 'error' })
    expect(w.vibe.commands).toEqual([])
    expect(w.git.worktrees).toEqual([])
  })

  it('refuses a writing run without a base or without a check, naming the field', async () => {
    const w = await readyRun()

    expect(printed(await w.run(writeInput(w, { base: undefined })))).toEqual({
      code: 'invalid-input',
      message: 'a writing run must name its base, a ref the worktree starts from',
      status: 'error',
    })
    expect(printed(await w.run(writeInput(w, { check: undefined })))).toEqual({
      code: 'invalid-input',
      message: 'a writing run must name its check, a command that tests the change',
      status: 'error',
    })
    expect(printed(await w.run(writeInput(w, { base: '' }))).code).toBe('invalid-input')
    expect(printed(await w.run(writeInput(w, { check: 7 }))).code).toBe('invalid-input')
    expect(w.vibe.commands).toEqual([])
  })

  it('refuses a base or a check on a read-only run, which has neither', async () => {
    const w = await readyRun()

    expect(printed(await w.run(w.input({ base: 'main' })))).toMatchObject({
      code: 'invalid-input',
      message: 'base and check are for writing runs only',
    })
    expect(printed(await w.run(w.input({ check: 'npm test' }))).code).toBe('invalid-input')
    expect(w.vibe.commands).toEqual([])
  })

  it('needs the code permission of the repository’s entry, as every run that reads a repository does', async () => {
    const w = await readyRun({ allowlist: allowlist(['zhipu'], false, true) })

    expect(printed(await w.run(writeInput(w)))).toMatchObject({
      code: 'policy-refused',
      message: `vibe:glm-5-3 may not carry code: ${KEY}: the entry allows zhipu`,
    })
    expect(w.vibe.commands).toEqual([])
    expect(w.git.worktrees).toEqual([])
  })

  it('refuses a model the entry’s families do not allow, as a read-only run is', async () => {
    const w = await readyRun({ allowlist: allowlist(['mistral'], true, true) })

    expect(printed(await w.run(writeInput(w))).code).toBe('policy-refused')
    expect(w.git.worktrees).toEqual([])
  })
})

describe('model-gateway run: writing, the worktree and the check', () => {
  it('creates a worktree off the base on a new branch, runs the delegate there only, stages its work, then runs the check', async () => {
    const stagedBeforeCheck: number[] = []
    const w = await readyRun({
      scripts: [
        { effect: edits('src/a.ts', 'lib/b.ts'), exportJson: exported({ cost: 0.02, input: 100, output: 10 }), stdout: journal('done\nSUMMARY: edited two files') },
        {
          effect: () => {
            stagedBeforeCheck.push(w.git.stagedIn.length)
            return Promise.resolve()
          },
          stdout: '3 passed\n',
        },
      ],
    })

    const result = await w.run(writeInput(w, { time_limit_s: 120 }))

    expect(result.exitCode).toBe(0)
    const out = printed(result)
    const runId = String(out.run_id)
    const worktree = join(w.repo, '.git', 'model-gateway', 'worktrees', runId)
    expect(out).toEqual({
      attempts: [{ client: 'vibe', model: 'glm-5-3', run_id: runId, status: 'ok' }],
      branch: `gateway/${runId}`,
      check: { command: 'npm test', exit_code: 0, output_path: join(runDirOf(w, result), 'check-output.txt') },
      client: 'vibe',
      cost_usd: 0.02,
      family: 'zhipu',
      mode: 'write',
      model: 'glm-5-3',
      output_path: join(runDirOf(w, result), 'output.json'),
      run_id: runId,
      staged_files: ['lib/b.ts', 'src/a.ts'],
      status: 'ok',
      summary: 'edited two files',
      tokens: { cached: 0, input: 100, output: 10 },
      workflow: 'spec-293',
      worktree,
    })
    expect(w.git.worktrees).toEqual([{ action: 'create', base: 'main', branch: `gateway/${runId}`, path: worktree, repo: w.repo }])
    const [delegate, check] = w.vibe.commands
    expect(delegate?.cwd).toBe(worktree)
    expect(flag(delegate, '--workdir')).toBe(worktree)
    expect(check).toMatchObject({ args: ['-c', 'npm test'], command: 'sh', cwd: worktree, timeoutMs: 120_000 })
    expect(check?.env).toEqual({ AGENT_KIT_DELEGATION: '1' })
    expect(w.vibe.commands).toHaveLength(2)
    expect(stagedBeforeCheck).toEqual([1])
    expect(w.git.stagedIn).toEqual([worktree])
    expect(await readFile(join(runDirOf(w, result), 'check-output.txt'), 'utf8')).toBe('3 passed\n')
    expect(JSON.parse(await readFile(join(runDirOf(w, result), 'result.json'), 'utf8'))).toEqual(withoutAttempts(result))
    expect(await readdir(worktree)).toContain('src')
  })

  it('tells the delegate to edit inside its working directory and not to commit', async () => {
    const w = await readyRun({ scripts: [{ stdout: journal('x') }, { stdout: '' }] })

    const result = await w.run(writeInput(w))

    expect(await readFile(join(runDirOf(w, result), 'prompt.md'), 'utf8')).toBe(
      `read this\n\n${EDIT_RULE}\n\nEnd your answer with a line "SUMMARY:" followed by at most 1500 characters that summarise the result.`,
    )
    const read = await readyRun({ scripts: [{ stdout: journal('x') }] })
    expect(await readFile(join(runDirOf(read, await read.run(read.input())), 'prompt.md'), 'utf8')).not.toContain('Do not commit')
  })

  it('launches Vibe through the write agent, which edits files and runs no shell', async () => {
    const w = await readyRun({ scripts: [{ stdout: journal('x') }, { stdout: '' }] })

    await w.run(writeInput(w))

    expect(flag(w.vibe.commands[0], '--agent')).toBe('delegate-write')
    expect(w.vibe.commands[0]?.env).toMatchObject({ AGENT_KIT_DELEGATION: '1', VIBE_OUTSIDE_WORKDIR: 'deny' })
  })

  it('launches Claude Code with acceptEdits, Antigravity with accept-edits and OpenCode with its default agent', async () => {
    const w = await clientRun({
      scripts: [
        { stdout: claudeResult('x') },
        { stdout: '' },
        { stdout: agyResult('x') },
        { stdout: '' },
        { stdout: openCodeEvents(OPENCODE_STEP_START, openCodeText('x')) },
        { stdout: '' },
      ],
    })

    await w.run(writeInput(w, { client: 'claude', model: 'claude-sonnet-5-5' }))
    await w.run(writeInput(w, { client: 'agy', model: 'gemini-3.8-flash-high' }))
    await w.run(writeInput(w, { client: 'opencode', model: 'opencode-go/glm-5-3' }))

    const [claude, , agy, , opencode] = w.vibe.commands
    expect(claude?.args).toEqual([
      '-p',
      '--model',
      'claude-sonnet-5-5',
      '--output-format',
      'json',
      '--permission-mode',
      'acceptEdits',
      '--allowedTools',
      'Read,Grep,Glob,Edit,Write',
    ])
    expect(claude?.stdin).toContain(EDIT_RULE)
    expect(flag(agy, '--mode')).toBe('accept-edits')
    expect(agy?.args[1]).toContain(EDIT_RULE)
    expect(opencode?.args).toEqual(['run', '--model', 'opencode-go/glm-5-3', '--format', 'json', expect.stringContaining(EDIT_RULE) as unknown])
    for (const command of [claude, agy, opencode]) expect(String(command?.cwd)).toContain(join('.git', 'model-gateway', 'worktrees'))
  })

  it('works in its own worktree even when the model’s read-only proof passed', async () => {
    const w = await readyRun({ scripts: [{ stdout: journal('x') }, { stdout: '' }], setupResults: setupResults('pass') })

    const result = await w.run(writeInput(w))

    expect(w.vibe.commands[0]?.cwd).toBe(worktreeOf(result))
    expect(w.vibe.commands[0]?.cwd).not.toBe(w.repo)
  })

  it('reports a check that fails as a failed run, keeps its worktree, and still stages what the delegate wrote', async () => {
    const w = await readyRun({
      scripts: [{ effect: edits('src/a.ts'), stdout: journal('done') }, { exitCode: 1, stderr: '1 failed\n', stdout: 'running\n' }],
    })

    const result = await w.run(writeInput(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({
      check: { command: 'npm test', exit_code: 1 },
      reason: 'check-failed',
      staged_files: ['src/a.ts'],
      status: 'failed',
    })
    expect(await readFile(join(runDirOf(w, result), 'check-output.txt'), 'utf8')).toBe('running\n1 failed\n')
    expect(await readdir(worktreeOf(result))).toContain('src')
    expect(w.git.worktrees.map((call) => call.action)).toEqual(['create'])
    expect(JSON.parse((await readFile(w.ledger, 'utf8')).trimEnd())).toMatchObject({ mode: 'write', outcome: 'failed', reason: 'check-failed' })
  })

  it('reports a check that cannot start as a failed check with exit code 127 and the reason as its output', async () => {
    const w = await readyRun({ scripts: [{ stdout: journal('done') }] })

    const result = await w.run(writeInput(w))

    expect(printed(result)).toMatchObject({ check: { exit_code: 127 }, reason: 'check-failed', status: 'failed' })
    expect(await readFile(join(runDirOf(w, result), 'check-output.txt'), 'utf8')).toBe('unexpected process launch')
  })

  it('reports a check that outlives its time limit as a failed check', async () => {
    const w = await readyRun({ scripts: [{ stdout: journal('done') }, { stdout: '', timedOut: true }] })

    expect(printed(await w.run(writeInput(w)))).toMatchObject({ check: { exit_code: 124 }, reason: 'check-failed', status: 'failed' })
  })

  it('leaves the worktree of a run that hit its time limit unstaged and kept, runs no check, and lists it for the workflow’s end', async () => {
    const w = await readyRun({ scripts: [{ effect: edits('src/partial.ts'), stdout: '', timedOut: true }] })

    const result = await w.run(writeInput(w))

    expect(printed(result)).toMatchObject({ reason: 'timeout', staged_files: [], status: 'failed' })
    expect(printed(result)).not.toHaveProperty('check')
    expect(w.vibe.commands).toHaveLength(1)
    expect(w.git.stagedIn).toEqual([])
    expect(await readdir(worktreeOf(result))).toContain('src')
    expect(w.git.worktrees.map((call) => call.action)).toEqual(['create'])
    expect(JSON.parse(await readFile(w.kept, 'utf8'))).toEqual({
      version: 1,
      worktrees: [
        {
          at: expect.any(String) as unknown,
          branch: printed(result).branch,
          path: worktreeOf(result),
          repo: w.repo,
          run_id: printed(result).run_id,
          workflow: 'spec-293',
        },
      ],
    })
  })

  it('leaves the worktree of any run the delegate failed unstaged and kept, with the delegate’s own reason', async () => {
    const w = await readyRun({ scripts: [{ exitCode: 3, stdout: '' }, { stdout: 'not a journal' }] })

    expect(printed(await w.run(writeInput(w)))).toMatchObject({ reason: 'exit-3', staged_files: [], status: 'failed' })
    expect(printed(await w.run(writeInput(w)))).toMatchObject({ reason: 'parse-error', staged_files: [], status: 'failed' })
    expect(w.vibe.commands).toHaveLength(2)
    expect(w.git.stagedIn).toEqual([])
  })

  it('stops the run when git cannot create the worktree off the base, and leaves nothing behind', async () => {
    const w = await readyRun({
      repositories: (repo) => ({
        [repo]: { commonDir: join(repo, '.git'), remotes: [AGENT_KIT_REMOTE], root: repo, worktreeError: 'fatal: invalid reference: nowhere' },
      }),
      scripts: [{ stdout: journal('x') }],
    })

    const result = await w.run(writeInput(w, { base: 'nowhere' }))

    expect(printed(result)).toEqual({
      code: 'worktree-failed',
      message: `${w.repo}: cannot create a worktree off nowhere: fatal: invalid reference: nowhere`,
      status: 'error',
    })
    expect(w.vibe.commands).toEqual([])
    await expect(readFile(w.kept, 'utf8')).rejects.toThrow()
    expect(await readdir(join(w.repo, '.git', 'model-gateway')).catch(() => [])).not.toContain('worktrees')
  })

  it('stages the delegate’s work before the check, so what the check writes is not the delegate’s diff', async () => {
    const callsAtCheck: string[][] = []
    const w = await readyRun({
      scripts: [
        { effect: edits('src/a.ts'), stdout: journal('done') },
        {
          effect: async (command) => {
            callsAtCheck.push([...w.git.calls])
            await mkdir(join(String(command.cwd), 'coverage'), { recursive: true })
            await writeFile(join(String(command.cwd), 'coverage', 'lcov.info'), 'generated by the check')
            await writeFile(join(String(command.cwd), 'src', 'a.ts'), 'rewritten by the check')
          },
          stdout: '3 passed\n',
        },
      ],
    })

    const result = await w.run(writeInput(w))

    expect(printed(result)).toMatchObject({ staged_files: ['src/a.ts'], status: 'ok' })
    expect(callsAtCheck[0]?.slice(-2)).toEqual(['stageAll', 'stagedFiles'])
    expect(w.git.calls.filter((call) => call === 'stageAll')).toHaveLength(1)
    expect(w.git.calls.filter((call) => call === 'stagedFiles')).toHaveLength(1)
    expect(await readFile(join(worktreeOf(result), 'src', 'a.ts'), 'utf8')).toBe('rewritten by the check')
    expect(await readdir(worktreeOf(result))).toContain('coverage')
  })

  it('reports a failed run when git cannot list what it staged, and keeps the worktree', async () => {
    const w = await readyRun({ scripts: [{ effect: edits('src/a.ts'), stdout: journal('done') }, { stdout: '3 passed\n' }] })
    w.git.stagedFilesError = 'fatal: index file smaller than expected'

    const result = await w.run(writeInput(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({
      check: { command: 'npm test', exit_code: 0 },
      reason: 'stage-list-failed',
      staged_files: null,
      status: 'failed',
      worktree_error: 'fatal: index file smaller than expected',
    })
    expect(w.git.stagedIn).toEqual([worktreeOf(result)])
    expect(await readdir(worktreeOf(result))).toContain('src')
    expect(JSON.parse((await readFile(w.ledger, 'utf8')).trimEnd())).toMatchObject({ outcome: 'failed', reason: 'stage-list-failed' })
    expect(JSON.parse(await readFile(w.kept, 'utf8'))).toMatchObject({ worktrees: [{ run_id: printed(result).run_id }] })
  })

  it('reports a failed run, not an error, when git cannot stage after a delegate and a check that went well', async () => {
    const w = await readyRun({ scripts: [{ effect: edits('src/a.ts'), stdout: journal('done') }, { stdout: '3 passed\n' }] })
    w.git.stageError = 'fatal: not a git repository: .git'

    const result = await w.run(writeInput(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({
      branch: expect.stringMatching(/^gateway\//) as unknown,
      check: { command: 'npm test', exit_code: 0 },
      reason: 'stage-failed',
      staged_files: [],
      status: 'failed',
      worktree_error: 'fatal: not a git repository: .git',
    })
    expect(worktreeOf(result)).toContain(join('.git', 'model-gateway', 'worktrees'))
    expect(await readFile(join(runDirOf(w, result), 'output.json'), 'utf8')).toBe(journal('done'))
    expect(JSON.parse(await readFile(join(runDirOf(w, result), 'result.json'), 'utf8'))).toEqual(withoutAttempts(result))
    expect(JSON.parse((await readFile(w.ledger, 'utf8')).trimEnd())).toMatchObject({ mode: 'write', outcome: 'failed', reason: 'stage-failed' })
    expect(await readdir(worktreeOf(result))).toContain('src')
    expect(JSON.parse(await readFile(w.kept, 'utf8'))).toMatchObject({ worktrees: [{ run_id: printed(result).run_id }] })
  })

  it('reports a failed run when the check’s output cannot be written, and keeps the check’s exit code', async () => {
    const w = await readyRun({
      scripts: [
        {
          effect: async (command) => {
            await mkdir(join(dirname(String(flag(command, '--prompt-file'))), 'check-output.txt'), { recursive: true })
          },
          stdout: journal('done'),
        },
        { stdout: '3 passed\n' },
      ],
    })

    const result = await w.run(writeInput(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({
      check: { exit_code: 0 },
      reason: 'check-output-failed',
      status: 'failed',
      worktree_error: expect.stringContaining('check-output.txt') as unknown,
    })
    expect(JSON.parse((await readFile(w.ledger, 'utf8')).trimEnd())).toMatchObject({ outcome: 'failed', reason: 'check-output-failed' })
  })

  it('removes the worktree and its branch, and throws the original error, when the kept list cannot take it', async () => {
    const kept = new FakeKept()
    kept.addError = new Error('disk full')
    const w = await readyRun({ kept, scripts: [{ stdout: journal('x') }] })

    const result = await w.run(writeInput(w))

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toEqual({ code: 'unexpected', message: 'disk full', status: 'error' })
    const [created, removed] = w.git.worktrees
    expect(w.git.worktrees.map((call) => call.action)).toEqual(['create', 'remove'])
    expect(removed?.path).toBe(created?.path)
    expect(w.git.deletedBranches).toEqual([{ branch: created?.branch, repo: w.repo }])
    expect(w.vibe.commands).toEqual([])
  })

  it('throws the original error even when the worktree and the branch cannot be removed either', async () => {
    const kept = new FakeKept()
    kept.addError = new Error('disk full')
    const w = await readyRun({
      kept,
      repositories: (repo) => ({
        [repo]: { commonDir: join(repo, '.git'), remotes: [AGENT_KIT_REMOTE], removeError: 'fatal: cannot lock', root: repo },
      }),
    })
    w.git.deleteBranchError = 'error: branch not found'

    expect(printed(await w.run(writeInput(w)))).toEqual({ code: 'unexpected', message: 'disk full', status: 'error' })
    expect(w.git.worktrees.map((call) => call.action)).toEqual(['create', 'remove'])
  })

  it('never commits, merges or pushes: the only git it makes is finding, creating, staging, listing and removing', async () => {
    const w = await readyRun({
      scripts: [
        { effect: edits('a.ts'), stdout: journal('x') },
        { stdout: '' },
        { effect: edits('b.ts'), stdout: journal('y') },
        { exitCode: 1, stdout: '' },
        { stdout: '', timedOut: true },
      ],
    })

    await w.run(writeInput(w))
    await w.run(writeInput(w))
    await w.run(writeInput(w))
    await w.cli('worktrees', 'clean', '--json', JSON.stringify({ all: true }))

    expect(w.git.calls.length).toBeGreaterThan(0)
    for (const call of w.git.calls) expect(ALLOWED_GIT_CALLS.has(call)).toBe(true)
    expect(w.git.calls.filter((call) => /commit|merge|push/i.test(call))).toEqual([])
    expect(w.git.calls).toContain('stageAll')
  })

  it('lists every worktree it made, whatever the run came to, so the workflow’s end can remove them', async () => {
    const w = await readyRun({ scripts: [{ stdout: journal('x') }, { stdout: '' }] })

    const result = await w.run(writeInput(w))

    expect(JSON.parse(await readFile(w.kept, 'utf8'))).toMatchObject({ worktrees: [{ run_id: printed(result).run_id, workflow: 'spec-293' }] })
  })

  it('keeps the list when two runs add to it at once', async () => {
    const w = await readyRun({ scripts: Array.from({ length: 6 }, () => ({ stdout: journal('x') })) })
    w.vibe.queue(...Array.from({ length: 6 }, () => ({ stdout: '' })))

    const results = await Promise.all(Array.from({ length: 6 }, () => w.run(writeInput(w))))

    const listed = (JSON.parse(await readFile(w.kept, 'utf8')) as { worktrees: { run_id: string }[] }).worktrees.map((entry) => entry.run_id)
    expect(listed.sort()).toEqual(results.map((result) => String(printed(result).run_id)).sort())
  })
})

describe('model-gateway worktrees clean', () => {
  /** A workspace with one finished writing run of workflow `spec-293` and one that timed out in `spec-294`. */
  async function withKept(): Promise<{ w: RunWorkspace; ok: ModelGatewayCliResult; timedOut: ModelGatewayCliResult }> {
    const w = await readyRun({ scripts: [{ stdout: journal('x') }, { stdout: '' }, { stdout: '', timedOut: true }] })
    const ok = await w.run(writeInput(w))
    const timedOut = await w.run(writeInput(w, { workflow: 'spec-294' }))
    return { ok, timedOut, w }
  }

  it('removes the worktrees of the workflow that ended, and their branches, and keeps the others listed', async () => {
    const { w, ok, timedOut } = await withKept()
    w.git.worktrees.length = 0

    const result = await w.cli('worktrees', 'clean', '--json', '{"workflow":"spec-294"}')

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toEqual({
      branch_kept: [],
      failed: [],
      removed: [{ branch: printed(timedOut).branch, run_id: printed(timedOut).run_id, workflow: 'spec-294', worktree: worktreeOf(timedOut) }],
      status: 'ok',
    })
    expect(w.git.worktrees).toEqual([{ action: 'remove', path: worktreeOf(timedOut), repo: w.repo }])
    expect(w.git.deletedBranches).toEqual([{ branch: printed(timedOut).branch, repo: w.repo }])
    expect(await readdir(dirname(worktreeOf(timedOut)))).not.toContain(basename(worktreeOf(timedOut)))
    expect((JSON.parse(await readFile(w.kept, 'utf8')) as { worktrees: { run_id: string }[] }).worktrees.map((entry) => entry.run_id)).toEqual([
      printed(ok).run_id,
    ])
  })

  it('counts a worktree as cleaned once it is removed, and reports a branch git would not delete', async () => {
    const { w, timedOut } = await withKept()
    w.git.deleteBranchError = 'error: Cannot delete branch checked out at /elsewhere'

    const result = await w.cli('worktrees', 'clean', '--json', '{"workflow":"spec-294"}')

    expect(printed(result)).toEqual({
      branch_kept: [
        { branch: printed(timedOut).branch, reason: 'error: Cannot delete branch checked out at /elsewhere', run_id: printed(timedOut).run_id },
      ],
      failed: [],
      removed: [{ branch: printed(timedOut).branch, run_id: printed(timedOut).run_id, workflow: 'spec-294', worktree: worktreeOf(timedOut) }],
      status: 'ok',
    })
    expect((JSON.parse(await readFile(w.kept, 'utf8')) as { worktrees: { workflow: string }[] }).worktrees.map((entry) => entry.workflow)).toEqual(['spec-293'])
  })

  it('removes every kept worktree when told to, and leaves an empty list', async () => {
    const { w } = await withKept()

    const result = await w.cli('worktrees', 'clean', '--json', '{"all":true}')

    expect((printed(result).removed as unknown[]).length).toBe(2)
    expect(JSON.parse(await readFile(w.kept, 'utf8'))).toEqual({ version: 1, worktrees: [] })
  })

  it('removes nothing for a workflow it kept nothing for, and when it has kept nothing at all', async () => {
    const { w } = await withKept()
    expect(printed(await w.cli('worktrees', 'clean', '--json', '{"workflow":"other"}'))).toEqual({ branch_kept: [], failed: [], removed: [], status: 'ok' })
    expect(w.git.worktrees.map((call) => call.action)).toEqual(['create', 'create'])

    const empty = await runWorkspace()
    expect(printed(await empty.cli('worktrees', 'clean', '--json', '{"all":true}'))).toEqual({ branch_kept: [], failed: [], removed: [], status: 'ok' })
  })

  it('refuses input that names both a workflow and all', async () => {
    const w = await runWorkspace()

    const result = await w.cli('worktrees', 'clean', '--json', '{"workflow":"spec-294","all":true}')

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'invalid-input', status: 'error' })
    expect(String(printed(result).message)).toContain('not both')
  })

  it('keeps the entry of a worktree it could not remove, and says why', async () => {
    const w = await readyRun({
      repositories: (repo) => ({
        [repo]: { commonDir: join(repo, '.git'), remotes: [AGENT_KIT_REMOTE], removeError: 'fatal: cannot lock', root: repo },
      }),
      scripts: [{ stdout: journal('x') }, { stdout: '' }],
    })
    const ran = await w.run(writeInput(w))

    const result = await w.cli('worktrees', 'clean', '--json', '{"workflow":"spec-293"}')

    expect(printed(result)).toEqual({
      branch_kept: [],
      failed: [{ reason: 'fatal: cannot lock', run_id: printed(ran).run_id, worktree: worktreeOf(ran) }],
      removed: [],
      status: 'ok',
    })
    expect(w.git.deletedBranches).toEqual([])
    expect((JSON.parse(await readFile(w.kept, 'utf8')) as { worktrees: unknown[] }).worktrees).toHaveLength(1)
  })

  it('refuses input that names neither a workflow nor all, or both', async () => {
    const w = await readyRun()

    for (const input of ['{}', '{"workflow":1}', '{"workflow":""}', '{"all":false}', '{"all":true,"workflow":"x"}']) {
      const result = await w.cli('worktrees', 'clean', '--json', input)
      expect(result.exitCode).toBe(1)
      expect(printed(result)).toMatchObject({ code: 'invalid-input', status: 'error' })
    }
  })

  it('names a list it cannot read', async () => {
    const w = await readyRun()
    await mkdir(dirname(w.kept), { recursive: true })
    await writeFile(w.kept, 'not json')

    const result = await w.cli('worktrees', 'clean', '--json', '{"all":true}')

    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.kept)
    await writeFile(w.kept, '{"version":1,"worktrees":"x"}')
    expect(printed(await w.cli('worktrees', 'clean', '--json', '{"all":true}')).code).toBe('invalid-config')
  })

  it('is a subcommand of its own: worktrees alone, or with another word, is unknown', async () => {
    const w = await readyRun()

    expect(printed(await w.cli('worktrees', '--json', '{}'))).toMatchObject({ code: 'unknown-command', status: 'error' })
    expect(printed(await w.cli('worktrees', 'list', '--json', '{}'))).toMatchObject({ code: 'unknown-command', status: 'error' })
    expect(printed(await w.cli('worktrees'))).toMatchObject({ code: 'unknown-command', status: 'error' })
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

    const result = await w.cli('workflow', 'start', '--json', '{"name":"spec-293"}')

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

    const result = await w.cli('workflow', 'start', '--json', '{"name":"spec-293"}')

    expect(printed(result)).toMatchObject({ workflow: { started_at: '2026-11-04T10:00:00.000Z', state: 'running' } })
  })

  it('ends a workflow and reports when it ended', async () => {
    const w = await runWorkspace()
    await w.writeWorkflows({ version: 1, workflows: [{ name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }] })

    const result = await w.cli('workflow', 'end', '--json', '{"name":"spec-293"}')

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

    const result = await w.cli('workflow', 'list', '--json', '{}')

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

    expect(printed(await w.cli('workflow', 'list', '--json', '{}'))).toEqual({ status: 'ok', workflows: [] })
  })

  it('refuses a workflow command that names no workflow', async () => {
    const w = await runWorkspace()

    expect(printed(await w.cli('workflow', 'start', '--json', '{}'))).toEqual({
      code: 'invalid-input',
      message: 'a workflow command must name its workflow',
      status: 'error',
    })
    expect(printed(await w.cli('workflow', 'end', '--json', '{"name":""}'))).toMatchObject({ code: 'invalid-input' })
  })

  it('refuses to end a workflow this machine never started', async () => {
    const w = await runWorkspace()

    expect(printed(await w.cli('workflow', 'end', '--json', '{"name":"nowhere"}'))).toEqual({
      code: 'unknown-workflow',
      message: 'no workflow named nowhere has started on this machine',
      status: 'error',
    })
  })

  it('refuses a workflow file it cannot read, naming the file', async () => {
    const w = await runWorkspace()
    await w.writeWorkflows({ version: 2, workflows: [] })

    const result = await w.cli('workflow', 'list', '--json', '{}')

    expect(printed(result)).toMatchObject({ code: 'invalid-config', status: 'error' })
    expect(String(printed(result).message)).toContain(w.workflows)
  })

  it('refuses a workflow subcommand it does not have', async () => {
    const w = await runWorkspace()

    expect(printed(await w.cli('workflow', 'pause', '--json', '{}'))).toMatchObject({ code: 'unknown-command' })
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

  it('writes no ledger line for a subsidised run whose workflow cannot be started, which reserved nothing', async () => {
    const workflows = new FakeWorkflows()
    workflows.updateError = new ModelGatewayError(
      'workflow-ended',
      'workflow spec-293 ended at 2026-11-04T11:00:00.000Z: a run may not join it, so start a new workflow',
    )
    const w = await readyRun({ workflows })

    expect(printed(await w.run(w.input()))).toMatchObject({ code: 'workflow-ended' })
    expect(ledgerLines(await readFile(w.ledger, 'utf8').catch(() => ''))).toEqual([])
  })

  it('starts the workflow of a run with no passing read-only proof, which is not a refusal', async () => {
    const { w } = await readyHermes({ setupResults: undefined })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({ status: 'ok' })
    expect(await listedWorkflows(w)).toMatchObject([{ name: 'spec-293', state: 'running' }])
  })
})

/** The workflows `workflow list` reports, as the gateway itself reads them. */
async function listedWorkflows(w: RunWorkspace): Promise<readonly unknown[]> {
  return (printed(await w.cli('workflow', 'list', '--json', '{}')) as { workflows: readonly unknown[] }).workflows
}

/** The machine file that lets Hermes run on Anthropic, with the price and tier a metered run is estimated at. */
const HERMES_MACHINE = {
  clients: { hermes: { providers: ['anthropic'] } },
  models: [
    {
      client: 'hermes',
      family: 'anthropic',
      model: 'anthropic/claude-sonnet-4.6',
      notes: 'Metered on the owner account.',
      prices: { cached_input: 0.3, input: 3, output: 15 },
      tier: 'routine',
    },
  ],
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

  it('has no read-only proof, so it runs in a throwaway worktree of the repository', async () => {
    const { hermes, w } = await readyHermes({ setupResults: undefined })

    expect(printed(await w.run(hermesInput(w))).status).toBe('ok')

    const [command] = hermes.commands
    expect(command?.cwd).not.toBe(w.repo)
    expect(String(command?.cwd).startsWith(tmpdir())).toBe(true)
    expect(w.git.worktrees).toEqual([
      { action: 'add', path: command?.cwd, repo: w.repo },
      { action: 'remove', path: command?.cwd, repo: w.repo },
    ])
  })

  it('launches hermes one-shot, in a throwaway worktree, with its model, provider and usage file', async () => {
    const { hermes, w } = await readyHermes()

    const result = await w.run(hermesInput(w, { time_limit_s: 120 }))

    expect(result.exitCode).toBe(0)
    const [command] = hermes.commands
    expect(command?.command).toBe('hermes')
    expect(command?.cwd).not.toBe(w.repo)
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
      attempts: [{ client: 'hermes', model: 'anthropic/claude-sonnet-4.6', run_id: printed(result).run_id, status: 'ok' }],
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

    const lines = ledgerLines(await readFile(w.ledger, 'utf8')).filter((line) => line.kind === undefined)
    expect(lines).toHaveLength(1)
    expect(lines[0]).toEqual({
      at: expect.any(String) as unknown,
      client: 'hermes',
      cost_class: 'metered-personal',
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

  it('counts the run at the catalogue price when the usage file prices nothing', async () => {
    const { w } = await readyHermes({ usage: { failed: false, input_tokens: 10, output_tokens: 5 } })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({ cost_usd: 0.000105, status: 'ok' })
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

/** One attempt, as a delegation's result lists it. */
interface AttemptRow {
  readonly run_id: string
  readonly client: string
  readonly model: string
  readonly status: string
  readonly reason?: string
}

function attemptsOf(result: ModelGatewayCliResult): AttemptRow[] {
  return printed(result).attempts as AttemptRow[]
}

/** The program each launched command runs: Vibe is launched behind `env -u ...`, which this looks through. */
function binariesOf(commands: readonly { readonly command: string; readonly args: readonly string[] }[]): string[] {
  return commands.map((launched) => {
    if (launched.command !== 'env') return launched.command
    let skip = false
    for (const arg of launched.args) {
      if (skip) {
        skip = false
      } else if (arg === '-u') {
        skip = true
      } else {
        return arg
      }
    }
    return launched.command
  })
}

/** The `<client>:<model>` of each attempt a delegation made, in order. */
function attemptedModels(result: ModelGatewayCliResult): string[] {
  return attemptsOf(result).map((attempt) => `${attempt.client}:${attempt.model}`)
}

/** The prompt file of one run of the test repository. */
async function promptOf(w: RunWorkspace, runId: string): Promise<string> {
  return readFile(join(w.repo, '.git', 'model-gateway', 'runs', runId, 'prompt.md'), 'utf8')
}

/** A delegation the escalation handles: the test input with the escalation left at its default. */
function delegation(w: RunWorkspace, overrides: Readonly<Record<string, unknown>> = {}): unknown {
  return w.input({ escalate: undefined, ...overrides })
}

const FAILS: VibeScript = { exitCode: 3, stderr: 'boom: the tests did not pass', stdout: '' }
const SUCCEEDS: VibeScript = { exportJson: exported({ cost: 0.01, input: 10 }), stdout: journal('fixed\nSUMMARY: done') }
const CLAUDE_SUCCEEDS: VibeScript = { stdout: claudeResult('fixed\nSUMMARY: done by claude') }
const ALL_FAMILIES = ['anthropic', 'deepseek', 'google', 'mistral', 'zhipu']

async function escalating(options: RunWorkspaceOptions = {}): Promise<RunWorkspace> {
  return runWorkspace({ allowlist: allowlist(ALL_FAMILIES, true, true), setupResults: setupResults('pass'), ...options })
}

describe('model-gateway run: retry', () => {
  it('returns a result that is fine at once, with one attempt and no link on its ledger line', async () => {
    const w = await escalating({ scripts: [SUCCEEDS] })

    const result = await w.run(delegation(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({ attempts: [{ client: 'vibe', model: 'glm-5-3', status: 'ok' }], status: 'ok' })
    expect(printed(result)).not.toHaveProperty('escalation')
    expect(w.vibe.commands).toHaveLength(1)
    const [line] = await ledgerOf(w, undefined)
    expect(line).not.toHaveProperty('attempt')
    expect(line).not.toHaveProperty('attempt_of')
  })

  it('retries a failed result once on the same client model, with the failure attached to the prompt', async () => {
    const w = await escalating({ scripts: [FAILS, SUCCEEDS] })

    const result = await w.run(delegation(w))

    expect(printed(result)).toMatchObject({ client: 'vibe', model: 'glm-5-3', status: 'ok' })
    expect(printed(result)).not.toHaveProperty('escalation')
    const [first, second] = attemptsOf(result)
    expect(first).toMatchObject({ client: 'vibe', model: 'glm-5-3', reason: 'exit-3', status: 'failed' })
    expect(second).toMatchObject({ client: 'vibe', model: 'glm-5-3', status: 'ok' })
    expect(printed(result).run_id).toBe(second?.run_id)
    expect(first?.run_id).not.toBe(second?.run_id)
    expect(w.vibe.commands).toHaveLength(2)
    expect(await promptOf(w, String(first?.run_id))).toBe(WRAPPED_PROMPT)
    expect(await promptOf(w, String(second?.run_id))).toBe(
      'read this\n\nA previous attempt failed: exit-3; boom: the tests did not pass\n\nEnd your answer with a line "SUMMARY:" followed by at most 1500 characters that summarise the result.',
    )
  })

  it('gives each attempt a ledger line of its own, linked to the first by attempt_of and its attempt number', async () => {
    const w = await escalating({ scripts: [FAILS, SUCCEEDS] })

    const result = await w.run(delegation(w))

    const [first, second] = attemptsOf(result)
    const lines = await ledgerOf(w, undefined)
    expect(lines.map((line) => line.run_id)).toEqual([first?.run_id, second?.run_id])
    expect(lines[0]).toMatchObject({ outcome: 'failed', reason: 'exit-3' })
    expect(lines[0]).not.toHaveProperty('attempt')
    expect(lines[1]).toMatchObject({ attempt: 2, attempt_of: first?.run_id, outcome: 'ok' })
    expect(printed(result)).toMatchObject({ attempt: 2, attempt_of: first?.run_id })
  })

  it('shows the retry the last 1,000 characters of what the failed attempt printed, its stderr first', async () => {
    const w = await escalating({ scripts: [{ exitCode: 1, stderr: `${'x'.repeat(1500)}THE END`, stdout: 'stdout is not shown' }, SUCCEEDS] })

    const result = await w.run(delegation(w))

    const prompt = await promptOf(w, String(attemptsOf(result)[1]?.run_id))
    const [, block = ''] = prompt.split('A previous attempt failed: exit-1; ')
    const shown = block.split('\n\nEnd your answer')[0]
    expect(shown).toHaveLength(1000)
    expect(shown?.endsWith('xxxTHE END')).toBe(true)
    expect(prompt).not.toContain('stdout is not shown')
  })

  it('shows the retry what the failed attempt printed on stdout when it printed nothing on stderr, and nothing when it printed nothing', async () => {
    const out = await escalating({ scripts: [{ stdout: 'not a journal' }, SUCCEEDS] })
    const withOutput = await out.run(delegation(out))
    expect(await promptOf(out, String(attemptsOf(withOutput)[1]?.run_id))).toContain('A previous attempt failed: parse-error; not a journal\n\n')

    const silent = await escalating({ scripts: [{ exitCode: 2, stdout: '' }, SUCCEEDS] })
    const withNothing = await silent.run(delegation(silent))
    expect(await promptOf(silent, String(attemptsOf(withNothing)[1]?.run_id))).toContain('A previous attempt failed: exit-2\n\n')
  })

  it('counts a timeout and output it cannot parse as failed results, and keeps the raw output with the reason', async () => {
    const w = await escalating({ scripts: [{ stdout: 'partial', timedOut: true }, { stdout: 'not a journal' }, CLAUDE_SUCCEEDS] })

    const result = await w.run(delegation(w))

    expect(attemptsOf(result).map((attempt) => attempt.reason)).toEqual(['timeout', 'parse-error', undefined])
    const [timedOut] = attemptsOf(result)
    const runDir = join(w.repo, '.git', 'model-gateway', 'runs', String(timedOut?.run_id))
    expect(await readFile(join(runDir, 'output.json'), 'utf8')).toBe('partial')
    expect(JSON.parse(await readFile(join(runDir, 'result.json'), 'utf8'))).toMatchObject({ reason: 'timeout', status: 'failed' })
  })

  it('retries a writing run whose check failed in a fresh worktree, showing what the check printed', async () => {
    const w = await escalating({
      scripts: [
        { effect: edits('src/a.ts'), stdout: journal('done') },
        { exitCode: 1, stdout: '1 failed: a.test.ts' },
        { effect: edits('src/a.ts'), stdout: journal('fixed\nSUMMARY: fixed it') },
        { stdout: '3 passed' },
      ],
    })

    const result = await w.run(writeInput(w, { escalate: undefined }))

    expect(printed(result)).toMatchObject({ check: { exit_code: 0 }, status: 'ok' })
    const [first, second] = attemptsOf(result)
    expect(first).toMatchObject({ reason: 'check-failed', status: 'failed' })
    const created = w.git.worktrees.filter((call) => call.action === 'create')
    expect(created).toHaveLength(2)
    expect(created[0]?.path).not.toBe(created[1]?.path)
    expect(worktreeOf(result)).toBe(created[1]?.path)
    expect(await promptOf(w, String(second?.run_id))).toContain('A previous attempt failed: check-failed; 1 failed: a.test.ts')
  })

  it('does not retry when the caller turns the escalation off, and says so with one attempt', async () => {
    const w = await escalating({ scripts: [FAILS, SUCCEEDS] })

    const result = await w.run(delegation(w, { escalate: false }))

    expect(printed(result)).toMatchObject({ attempts: [{ reason: 'exit-3', status: 'failed' }], reason: 'exit-3', status: 'failed' })
    expect(printed(result)).not.toHaveProperty('escalation')
    expect(w.vibe.commands).toHaveLength(1)
  })

  it('refuses a malformed escalate or max_attempts before anything is launched', async () => {
    const w = await escalating({ scripts: [] })

    expect(printed(await w.run(delegation(w, { escalate: 'no' })))).toMatchObject({
      code: 'invalid-input',
      message: 'escalate must be true or false',
    })
    expect(printed(await w.run(delegation(w, { max_attempts: 0 })))).toMatchObject({
      code: 'invalid-input',
      message: 'max_attempts must be a whole number above 0',
    })
    expect(printed(await w.run(delegation(w, { max_attempts: 1.5 })))).toMatchObject({
      code: 'invalid-input',
      message: 'max_attempts must be a whole number',
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('never retries a refusal: a run the gateway refuses is an error that stops the call before anything is launched', async () => {
    const w = await escalating({ allowlist: allowlist(['mistral'], true, true), scripts: [SUCCEEDS] })

    const result = await w.run(delegation(w))

    expect(result.exitCode).toBe(1)
    expect(printed(result)).toMatchObject({ code: 'policy-refused', status: 'error' })
    expect(w.vibe.commands).toEqual([])
  })

  it('stops with the last result when a later attempt is refused, naming the refusal', async () => {
    const w = await escalating({ scripts: [] })
    w.vibe.queue({
      effect: async () => {
        await w.cli('workflow', 'end', '--json', JSON.stringify({ name: 'spec-293' }))
      },
      exitCode: 3,
      stdout: '',
    })

    const result = await w.run(delegation(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({
      attempts: [{ reason: 'exit-3', status: 'failed' }],
      escalation: 'stopped',
      reason: 'exit-3',
      status: 'failed',
    })
    expect(String(printed(result).stopped_reason)).toMatch(/^workflow-ended: /)
  })

  it('lets a first attempt that is refused for any other reason propagate as the error it is', async () => {
    const w = await escalating({ scripts: [SUCCEEDS] })

    const result = await w.run(delegation(w, { model: 'unknown-model-1' }))

    expect(printed(result)).toMatchObject({ code: 'unknown-model', status: 'error' })
  })
})

describe('model-gateway run: climb', () => {
  it('climbs to the next stronger permitted subsidised model after a second failure', async () => {
    const w = await escalating({ scripts: [FAILS, FAILS, CLAUDE_SUCCEEDS] })

    const result = await w.run(delegation(w))

    expect(printed(result)).toMatchObject({ client: 'claude', model: 'claude-opus-5-5', status: 'ok' })
    expect(attemptedModels(result)).toEqual(['vibe:glm-5-3', 'vibe:glm-5-3', 'claude:claude-opus-5-5'])
    expect(binariesOf(w.vibe.commands)).toEqual(['vibe', 'vibe', 'claude'])
    const lines = await ledgerOf(w, undefined)
    expect(lines.map((line) => line.attempt)).toEqual([undefined, 2, 3])
    expect(lines[2]).toMatchObject({ attempt_of: attemptsOf(result)[0]?.run_id })
  })

  it('climbs by tier and then by catalogue order, within the same cost class', async () => {
    const w = await escalating({ scripts: [FAILS, FAILS, CLAUDE_SUCCEEDS] })

    const result = await w.run(delegation(w, { model: 'deepseek-v4-1-flash-1017' }))

    expect(attemptedModels(result)).toEqual([
      'vibe:deepseek-v4-1-flash-1017',
      'vibe:deepseek-v4-1-flash-1017',
      'claude:claude-sonnet-5-5',
    ])
  })

  it('climbs only through the models the repository’s allowlist permits', async () => {
    const w = await escalating({ allowlist: allowlist(['deepseek', 'mistral'], true, true), scripts: [FAILS, FAILS, SUCCEEDS] })

    const result = await w.run(delegation(w, { model: 'deepseek-v4-1-flash-1017' }))

    expect(attemptedModels(result)).toEqual([
      'vibe:deepseek-v4-1-flash-1017',
      'vibe:deepseek-v4-1-flash-1017',
      'vibe:mistral-large-latest',
    ])
  })

  it('returns the last result with escalation exhausted when no stronger model is left', async () => {
    const w = await escalating({ allowlist: allowlist(['zhipu'], true, true), scripts: [FAILS, FAILS] })

    const result = await w.run(delegation(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({ escalation: 'exhausted', reason: 'exit-3', status: 'failed' })
    expect(attemptedModels(result)).toEqual(['vibe:glm-5-3', 'vibe:glm-5-3'])
    expect(w.vibe.commands).toHaveLength(2)
  })

  it('stops at max_attempts: one attempt is no retry, two are a retry and no climb', async () => {
    const one = await escalating({ scripts: [FAILS, SUCCEEDS] })
    const oneResult = await one.run(delegation(one, { max_attempts: 1 }))
    expect(printed(oneResult)).toMatchObject({ escalation: 'exhausted', status: 'failed' })
    expect(attemptsOf(oneResult)).toHaveLength(1)

    const two = await escalating({ scripts: [FAILS, FAILS, SUCCEEDS] })
    const twoResult = await two.run(delegation(two, { max_attempts: 2 }))
    expect(printed(twoResult)).toMatchObject({ escalation: 'exhausted', status: 'failed' })
    expect(attemptsOf(twoResult)).toHaveLength(2)

    const five = await escalating({ scripts: [FAILS, FAILS, FAILS, FAILS, SUCCEEDS] })
    const fiveResult = await five.run(delegation(five, { max_attempts: 5 }))
    expect(printed(fiveResult)).toMatchObject({ escalation: 'exhausted', status: 'failed' })
    expect(attemptedModels(fiveResult)).toEqual(['vibe:glm-5-3', 'vibe:glm-5-3', 'claude:claude-opus-5-5', 'claude:claude-opus-5-5'])
  })

  it('never climbs into a costlier cost class, a metered-employer one included', async () => {
    for (const costClass of ['metered-personal', 'metered-employer']) {
      const w = await escalating({
        allowlist: allowlist(['deepseek', 'google'], true, true),
        override: {
          clients: { agy: { cost_class: costClass } },
          models: [{ client: 'hermes', family: 'google', model: 'google/gemini-5', notes: 'x', tier: 'frontier' }],
        },
        scripts: [FAILS, FAILS, SUCCEEDS],
      })

      const result = await w.run(delegation(w, { model: 'deepseek-v4-1-flash-1017' }))

      expect(printed(result)).toMatchObject({ escalation: 'exhausted', status: 'failed' })
      expect(attemptedModels(result)).toEqual(['vibe:deepseek-v4-1-flash-1017', 'vibe:deepseek-v4-1-flash-1017'])
      expect(binariesOf(w.vibe.commands)).toEqual(['vibe', 'vibe'])
    }
  })

  it('may climb into a cheaper cost class, which is never a costlier one', async () => {
    const w = await escalating({
      allowlist: allowlist(['anthropic', 'google'], true, true),
      override: {
        clients: { agy: { cost_class: 'metered-personal' } },
        models: [{ client: 'agy', model: 'gemini-3.8-flash-high', prices: { cached_input: 0, input: 1, output: 2 } }],
      },
      scripts: [FAILS, FAILS, CLAUDE_SUCCEEDS],
      setupResults: AGY_PROVEN,
    })

    const result = await w.run(delegation(w, { client: 'agy', model: 'gemini-3.8-flash-high' }))

    expect(attemptedModels(result)).toEqual(['agy:gemini-3.8-flash-high', 'agy:gemini-3.8-flash-high', 'claude:claude-opus-5-5'])
  })
})

const TODAY = '2026-11-04T09:00:00.000Z'

/** A plan that allows one run a day, which a ledger line of today fills. */
function oneRunADay(client: string): unknown {
  return { clients: { [client]: { plan_limits: [{ limit_runs: 1, window: 'day' }] } } }
}

const RATE_LIMITED: VibeScript = { exitCode: 1, stderr: '429 Too Many Requests', stdout: '' }
const EXITS_1: VibeScript = { exitCode: 1, stderr: 'boom: the tests did not pass', stdout: '' }

describe('model-gateway run: fall-through', () => {
  it('swaps a client whose plan window is spent for another permitted client model of its cost class that covers the tier', async () => {
    const w = await escalating({ override: oneRunADay('vibe'), scripts: [CLAUDE_SUCCEEDS] })
    await w.writeLedger([ledgerRecord(TODAY, 'vibe')])

    const result = await w.run(delegation(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({ client: 'claude', model: 'claude-sonnet-5-5', status: 'ok' })
    expect(attemptedModels(result)).toEqual(['claude:claude-sonnet-5-5'])
    expect(binariesOf(w.vibe.commands)).toEqual(['claude'])
    expect(printed(result).skipped).toEqual([
      { client: 'vibe', model: 'glm-5-3', reason: 'unable_to_start: at 100% of its day plan limit (1 of 1 runs)' },
    ])
    expect(printed(result)).not.toHaveProperty('attempt')
  })

  it('swaps a client whose last run was refused for its credentials, and runs the swapped model with the caller’s own prompt', async () => {
    const w = await escalating({ scripts: [CLAUDE_SUCCEEDS] })
    await w.writeLedger([ledgerRecord('2026-11-04T10:30:00.000Z', 'vibe', { outcome: 'failed', reason: 'auth' })])

    const result = await w.run(delegation(w))

    expect(attemptedModels(result)).toEqual(['claude:claude-sonnet-5-5'])
    expect((printed(result).skipped as { reason: string }[])[0]?.reason).toMatch(/^unavailable: auth at /)
    expect(w.vibe.commands[0]?.stdin).toBe(WRAPPED_PROMPT)
  })

  it('swaps a client whose launch failed before the client ran, and does not count the swap as a retry', async () => {
    const w = await escalating({
      scripts: [{ effect: () => Promise.reject(new Error('spawn vibe ENOENT')), stdout: '' }, CLAUDE_SUCCEEDS],
    })

    const result = await w.run(delegation(w))

    expect(printed(result)).toMatchObject({ attempt: 2, client: 'claude', model: 'claude-sonnet-5-5', status: 'ok' })
    expect(attemptsOf(result)).toMatchObject([
      { client: 'vibe', reason: 'launch-failed', status: 'failed' },
      { client: 'claude', status: 'ok' },
    ])
    expect(printed(result).skipped).toEqual([{ client: 'vibe', model: 'glm-5-3', reason: 'launch-failed' }])
    expect(binariesOf(w.vibe.commands)).toEqual(['vibe', 'claude'])
  })

  it('swaps a client that answers rate-limited rather than retrying it, and keeps the failed run in the attempts', async () => {
    const w = await escalating({ scripts: [RATE_LIMITED, CLAUDE_SUCCEEDS] })

    const result = await w.run(delegation(w))

    expect(attemptsOf(result)).toMatchObject([
      { client: 'vibe', reason: 'rate-limited', status: 'failed' },
      { client: 'claude', model: 'claude-sonnet-5-5', status: 'ok' },
    ])
    expect(binariesOf(w.vibe.commands)).toEqual(['vibe', 'claude'])
  })

  it('reports the delegation unavailable, with the reasons per candidate, when no model of the cost class can start', async () => {
    const w = await escalating({ allowlist: allowlist(['zhipu'], true, true), override: oneRunADay('vibe'), scripts: [SUCCEEDS] })
    await w.writeLedger([ledgerRecord(TODAY, 'vibe')])

    const result = await w.run(delegation(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toEqual({
      attempts: [],
      client: 'vibe',
      message:
        'vibe:glm-5-3 cannot start, and no other permitted subsidised client model covers its tier; the gateway never moves into a costlier cost class by itself',
      model: 'glm-5-3',
      reasons: [{ client: 'vibe', model: 'glm-5-3', reason: 'unable_to_start: at 100% of its day plan limit (1 of 1 runs)' }],
      status: 'unavailable',
      workflow: 'spec-293',
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('never swaps into a costlier cost class, however strong the model there is', async () => {
    const w = await escalating({
      allowlist: allowlist(['anthropic', 'zhipu'], true, true),
      override: { clients: { claude: { cost_class: 'metered-personal' }, vibe: { plan_limits: [{ limit_runs: 1, window: 'day' }] } } },
      scripts: [CLAUDE_SUCCEEDS],
    })
    await w.writeLedger([ledgerRecord(TODAY, 'vibe')])

    const result = await w.run(delegation(w))

    expect(printed(result)).toMatchObject({ status: 'unavailable' })
    expect(w.vibe.commands).toEqual([])
  })

  it('never swaps into a model below the tier the caller asked for', async () => {
    const w = await escalating({ allowlist: allowlist(['deepseek', 'zhipu'], true, true), override: oneRunADay('vibe'), scripts: [SUCCEEDS] })
    await w.writeLedger([ledgerRecord(TODAY, 'vibe')])

    const result = await w.run(delegation(w))

    expect(printed(result)).toMatchObject({ status: 'unavailable' })
    expect(w.vibe.commands).toEqual([])
  })

  it('reports the delegation unavailable, with the failed run in its attempts, when the client that failed has no stand-in', async () => {
    const w = await escalating({ allowlist: allowlist(['zhipu'], true, true), scripts: [RATE_LIMITED, SUCCEEDS] })

    const result = await w.run(delegation(w))

    expect(printed(result)).toMatchObject({
      attempts: [{ client: 'vibe', reason: 'rate-limited', status: 'failed' }],
      reasons: [{ client: 'vibe', model: 'glm-5-3', reason: 'rate-limited' }],
      status: 'unavailable',
    })
    expect(w.vibe.commands).toHaveLength(1)
  })

  it('returns the run it really made when the retry cannot start and no model can stand in', async () => {
    const w = await escalating({ allowlist: allowlist(['zhipu'], true, true), scripts: [EXITS_1, RATE_LIMITED] })

    const result = await w.run(delegation(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({
      attempts: [
        { client: 'vibe', reason: 'exit-1', status: 'failed' },
        { client: 'vibe', reason: 'rate-limited', status: 'failed' },
      ],
      cost_usd: 0,
      escalation: 'stopped',
      output_path: join(runDirOf(w, result), 'output.json'),
      reason: 'exit-1',
      status: 'failed',
      stopped_reason: 'rate-limited',
    })
    expect(w.vibe.commands).toHaveLength(2)
  })

  it('returns the run it really made when the retry’s client cannot start at all, naming why', async () => {
    const w = await escalating({ allowlist: allowlist(['zhipu'], true, true), override: oneRunADay('vibe'), scripts: [EXITS_1] })

    const result = await w.run(delegation(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({
      attempts: [{ client: 'vibe', reason: 'exit-1', status: 'failed' }],
      escalation: 'stopped',
      output_path: join(runDirOf(w, result), 'output.json'),
      reason: 'exit-1',
      status: 'failed',
      stopped_reason: 'unable_to_start: at 100% of its day plan limit (1 of 1 runs)',
    })
    expect(w.vibe.commands).toHaveLength(1)
  })

  it('in a nedap repository, never swaps from Claude Code to Vibe or Antigravity', async () => {
    const w = await escalating({
      override: oneRunADay('claude'),
      repositories: (repo) => ({
        [repo]: { commonDir: join(repo, '.git'), remotes: [{ name: 'origin', url: 'git@github.com:nedap/agent-kit.git' }], root: repo },
      }),
      scripts: [SUCCEEDS],
    })
    await w.writeLedger([ledgerRecord(TODAY, 'claude')])

    const result = await w.run(delegation(w, { client: 'claude', model: 'claude-sonnet-5-5' }))

    expect(printed(result)).toMatchObject({
      reasons: [
        { client: 'claude', model: 'claude-sonnet-5-5' },
        { client: 'claude', model: 'claude-opus-5-5' },
      ],
      status: 'unavailable',
    })
    expect(w.vibe.commands).toEqual([])
  })

  it('does not look for a client that can start when the caller turns the escalation off', async () => {
    const w = await escalating({ override: oneRunADay('vibe'), scripts: [SUCCEEDS] })
    await w.writeLedger([ledgerRecord(TODAY, 'vibe')])

    const result = await w.run(delegation(w, { escalate: false }))

    expect(printed(result)).toMatchObject({ client: 'vibe', status: 'ok' })
  })

  it('passes over a stronger model that cannot start when it climbs, and says so', async () => {
    const w = await escalating({ override: oneRunADay('claude'), scripts: [FAILS, FAILS, SUCCEEDS] })
    await w.writeLedger([ledgerRecord(TODAY, 'claude')])

    const result = await w.run(delegation(w, { model: 'deepseek-v4-1-flash-1017' }))

    expect(attemptedModels(result)).toEqual([
      'vibe:deepseek-v4-1-flash-1017',
      'vibe:deepseek-v4-1-flash-1017',
      'vibe:glm-5-3',
    ])
    expect(printed(result).skipped).toMatchObject([{ client: 'claude', model: 'claude-sonnet-5-5' }])
  })

  it('returns the last result exhausted when every stronger model it could climb to cannot start', async () => {
    const w = await escalating({
      allowlist: allowlist(['anthropic', 'zhipu'], true, true),
      override: oneRunADay('claude'),
      scripts: [FAILS, FAILS, CLAUDE_SUCCEEDS],
    })
    await w.writeLedger([ledgerRecord(TODAY, 'claude')])

    const result = await w.run(delegation(w))

    expect(printed(result)).toMatchObject({ escalation: 'exhausted', reason: 'exit-3', status: 'failed' })
    expect(printed(result).skipped).toMatchObject([{ client: 'claude', model: 'claude-opus-5-5' }])
    expect(attemptsOf(result)).toHaveLength(2)
  })

  it('swaps into an OpenCode model on a provider the machine allows, and passes over one it does not', async () => {
    const w = await escalating({
      allowlist: allowlist(['zhipu'], true, true),
      override: {
        clients: { vibe: { plan_limits: [{ limit_runs: 1, window: 'day' }] } },
        models: [
          { client: 'opencode', family: 'zhipu', model: 'other/glm-5.3', notes: 'x', tier: 'hard' },
          { client: 'opencode', family: 'zhipu', model: 'opencode-go/glm-5.3', notes: 'x', tier: 'hard' },
        ],
      },
      scripts: [{ stdout: openCodeEvents(OPENCODE_STEP_START, openCodeText('fine\nSUMMARY: ok'), openCodeStepFinish(0.01, { input: 10, output: 5 })) }],
    })
    await w.writeLedger([ledgerRecord(TODAY, 'vibe')])

    const result = await w.run(delegation(w))

    expect(attemptedModels(result)).toEqual(['opencode:opencode-go/glm-5.3'])
    expect(printed(result)).toMatchObject({ status: 'ok' })
  })
})

describe('model-gateway run: review fixes', () => {
  it('never climbs to a model the machine disabled, by its alias or by a name it is known by', async () => {
    const w = await escalating({ override: { disabled: ['claude:claude-opus-5-5'] }, scripts: [FAILS, FAILS, CLAUDE_SUCCEEDS] })

    const result = await w.run(delegation(w))

    expect(printed(result)).toMatchObject({ escalation: 'exhausted', status: 'failed' })
    expect(attemptedModels(result)).toEqual(['vibe:glm-5-3', 'vibe:glm-5-3'])

    const named = await escalating({
      allowlist: allowlist(['deepseek', 'mistral'], true, true),
      override: { disabled: ['vibe:mistral-large-4-0'] },
      scripts: [FAILS, FAILS, SUCCEEDS],
    })
    const byName = await named.run(delegation(named, { model: 'deepseek-v4-1-flash-1017' }))
    expect(printed(byName)).toMatchObject({ escalation: 'exhausted', status: 'failed' })
  })

  it('never swaps to a model the machine disabled, by its alias or by a name it is known by', async () => {
    const w = await escalating({
      allowlist: allowlist(['anthropic', 'zhipu'], true, true),
      override: {
        clients: { vibe: { plan_limits: [{ limit_runs: 1, window: 'day' }] } },
        disabled: ['claude:claude-sonnet-5-5', 'claude:claude-opus-5-5', 'claude:claude-x-alias'],
        models: [{ client: 'claude', family: 'anthropic', model: 'claude-x', names: ['claude-x-alias'], notes: 'x', tier: 'hard' }],
      },
      scripts: [CLAUDE_SUCCEEDS],
    })
    await w.writeLedger([ledgerRecord(TODAY, 'vibe')])

    const result = await w.run(delegation(w))

    expect(printed(result)).toMatchObject({ status: 'unavailable' })
    expect(w.vibe.commands).toEqual([])
  })

  it('climbs from a metered-personal model only to a stronger subsidised one, never to a stronger metered-personal one', async () => {
    const hermes = new FakeHermesProcess([
      { stdout: 'no', usage: hermesUsageFile({ failed: true }) },
      { stdout: 'no', usage: hermesUsageFile({ failed: true }) },
      { stdout: claudeResult('fine\nSUMMARY: ok') },
    ])
    const w = await runWorkspace({
      allowlist: allowlist(['anthropic'], true, true),
      override: {
        ...HERMES_MACHINE,
        models: [
          {
            client: 'hermes',
            family: 'anthropic',
            model: 'anthropic/claude-sonnet-4.6',
            notes: 'x',
            prices: { cached_input: 0.3, input: 3, output: 15 },
            tier: 'hard',
          },
          {
            client: 'hermes',
            family: 'anthropic',
            model: 'anthropic/claude-opus-4.8',
            notes: 'x',
            prices: { cached_input: 0.3, input: 3, output: 15 },
            tier: 'frontier',
          },
        ],
      },
      process: hermes,
      setupResults: setupResults('pass', 'hermes'),
    })

    const result = await w.run(hermesInput(w, { escalate: undefined }))

    expect(attemptedModels(result)).toEqual([
      'hermes:anthropic/claude-sonnet-4.6',
      'hermes:anthropic/claude-sonnet-4.6',
      'claude:claude-opus-5-5',
    ])
    expect(printed(result)).toMatchObject({ status: 'ok' })
  })

  it('swaps a Hermes run that cannot start only to another Hermes model of the provider it named', async () => {
    const hermes = new FakeHermesProcess([
      { effect: () => Promise.reject(new Error('spawn hermes ENOENT')), stdout: '' },
      { stdout: 'the answer\nSUMMARY: ok', usage: hermesUsageFile() },
    ])
    const w = await runWorkspace({
      allowlist: allowlist(['anthropic'], true, true),
      override: {
        ...HERMES_MACHINE,
        models: [
          {
            client: 'hermes',
            family: 'anthropic',
            model: 'anthropic/claude-sonnet-4.6',
            notes: 'x',
            prices: { cached_input: 0.3, input: 3, output: 15 },
            tier: 'hard',
          },
          {
            client: 'hermes',
            family: 'anthropic',
            model: 'anthropic/claude-opus-4.8',
            notes: 'x',
            prices: { cached_input: 0.3, input: 3, output: 15 },
            tier: 'frontier',
          },
        ],
      },
      process: hermes,
      setupResults: setupResults('pass', 'hermes'),
    })

    const result = await w.run(hermesInput(w, { escalate: undefined }))

    expect(attemptedModels(result)).toEqual(['hermes:anthropic/claude-sonnet-4.6', 'hermes:anthropic/claude-opus-4.8'])
    expect(printed(result)).toMatchObject({ status: 'ok' })
    expect(hermes.commands[1]?.args).toContain('anthropic')
  })

  it('does not count an attempt whose client could not start toward max_attempts, so the climb still launches', async () => {
    const w = await escalating({
      scripts: [
        { exitCode: 1, stderr: 'Not logged in. Run `vibe login`.', stdout: '' },
        { stdout: claudeResult('done') },
        { exitCode: 1, stdout: '1 failed' },
        { stdout: claudeResult('done again') },
        { exitCode: 1, stdout: '1 failed' },
        { stdout: claudeResult('done at last') },
        { stdout: '3 passed' },
      ],
    })

    const result = await w.run(writeInput(w, { escalate: undefined }))

    expect(attemptedModels(result)).toEqual([
      'vibe:glm-5-3',
      'claude:claude-sonnet-5-5',
      'claude:claude-sonnet-5-5',
      'claude:claude-opus-5-5',
    ])
    expect(attemptsOf(result).map((attempt) => attempt.reason)).toEqual(['auth', 'check-failed', 'check-failed', undefined])
    expect(printed(result)).toMatchObject({ model: 'claude-opus-5-5', status: 'ok' })
    expect(printed(result)).not.toHaveProperty('escalation')
  })
})

/** A machine file that holds Vibe to the metered-personal caps, as a client the owner pays per token for. */
const METERED_VIBE = { clients: { vibe: { cost_class: 'metered-personal' } } }

/**
 * A machine file whose OpenCode client is subsidised on its own subscription
 * but metered on the zai provider account, with a cap just above one run's
 * estimate: 1,000 prompt tokens at $1 and 6,000 output tokens at $2 is $0.013.
 */
const METERED_OPENCODE = {
  clients: {
    opencode: {
      cost_class: 'subsidised',
      provider_cost_classes: { zai: 'metered-personal' },
      providers: ['opencode-go', 'zai'],
    },
  },
  models: [
    {
      client: 'opencode',
      family: 'zhipu',
      model: 'zai/glm-5-3',
      notes: 'Metered on the provider account.',
      prices: { cached_input: 0, input: 1, output: 2 },
      tier: 'hard',
    },
  ],
  settings: { metered_personal_caps: { per_day_usd: 5, per_workflow_usd: 0.02 } },
}

/** A prompt of 3,500 characters, which is 1,000 tokens at 3.5 characters a token. */
const LONG_PROMPT = 'x'.repeat(3500)

/** One reservation line, as a run that never settled left it. */
function reservationRecord(overrides: Partial<ReservationRecord> = {}): ReservationRecord {
  return {
    client: 'hermes',
    cost_class: 'metered-personal',
    estimate_usd: 0.5,
    expires_at: '2026-11-04T10:00:00.000Z',
    kind: 'reservation',
    model: 'anthropic/claude-sonnet-4.6',
    reserved_at: '2026-11-04T09:00:00.000Z',
    run_dir: '/nowhere',
    run_id: 'run-1',
    workflow: 'spec-293',
    ...overrides,
  }
}

/** One settlement line, as a run that ended, or a reservation that expired, left it. */
function settledLine(runId: string, costUsd: number, reason?: 'expired' | 'not-started'): SettledRecord {
  return {
    client: 'hermes',
    cost_class: 'metered-personal',
    cost_usd: costUsd,
    kind: 'settled',
    reserved_at: '2026-11-04T09:00:00.000Z',
    run_id: runId,
    settled_at: '2026-11-04T09:05:00.000Z',
    workflow: 'spec-293',
    ...(reason === undefined ? {} : { reason }),
  }
}

/** The real files, with one path whose existence cannot be looked at: the file system's own error, not a missing file. */
function filesWithUnreadableExists(broken: string): ModelGatewayFilesPort {
  const files = new NodeModelGatewayFiles()
  return {
    exists: (path) => (path === broken ? Promise.reject(new Error(`EACCES: permission denied, access '${path}'`)) : files.exists(path)),
    mkdir: (path) => files.mkdir(path),
    readText: (path) => files.readText(path),
    remove: (path) => files.remove(path),
    tempDir: (prefix) => files.tempDir(prefix),
    writeText: (path, content) => files.writeText(path, content),
  }
}

/** The ledger's lines of one kind: `reservation`, `settled`, or a run's own line for `undefined`. */
async function ledgerOf(w: RunWorkspace, kind: 'reservation' | 'settled' | undefined): Promise<Record<string, unknown>[]> {
  return ledgerLines(await readFile(w.ledger, 'utf8').catch(() => '')).filter((line) => line.kind === kind)
}

describe('model-gateway run: the metered-personal caps', () => {
  it('reserves the estimate: the prompt at the input price plus the tier’s allowance at the output price', async () => {
    const w = await readyRun({ override: METERED_VIBE })

    const result = await w.run(w.input({ prompt: LONG_PROMPT }))

    expect(result.exitCode).toBe(0)
    expect(await ledgerOf(w, 'reservation')).toEqual([
      {
        client: 'vibe',
        cost_class: 'metered-personal',
        estimate_usd: 0.0278,
        expires_at: '2026-11-04T11:08:15.000Z',
        kind: 'reservation',
        model: 'glm-5-3',
        reserved_at: '2026-11-04T10:52:15.000Z',
        run_dir: runDirOf(w, result),
        run_id: printed(result).run_id,
        workflow: 'spec-293',
      },
    ])
  })

  it('takes the output allowance from the machine file where it overrides the shipped one', async () => {
    const w = await readyRun({
      override: { ...METERED_VIBE, settings: { output_allowance_tokens: { hard: 1000 } } },
    })

    await w.run(w.input({ prompt: LONG_PROMPT }))

    expect((await ledgerOf(w, 'reservation'))[0]?.estimate_usd).toBe(0.0058)
  })

  it('passes the estimate to the client as its hard spend limit, where the client takes one', async () => {
    const w = await readyRun({ override: METERED_VIBE })

    await w.run(w.input({ prompt: LONG_PROMPT }))

    expect(flag(w.vibe.commands[0], '--max-price')).toBe('0.0278')
  })

  it('reserves nothing for a subsidised run, which its plan limit watches', async () => {
    const w = await readyRun()

    await w.run(w.input({ prompt: LONG_PROMPT }))

    expect(await ledgerOf(w, 'reservation')).toEqual([])
    expect(flag(w.vibe.commands[0], '--max-price')).toBeUndefined()
  })

  it('refuses a metered run the catalogue cannot estimate, before anything is launched', async () => {
    const { hermes, w } = await readyHermes({ override: { clients: { hermes: { providers: ['anthropic'] } } } })

    expect(printed(await w.run(hermesInput(w)))).toEqual({
      code: 'no-estimate',
      message:
        'hermes:anthropic/claude-sonnet-4.6 cannot be estimated: the catalogue gives it no price or no tier, so a metered run on it cannot be held to the caps',
      status: 'error',
    })
    expect(hermes.commands).toEqual([])
  })

  it('refuses a run that does not fit the per-workflow cap, naming the cap, its limit, the spend and the estimate', async () => {
    const { hermes, w } = await readyHermes()
    await w.writeLedger([ledgerRecord('2026-11-04T09:00:00.000Z', 'hermes', { cost_usd: 1.99 })])

    expect(printed(await w.run(hermesInput(w, { prompt: LONG_PROMPT })))).toEqual({
      code: 'cap-exceeded',
      message:
        'hermes:anthropic/claude-sonnet-4.6 does not fit the per-workflow cap of $2.00: $1.99 spent and $0.00 reserved in workflow spec-293, and its estimate is $0.03',
      status: 'error',
    })
    expect(hermes.commands).toEqual([])
    expect(await ledgerOf(w, 'reservation')).toEqual([])
  })

  it('counts what live reservations hold, not only what is spent', async () => {
    const { w } = await readyHermes()
    await w.writeLedger([
      ledgerRecord('2026-11-04T09:00:00.000Z', 'hermes', { cost_usd: 0.49 }),
      reservationRecord({ estimate_usd: 1.5, expires_at: '2026-11-04T12:00:00.000Z' }),
    ])

    expect(printed(await w.run(hermesInput(w, { prompt: LONG_PROMPT })))).toMatchObject({
      code: 'cap-exceeded',
      message: expect.stringContaining('$0.49 spent and $1.50 reserved in workflow spec-293') as unknown,
    })
  })

  it('refuses a run that does not fit the per-day cap, naming the UTC day', async () => {
    const { w } = await readyHermes()
    await w.writeLedger([ledgerRecord('2026-11-04T09:00:00.000Z', 'hermes', { cost_usd: 4.99, workflow: 'other' })])

    expect(printed(await w.run(hermesInput(w, { prompt: LONG_PROMPT })))).toEqual({
      code: 'cap-exceeded',
      message:
        'hermes:anthropic/claude-sonnet-4.6 does not fit the per-day cap of $5.00: $4.99 spent and $0.00 reserved on 2026-11-04, and its estimate is $0.03',
      status: 'error',
    })
  })

  it('counts what live reservations hold against the day cap too', async () => {
    const { w } = await readyHermes()
    await w.writeLedger([
      ledgerRecord('2026-11-04T09:00:00.000Z', 'hermes', { cost_usd: 4.5, workflow: 'other' }),
      reservationRecord({ estimate_usd: 0.5, expires_at: '2026-11-04T12:00:00.000Z', workflow: 'another' }),
    ])

    expect(printed(await w.run(hermesInput(w, { prompt: LONG_PROMPT })))).toMatchObject({
      code: 'cap-exceeded',
      message: expect.stringContaining('$4.50 spent and $0.50 reserved on 2026-11-04') as unknown,
    })
  })

  it('counts a settled run once, from its own line rather than its settlement', async () => {
    const { w } = await readyHermes()
    await w.writeLedger([
      ledgerRecord('2026-11-04T09:00:00.000Z', 'hermes', { cost_usd: 1, run_id: 'run-a' }),
      settledLine('run-a', 1),
      ledgerRecord('2026-11-04T09:10:00.000Z', 'hermes', { cost_usd: 0.9, run_id: 'run-b' }),
      settledLine('run-b', 0.9, 'expired'),
    ])

    expect(printed(await w.run(hermesInput(w, { prompt: LONG_PROMPT }))).status).toBe('ok')
  })

  it('counts a run of another day against its own day, not today’s', async () => {
    const { w } = await readyHermes()
    await w.writeLedger([ledgerRecord('2026-11-03T09:00:00.000Z', 'hermes', { cost_usd: 4.99, workflow: 'other' })])

    expect(printed(await w.run(hermesInput(w, { prompt: LONG_PROMPT }))).status).toBe('ok')
  })

  it('replaces the reservation with the run’s actual spend when it ends', async () => {
    const { w } = await readyHermes()

    const result = await w.run(hermesInput(w))

    expect(await ledgerOf(w, 'settled')).toEqual([
      {
        client: 'hermes',
        cost_class: 'metered-personal',
        cost_usd: 0.0234,
        kind: 'settled',
        reserved_at: '2026-11-04T10:52:15.000Z',
        run_id: printed(result).run_id,
        settled_at: '2026-11-04T10:52:15.000Z',
        workflow: 'spec-293',
      },
    ])
    expect(printed(await w.cli('usage', '--json', '{}')).clients).toContainEqual(
      expect.objectContaining({ client: 'hermes', reserved_runs: 0, reserved_usd: 0 }),
    )
  })

  it('settles a reservation whose run died from the result the run left', async () => {
    const w = await runWorkspace()
    const runDir = join(w.dir, 'dead-run')
    await mkdir(runDir, { recursive: true })
    await writeFile(join(runDir, 'result.json'), JSON.stringify({ cost_usd: 0.4 }))
    await w.writeLedger([reservationRecord({ run_dir: runDir })])

    await w.cli('usage', '--json', '{}')

    expect(await ledgerOf(w, 'settled')).toEqual([
      {
        client: 'hermes',
        cost_class: 'metered-personal',
        cost_usd: 0.4,
        kind: 'settled',
        reason: 'expired',
        reserved_at: '2026-11-04T09:00:00.000Z',
        run_id: 'run-1',
        settled_at: '2026-11-04T10:52:15.000Z',
        workflow: 'spec-293',
      },
    ])
  })

  it('counts a reservation whose run left nothing at its full estimate', async () => {
    const w = await runWorkspace()
    await w.writeLedger([reservationRecord({ estimate_usd: 0.5 })])

    await w.cli('usage', '--json', '{}')

    expect(await ledgerOf(w, 'settled')).toMatchObject([{ cost_usd: 0.5, reason: 'expired', run_id: 'run-1' }])
  })

  it('counts a reservation whose run left a result it cannot read at its full estimate', async () => {
    const w = await runWorkspace()
    const runDir = join(w.dir, 'torn-run')
    await mkdir(runDir, { recursive: true })
    await writeFile(join(runDir, 'result.json'), 'not json')
    await w.writeLedger([reservationRecord({ estimate_usd: 0.5, run_dir: runDir })])

    await w.cli('usage', '--json', '{}')

    expect(await ledgerOf(w, 'settled')).toMatchObject([{ cost_usd: 0.5, reason: 'expired' }])
  })

  it('counts an expired reservation as spent against the caps', async () => {
    const { w } = await readyHermes()
    await w.writeLedger([reservationRecord({ estimate_usd: 1.99 })])

    expect(printed(await w.run(hermesInput(w, { prompt: LONG_PROMPT })))).toMatchObject({ code: 'cap-exceeded' })
    expect(await ledgerOf(w, 'settled')).toMatchObject([{ cost_usd: 1.99, reason: 'expired' }])
  })

  it('leaves a reservation that has not expired alone, and reports what it holds', async () => {
    const w = await runWorkspace()
    await w.writeLedger([reservationRecord({ estimate_usd: 0.5, expires_at: '2026-11-04T12:00:00.000Z' })])

    const result = await w.cli('usage', '--json', '{}')

    expect(await ledgerOf(w, 'settled')).toEqual([])
    expect(usageClient(result, 'hermes')).toMatchObject({ reserved_runs: 1, reserved_usd: 0.5 })
  })

  it('counts a run once when its own line is there and its settlement is missing', async () => {
    const { w } = await readyHermes()
    await w.writeLedger([
      ledgerRecord('2026-11-04T09:00:00.000Z', 'hermes', { cost_usd: 1, run_id: 'run-1' }),
      reservationRecord({ estimate_usd: 1.99, expires_at: '2026-11-04T12:00:00.000Z' }),
    ])

    expect(printed(await w.run(hermesInput(w, { prompt: LONG_PROMPT }))).status).toBe('ok')
  })

  it('settles a run that left a result at once, before its reservation expires', async () => {
    const { w } = await readyHermes()
    const runDir = join(w.dir, 'finished-run')
    await mkdir(runDir, { recursive: true })
    await writeFile(join(runDir, 'result.json'), JSON.stringify({ cost_usd: 0.4 }))
    await w.writeLedger([
      reservationRecord({ estimate_usd: 1.99, expires_at: '2026-11-04T12:00:00.000Z', run_dir: runDir }),
    ])

    expect(printed(await w.run(hermesInput(w, { prompt: LONG_PROMPT }))).status).toBe('ok')
    expect(await ledgerOf(w, 'settled')).toContainEqual(expect.objectContaining({ cost_usd: 0.4, run_id: 'run-1' }))
  })

  it('holds a reservation whose run left neither a line nor a result until it expires', async () => {
    const { w } = await readyHermes()
    await w.writeLedger([reservationRecord({ estimate_usd: 1.99, expires_at: '2026-11-04T12:00:00.000Z' })])

    expect(printed(await w.run(hermesInput(w, { prompt: LONG_PROMPT })))).toMatchObject({
      code: 'cap-exceeded',
      message: expect.stringContaining('$1.99 reserved in workflow spec-293') as unknown,
    })
    expect(await ledgerOf(w, 'settled')).toEqual([])
  })

  it('counts a reservation whose run directory cannot be looked at at its full estimate, and still reports', async () => {
    const runDir = '/nowhere/unreadable'
    const w = await runWorkspace({ files: filesWithUnreadableExists(join(runDir, 'result.json')) })
    await w.writeLedger([reservationRecord({ estimate_usd: 0.5, run_dir: runDir })])

    const result = await w.cli('usage', '--json', '{}')

    expect(result.exitCode).toBe(0)
    expect(await ledgerOf(w, 'settled')).toMatchObject([{ cost_usd: 0.5, reason: 'expired' }])
  })

  it('holds the cap when two gateway processes reserve at once', async () => {
    let launched = (): void => undefined
    let release = (): void => undefined
    const started = new Promise<void>((resolve) => {
      launched = resolve
    })
    const blocked = new Promise<void>((resolve) => {
      release = resolve
    })
    const first = new FakeHermesProcess([
      {
        effect: async () => {
          launched()
          await blocked
        },
        stdout: 'a\nSUMMARY: a',
        usage: hermesUsageFile({ cost: 0.01 }),
      },
    ])
    const { w } = await readyHermes({ process: first })
    const other = w.gateway(new FakeHermesProcess([{ stdout: 'b\nSUMMARY: b', usage: hermesUsageFile({ cost: 0.01 }) }]))
    await w.writeLedger([ledgerRecord('2026-11-04T09:00:00.000Z', 'hermes', { cost_usd: 1.95 })])
    const input = hermesInput(w, { prompt: LONG_PROMPT })

    const one = w.run(input)
    await started
    const two = await runModelGatewayCli(['run', '--json', JSON.stringify(input)], other)
    release()

    expect(printed(await one).status).toBe('ok')
    expect(printed(two)).toMatchObject({ code: 'cap-exceeded' })
    expect(await ledgerOf(w, 'reservation')).toHaveLength(1)
  })

  it('holds the cap for a run whose provider is metered-personal, not only its client', async () => {
    let launched = (): void => undefined
    let release = (): void => undefined
    const started = new Promise<void>((resolve) => {
      launched = resolve
    })
    const blocked = new Promise<void>((resolve) => {
      release = resolve
    })
    const steps = openCodeEvents(OPENCODE_STEP_START, openCodeText('a\nSUMMARY: a'), openCodeStepFinish(0.01, { input: 100, output: 10 }))
    const w = await clientRun({
      override: METERED_OPENCODE,
      scripts: [
        {
          effect: async () => {
            launched()
            await blocked
          },
          stdout: steps,
        },
      ],
    })
    const other = w.gateway(new FakeVibeProcess([{ stdout: steps }]))
    const input = w.input({ client: 'opencode', model: 'zai/glm-5-3', prompt: LONG_PROMPT })

    const one = w.run(input)
    await started
    const two = await runModelGatewayCli(['run', '--json', JSON.stringify(input)], other)
    release()

    expect(printed(await one).status).toBe('ok')
    expect(printed(two)).toMatchObject({ code: 'cap-exceeded' })
  })

  it('counts an expired reservation of a metered provider run at its estimate', async () => {
    const w = await clientRun({ override: METERED_OPENCODE })
    await w.writeLedger([
      reservationRecord({ client: 'opencode', cost_class: 'metered-personal', estimate_usd: 0.02, model: 'zai/glm-5-3' }),
    ])

    expect(printed(await w.run(w.input({ client: 'opencode', model: 'zai/glm-5-3', prompt: LONG_PROMPT })))).toMatchObject({
      code: 'cap-exceeded',
    })
    expect(await ledgerOf(w, 'settled')).toMatchObject([{ cost_usd: 0.02, reason: 'expired' }])
  })

  it('starts no workflow and makes no worktree for a run the caps refuse', async () => {
    const { w } = await readyHermes()
    await w.writeLedger([ledgerRecord('2026-11-04T09:00:00.000Z', 'hermes', { cost_usd: 1.99 })])

    expect(printed(await w.run(hermesInput(w, { prompt: LONG_PROMPT })))).toMatchObject({ code: 'cap-exceeded' })
    expect(await listedWorkflows(w)).toEqual([])
    expect(w.git.worktrees).toEqual([])
  })

  it('takes no reservation for a run whose workflow ended', async () => {
    const { w } = await readyHermes()
    await w.writeWorkflows({
      version: 1,
      workflows: [{ ended_at: '2026-11-04T11:00:00.000Z', name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }],
    })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({ code: 'workflow-ended' })
    expect(ledgerLines(await readFile(w.ledger, 'utf8').catch(() => ''))).toEqual([])
  })

  it('releases the reservation when the workflow cannot be started after it', async () => {
    const workflows = new FakeWorkflows([{ name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }])
    workflows.updateError = new ModelGatewayError(
      'workflow-ended',
      'workflow spec-293 ended at 2026-11-04T11:00:00.000Z: a run may not join it, so start a new workflow',
    )
    const { hermes, w } = await readyHermes({ workflows })

    expect(printed(await w.run(hermesInput(w)))).toMatchObject({ code: 'workflow-ended' })
    expect(hermes.commands).toEqual([])
    expect(await ledgerOf(w, 'settled')).toMatchObject([{ cost_usd: 0, reason: 'not-started' }])
  })
})

/** What `codex exec --json` prints: one event per line, ending in the turn's usage. */
function codexEvents(...events: readonly unknown[]): string {
  return events.map((event) => JSON.stringify(event)).join('\n')
}

/** A Codex run's answer: the last agent message, then the turn's usage. */
function codexAnswer(text: string, usage: Readonly<Record<string, number>> = {}): string {
  return codexEvents(
    { thread_id: 't-1', type: 'thread.started' },
    { type: 'turn.started' },
    { item: { id: 'item_0', text, type: 'agent_message' }, type: 'item.completed' },
    {
      type: 'turn.completed',
      usage: { cached_input_tokens: 100, input_tokens: 1000, output_tokens: 200, reasoning_output_tokens: 50, ...usage },
    },
  )
}

/** The grants file the gateway reads and writes, in this workspace's machine configuration. */
function grantsFile(w: RunWorkspace): string {
  return join(w.home, '.config', 'agent-kit', 'model-gateway-grants.json')
}

/** The grants the machine holds, as the file holds them. */
async function grantsOf(w: RunWorkspace): Promise<Record<string, unknown>[]> {
  const text = await readFile(grantsFile(w), 'utf8').catch(() => '')
  return text === '' ? [] : (JSON.parse(text) as { grants: Record<string, unknown>[] }).grants
}

/** A Codex run's input: the metered-employer client, on the model the catalogue prices. */
function codexInput(w: RunWorkspace, overrides: Readonly<Record<string, unknown>> = {}): unknown {
  return w.input({ client: 'codex', model: 'gpt-5.5', ...overrides })
}

/** What `grant write` takes unless a test says otherwise: the owner's answer for the test workflow. */
const GRANT_INPUT = {
  answer: 'yes, up to five dollars',
  cap_usd: 5,
  client: 'codex',
  question: 'may I spend up to $5 of nedap money on spec-293?',
  workflow: 'spec-293',
}

/** Writes a grant through the command line, as the agent does after asking the owner, and hands back what the file holds. */
async function writeGrant(w: RunWorkspace, overrides: Readonly<Record<string, unknown>> = {}): Promise<Record<string, unknown>> {
  const result = await w.cli('grant', 'write', '--json', JSON.stringify({ ...GRANT_INPUT, ...overrides }))
  if (result.exitCode !== 0) throw new Error(`grant write failed: ${result.stdout}`)
  return storedGrant(printed(result).grant as Record<string, unknown>)
}

/** A grant as the grants file holds it: what `grant write` printed, without what it has left. */
function storedGrant(view: Record<string, unknown>): Record<string, unknown> {
  const stored = { ...view }
  delete stored.remaining_usd
  delete stored.reserved_usd
  delete stored.spent_usd
  return stored
}

interface CodexWorkspace {
  readonly w: RunWorkspace
  readonly codex: FakeVibeProcess
}

/** A workspace whose Codex run is permitted, and whose client is scripted to answer `text`. */
async function readyCodex(
  options: RunWorkspaceOptions & { readonly text?: string; readonly scripts?: readonly VibeScript[] } = {},
): Promise<CodexWorkspace> {
  const codex = new FakeVibeProcess([...(options.scripts ?? [{ stdout: codexAnswer(options.text ?? 'the answer\nSUMMARY: it is fine') }])])
  const w = await runWorkspace({
    allowlist: allowlist(['openai'], true, true),
    process: codex,
    ...options,
  })
  return { codex, w }
}

/**
 * A workspace whose machine configuration directory lies inside a git
 * repository, as a session that pointed XDG_CONFIG_HOME at its checkout would
 * have it: the grants file must be refused there.
 */
async function repositoryWorkspace(): Promise<CodexWorkspace & { readonly repo: string }> {
  const dir = await mkdtemp(join(tmpdir(), 'model-gateway-grants-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const repo = join(dir, 'repo')
  await mkdir(repo, { recursive: true })
  const { w, codex } = await readyCodex({
    env: { XDG_CONFIG_HOME: join(repo, 'config') },
    repositories: (own) => ({ [own]: { commonDir: join(own, '.git'), remotes: [AGENT_KIT_REMOTE], root: own }, [repo]: { remotes: [], root: repo } }),
  })
  return { codex, repo, w }
}

describe('model-gateway grant write', () => {
  it('stores the grant in the machine configuration, with the question the owner answered and when it lapses', async () => {
    const { w } = await readyCodex()

    const result = await w.cli('grant', 'write', '--json', JSON.stringify(GRANT_INPUT))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toEqual({
      grant: {
        answer: 'yes, up to five dollars',
        cap_usd: 5,
        client: 'codex',
        expires_at: '2026-11-05T10:52:15.000Z',
        id: expect.stringMatching(/^grant-20261104T105215Z-[0-9a-f]{6}$/) as unknown,
        question: 'may I spend up to $5 of nedap money on spec-293?',
        remaining_usd: 5,
        reserved_usd: 0,
        spent_usd: 0,
        workflow: 'spec-293',
        written_at: '2026-11-04T10:52:15.000Z',
      },
      status: 'ok',
    })
    expect(await grantsOf(w)).toEqual([storedGrant(printed(result).grant as Record<string, unknown>)])
  })

  it('ends earlier when the workflow’s own 24 hours run out first', async () => {
    const { w } = await readyCodex()
    await w.writeWorkflows({ version: 1, workflows: [{ name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }] })

    expect((await writeGrant(w)).expires_at).toBe('2026-11-05T10:00:00.000Z')
  })

  it('refuses a workflow whose 24 hours already ran out, and writes nothing', async () => {
    const { w } = await readyCodex()
    await w.writeWorkflows({ version: 1, workflows: [{ name: 'spec-293', started_at: '2026-11-03T10:00:00.000Z' }] })

    expect(printed(await w.cli('grant', 'write', '--json', JSON.stringify(GRANT_INPUT)))).toMatchObject({
      code: 'workflow-ended',
      status: 'error',
    })
    expect(await grantsOf(w)).toEqual([])
  })

  it('refuses a second live grant for the same workflow and client, naming the one that stands', async () => {
    const { w } = await readyCodex()
    const grant = await writeGrant(w)

    expect(printed(await w.cli('grant', 'write', '--json', JSON.stringify(GRANT_INPUT)))).toEqual({
      code: 'grant-exists',
      message: `workflow spec-293 already has live grant ${String(grant.id)} of $5.00 with $5.00 left: the owner re-approves only after it lapsed`,
      status: 'error',
    })
    expect(await grantsOf(w)).toEqual([grant])
  })

  it('writes a new grant once the one before it lapsed', async () => {
    const { w } = await readyCodex()
    await writeGrant(w, { cap_usd: 1 })
    await w.writeLedger([
      ledgerRecord('2026-11-04T10:55:00.000Z', 'codex', { cost_class: 'metered-employer', cost_usd: 1, workflow: 'spec-293' }),
    ])

    expect((await writeGrant(w)).cap_usd).toBe(5)
    expect(await grantsOf(w)).toHaveLength(2)
  })

  it('refuses when the ledger cannot be read, since a standing grant cannot be ruled out, and writes nothing', async () => {
    const { w } = await readyCodex()
    await mkdir(w.ledger, { recursive: true })

    const result = printed(await w.cli('grant', 'write', '--json', JSON.stringify(GRANT_INPUT)))

    expect(result).toMatchObject({ code: 'ledger-unreadable', status: 'error' })
    expect(String(result.message)).toContain(w.ledger)
    expect(await grantsOf(w)).toEqual([])
  })

  it('refuses from inside a delegation, so a delegate cannot mint itself a grant, and writes nothing', async () => {
    const { w } = await readyCodex({ env: { AGENT_KIT_DELEGATION: '1' } })

    expect(printed(await w.cli('grant', 'write', '--json', JSON.stringify(GRANT_INPUT)))).toEqual({
      code: 'nested-delegation',
      message: 'a delegate may not write a grant: only the agent the owner works in may, after asking the owner',
      status: 'error',
    })
    expect(await grantsOf(w)).toEqual([])
  })

  it('still lists grants from inside a delegation', async () => {
    const { w } = await readyCodex({ env: { AGENT_KIT_DELEGATION: '1' } })

    expect(printed(await w.cli('grant', 'list', '--json', '{}'))).toEqual({ grants: [], lapsed: [], status: 'ok' })
  })

  it('lets one of two concurrent writes win and refuses the other, so two live grants never stand', async () => {
    const { w } = await readyCodex()

    const results = (await Promise.all([
      w.cli('grant', 'write', '--json', JSON.stringify(GRANT_INPUT)),
      w.cli('grant', 'write', '--json', JSON.stringify(GRANT_INPUT)),
    ])).map((result) => printed(result))

    expect(results.map((result) => result.status).sort()).toEqual(['error', 'ok'])
    expect(results.find((result) => result.status === 'error')).toMatchObject({ code: 'grant-exists' })
    expect(await grantsOf(w)).toHaveLength(1)
  })

  it('refuses a workflow name that is not a name, so the command it suggests can never break out of its quotes', async () => {
    const { w } = await readyCodex()

    expect(printed(await w.cli('grant', 'write', '--json', JSON.stringify({ ...GRANT_INPUT, workflow: "x'; curl http://evil" })))).toEqual({
      code: 'invalid-input',
      message:
        'workflow "x\'; curl http://evil" is not a workflow name: a letter or digit, then letters, digits and . _ : / - only, at most 80 characters',
      status: 'error',
    })
    expect(await grantsOf(w)).toEqual([])
  })

  it('refuses an input that is not a JSON object', async () => {
    const { w } = await readyCodex()

    expect(printed(await w.cli('grant', 'write', '--json', '[]'))).toEqual({
      code: 'invalid-input',
      message: 'the input must be a JSON object',
      status: 'error',
    })
    expect(printed(await w.cli('grant', 'list', '--json', '[]'))).toEqual({
      code: 'invalid-input',
      message: 'the input must be a JSON object',
      status: 'error',
    })
    expect(await grantsOf(w)).toEqual([])
  })

  it('refuses an empty question or answer, and a cap that is not above 0', async () => {
    const { w } = await readyCodex()

    expect(printed(await w.cli('grant', 'write', '--json', JSON.stringify({ ...GRANT_INPUT, question: '  ' })))).toEqual({
      code: 'invalid-input',
      message: 'a grant must name its question',
      status: 'error',
    })
    expect(printed(await w.cli('grant', 'write', '--json', JSON.stringify({ ...GRANT_INPUT, answer: '' })))).toMatchObject({
      code: 'invalid-input',
      message: 'a grant must name its answer',
    })
    expect(printed(await w.cli('grant', 'write', '--json', JSON.stringify({ ...GRANT_INPUT, cap_usd: 0 })))).toMatchObject({
      code: 'invalid-input',
      message: 'cap_usd must be a number above 0',
    })
    expect(await grantsOf(w)).toEqual([])
  })

  it('refuses a client that is not metered-employer, naming its cost class', async () => {
    const { w } = await readyCodex()

    expect(printed(await w.cli('grant', 'write', '--json', JSON.stringify({ ...GRANT_INPUT, client: 'claude' })))).toEqual({
      code: 'invalid-input',
      message:
        "claude is subsidised: a grant is for a metered-employer client, which is the one that spends someone else's money",
      status: 'error',
    })
    expect(printed(await w.cli('grant', 'write', '--json', JSON.stringify({ ...GRANT_INPUT, client: 'nobody' })))).toMatchObject({
      code: 'unknown-model',
    })
    expect(await grantsOf(w)).toEqual([])
  })

  it('refuses a workflow that ended', async () => {
    const { w } = await readyCodex()
    await w.writeWorkflows({
      version: 1,
      workflows: [{ ended_at: '2026-11-04T11:00:00.000Z', name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }],
    })

    expect(printed(await w.cli('grant', 'write', '--json', JSON.stringify(GRANT_INPUT)))).toMatchObject({
      code: 'workflow-ended',
    })
    expect(await grantsOf(w)).toEqual([])
  })

  it('refuses to write a grant inside a git repository, naming the repository', async () => {
    const { w, repo } = await repositoryWorkspace()

    expect(printed(await w.cli('grant', 'write', '--json', JSON.stringify(GRANT_INPUT)))).toEqual({
      code: 'grants-in-repository',
      message: `${join(repo, 'config', 'agent-kit', 'model-gateway-grants.json')} is inside the git repository ${repo}: a grant lives in the machine configuration, never in a repository`,
      status: 'error',
    })
  })

  it('refuses to read a grants file inside a git repository too', async () => {
    const { w } = await repositoryWorkspace()

    expect(printed(await w.cli('grant', 'list', '--json', '{}'))).toMatchObject({ code: 'grants-in-repository' })
  })
})

describe('model-gateway grant list', () => {
  it('lists a live grant with what it has left: its cap minus what the workflow spent and what reservations hold', async () => {
    const { w } = await readyCodex()
    const grant = await writeGrant(w)
    await w.writeLedger([
      ledgerRecord('2026-11-04T10:55:00.000Z', 'codex', { cost_class: 'metered-employer', cost_usd: 1.5 }),
      reservationRecord({
        client: 'codex',
        cost_class: 'metered-employer',
        estimate_usd: 0.5,
        expires_at: '2026-11-04T12:00:00.000Z',
        reserved_at: '2026-11-04T10:52:15.000Z',
        run_id: 'run-live',
      }),
    ])

    expect(printed(await w.cli('grant', 'list', '--json', '{}'))).toEqual({
      grants: [{ ...grant, remaining_usd: 3, reserved_usd: 0.5, spent_usd: 1.5 }],
      lapsed: [],
      status: 'ok',
    })
  })

  it('lists a lapsed grant with why: its workflow ended, its 24 hours ran out, or its cap is spent', async () => {
    const { w } = await readyCodex()
    await writeGrant(w, { workflow: 'spec-293' })
    await w.writeWorkflows({
      version: 1,
      workflows: [
        { ended_at: '2026-11-04T11:00:00.000Z', name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' },
        { name: 'spec-294', started_at: '2026-11-04T10:00:00.000Z' },
      ],
    })
    await writeGrant(w, { workflow: 'spec-294', cap_usd: 1 })
    await w.writeLedger([
      ledgerRecord('2026-11-04T10:55:00.000Z', 'codex', { cost_class: 'metered-employer', cost_usd: 1, workflow: 'spec-294' }),
    ])

    const listed = printed(await w.cli('grant', 'list', '--json', '{}'))

    expect(listed.grants).toEqual([])
    expect((listed.lapsed as Record<string, unknown>[]).map((entry) => [entry.workflow, entry.reason])).toEqual([
      ['spec-293', 'workflow-ended'],
      ['spec-294', 'cap-spent'],
    ])
  })

  it('lapses a grant after 24 hours, and never renews it', async () => {
    const { w } = await readyCodex()
    const grant = await writeGrant(w)
    const late = defaultModelGatewayRuntime({
      clock: new FixedClock(new Date('2026-11-05T10:52:15.000Z')),
      defaultsPath: join(w.dir, 'defaults.json'),
      env: new FakeEnv({ HOME: w.home, XDG_STATE_HOME: w.state }),
      git: w.git,
      process: new FakeVibeProcess(),
      routingPath: join(w.dir, 'routing.json'),
    })

    expect(printed(await runModelGatewayCli(['grant', 'list', '--json', '{}'], late))).toMatchObject({
      grants: [],
      lapsed: [{ id: grant.id, reason: 'expired' }],
    })
    expect(await grantsOf(w)).toEqual([grant])
  })
})

describe('model-gateway run: Codex, the metered-employer client', () => {
  it('runs under the time left on its workflow, not its full time limit', async () => {
    const { codex, w } = await readyCodex()
    // The workflow's 24 hours run out 5 minutes after the fixed clock.
    await w.writeWorkflows({ version: 1, workflows: [{ name: 'spec-293', started_at: '2026-11-03T10:57:15.000Z' }] })
    await writeGrant(w)

    const result = await w.run(codexInput(w, { time_limit_s: 900 }))

    expect(printed(result)).toMatchObject({ status: 'ok' })
    expect(codex.commands[0]?.timeoutMs).toBe(300_000)
  })

  it('refuses a run launched with under a minute left on its grant, and starts nothing', async () => {
    const { codex, w } = await readyCodex()
    await w.writeWorkflows({ version: 1, workflows: [{ name: 'spec-293', started_at: '2026-11-03T10:52:45.000Z' }] })
    await writeGrant(w)

    expect(printed(await w.run(codexInput(w)))).toMatchObject({ code: 'grant-expiring', status: 'error' })
    expect(codex.commands).toEqual([])
  })

  it('runs codex exec non-interactively, and reads its tokens and answer from the events', async () => {
    const { codex, w } = await readyCodex()
    await writeGrant(w)

    const result = await w.run(codexInput(w))

    expect(result.exitCode).toBe(0)
    expect(printed(result)).toMatchObject({
      client: 'codex',
      cost_usd: 0.0031375,
      family: 'openai',
      model: 'gpt-5.5',
      status: 'ok',
      summary: 'it is fine',
      tokens: { cached: 100, input: 1000, output: 200 },
      workflow: 'spec-293',
    })
    const command = codex.commands[0]
    expect(command?.command).toBe('codex')
    expect(command?.args).toEqual([
      'exec',
      '--json',
      '--skip-git-repo-check',
      '-m',
      'gpt-5.5',
      '-c',
      'model_reasoning_effort="medium"',
      '-s',
      'read-only',
      '-C',
      expect.any(String) as unknown,
    ])
    expect(command?.stdin).toContain('read this')
    expect(command?.env).toEqual({ AGENT_KIT_DELEGATION: '1' })
  })

  it('asks for the reasoning effort the model id names, and a workspace-write sandbox for a writing run', async () => {
    const { codex, w } = await readyCodex({
      override: {
        models: [
          {
            client: 'codex',
            family: 'openai',
            model: 'gpt-5.5-high',
            notes: 'the same model, asking for more reasoning',
            prices: { cached_input: 0.125, input: 1.25, output: 10 },
            tier: 'frontier',
          },
        ],
      },
      scripts: [{ effect: edits('src/app.ts'), stdout: codexAnswer('done\nSUMMARY: added it') }, { stdout: '3 passed\n' }],
    })
    await writeGrant(w)

    const result = await w.run(codexInput(w, { base: 'main', check: 'true', mode: 'write', model: 'gpt-5.5-high' }))

    expect(printed(result)).toMatchObject({ status: 'ok' })
    expect(flag(codex.commands[0], '-c')).toBe('model_reasoning_effort="high"')
    expect(flag(codex.commands[0], '-s')).toBe('workspace-write')
  })

  it('gives the check only the time left on the grant after the delegate ran, not the time limit again', async () => {
    const clock = new FixedClock()
    const { codex, w } = await readyCodex({
      clock,
      scripts: [
        {
          effect: async (command) => {
            await edits('src/app.ts')(command)
            clock.advance(290_000)
          },
          stdout: codexAnswer('done\nSUMMARY: added it'),
        },
        { stdout: '3 passed\n' },
      ],
    })
    // The workflow's 24 hours run out 5 minutes after the clock starts.
    await w.writeWorkflows({ version: 1, workflows: [{ name: 'spec-293', started_at: '2026-11-03T10:57:15.000Z' }] })
    await writeGrant(w)

    const result = await w.run(codexInput(w, { base: 'main', check: 'true', mode: 'write', time_limit_s: 900 }))

    expect(printed(result)).toMatchObject({ status: 'ok' })
    expect(codex.commands[0]?.timeoutMs).toBe(300_000)
    expect(codex.commands[1]?.timeoutMs).toBe(10_000)
  })

  it('gives the check at least a second while the grant has any time left, and never more than its time limit', async () => {
    const clock = new FixedClock()
    const { codex, w } = await readyCodex({
      clock,
      scripts: [
        {
          effect: async (command) => {
            await edits('src/app.ts')(command)
            clock.advance(299_500)
          },
          stdout: codexAnswer('done\nSUMMARY: added it'),
        },
        { stdout: '3 passed\n' },
      ],
    })
    await w.writeWorkflows({ version: 1, workflows: [{ name: 'spec-293', started_at: '2026-11-03T10:57:15.000Z' }] })
    await writeGrant(w)

    await w.run(codexInput(w, { base: 'main', check: 'true', mode: 'write', time_limit_s: 900 }))

    expect(codex.commands[1]?.timeoutMs).toBe(1000)
  })

  it('does not start the check once the grant ran out while the delegate worked', async () => {
    const clock = new FixedClock()
    const { codex, w } = await readyCodex({
      clock,
      scripts: [
        {
          effect: async (command) => {
            await edits('src/app.ts')(command)
            clock.advance(300_000)
          },
          stdout: codexAnswer('done\nSUMMARY: added it'),
        },
        { stdout: '3 passed\n' },
      ],
    })
    await w.writeWorkflows({ version: 1, workflows: [{ name: 'spec-293', started_at: '2026-11-03T10:57:15.000Z' }] })
    await writeGrant(w)

    const result = printed(await w.run(codexInput(w, { base: 'main', check: 'true', mode: 'write', time_limit_s: 900 })))

    expect(result).toMatchObject({ reason: 'grant-expired', status: 'failed' })
    expect(result).not.toHaveProperty('check')
    expect(codex.commands).toHaveLength(1)
  })

  it('refuses a Codex run whose workflow no live grant covers, naming the workflow and the grant write call', async () => {
    const { codex, w } = await readyCodex()

    expect(printed(await w.run(codexInput(w)))).toEqual({
      code: 'no-grant',
      message:
        'no live grant covers workflow spec-293, so codex may not run: ask the owner, then add one with model-gateway grant write --json \'{"workflow":"spec-293","client":"codex","cap_usd":5,"question":"<what you asked the owner>","answer":"<what the owner answered>"}\'',
      status: 'error',
    })
    expect(codex.commands).toEqual([])
    expect(await listedWorkflows(w)).toEqual([])
    expect(ledgerLines(await readFile(w.ledger, 'utf8').catch(() => ''))).toEqual([])
  })

  it('refuses a headless Codex run without a grant with the same reason, and starts nothing', async () => {
    const { codex, w } = await readyCodex()

    expect(printed(await w.run(codexInput(w, { headless: true })))).toEqual({
      code: 'no-grant',
      message:
        'no live grant covers workflow spec-293, so codex may not run: ask the owner, then add one with model-gateway grant write --json \'{"workflow":"spec-293","client":"codex","cap_usd":5,"question":"<what you asked the owner>","answer":"<what the owner answered>"}\'',
      status: 'error',
    })
    expect(codex.commands).toEqual([])
    expect(await listedWorkflows(w)).toEqual([])
  })

  it('refuses a Codex run whose grant lapsed with its workflow, and one whose grant expired', async () => {
    const { w } = await readyCodex()
    await writeGrant(w)
    await w.writeWorkflows({
      version: 1,
      workflows: [{ ended_at: '2026-11-04T11:00:00.000Z', name: 'spec-293', started_at: '2026-11-04T10:00:00.000Z' }],
    })

    expect(printed(await w.run(codexInput(w)))).toMatchObject({ code: 'workflow-ended' })
  })

  it('reserves its estimate against the grant before it starts, and settles it with what the run cost', async () => {
    const { w } = await readyCodex()
    const grant = await writeGrant(w)

    const result = await w.run(codexInput(w))

    expect(await ledgerOf(w, 'reservation')).toEqual([
      {
        client: 'codex',
        cost_class: 'metered-employer',
        estimate_usd: 0.160004,
        expires_at: '2026-11-04T11:08:15.000Z',
        kind: 'reservation',
        model: 'gpt-5.5',
        reserved_at: '2026-11-04T10:52:15.000Z',
        run_dir: runDirOf(w, result),
        run_id: printed(result).run_id,
        workflow: 'spec-293',
      },
    ])
    expect(await ledgerOf(w, 'settled')).toMatchObject([{ cost_usd: 0.0031375, run_id: printed(result).run_id }])
    expect(printed(await w.cli('grant', 'list', '--json', '{}'))).toMatchObject({
      grants: [{ id: grant.id, remaining_usd: 4.996862, spent_usd: 0.003138 }],
    })
  })

  it('refuses a run that does not fit the grant, naming the grant, and reserves nothing', async () => {
    const { codex, w } = await readyCodex()
    const grant = await writeGrant(w, { cap_usd: 0.1 })

    expect(printed(await w.run(codexInput(w)))).toEqual({
      code: 'cap-exceeded',
      message: `codex:gpt-5.5 does not fit grant ${String(grant.id)} of $0.10 for workflow spec-293: $0.00 spent and $0.00 reserved, and its estimate is $0.16`,
      status: 'error',
    })
    expect(codex.commands).toEqual([])
    expect(await ledgerOf(w, 'reservation')).toEqual([])
    expect(await listedWorkflows(w)).toEqual([])
  })

  it('counts what live reservations hold against the grant, so two runs cannot together overshoot it', async () => {
    const { w } = await readyCodex()
    const grant = await writeGrant(w, { cap_usd: 0.2 })
    await w.writeLedger([
      reservationRecord({
        client: 'codex',
        cost_class: 'metered-employer',
        estimate_usd: 0.1,
        expires_at: '2026-11-04T12:00:00.000Z',
        reserved_at: '2026-11-04T10:52:15.000Z',
        run_id: 'run-live',
      }),
    ])

    expect(printed(await w.run(codexInput(w)))).toMatchObject({
      code: 'cap-exceeded',
      message: expect.stringContaining(`does not fit grant ${String(grant.id)} of $0.20`) as unknown,
    })
  })

  it('runs in a nedap repository under a grant, and is refused there without one', async () => {
    const nedap: GitRemote = { name: 'origin', url: 'git@github.com:nedap/agent-kit.git' }
    const { w } = await readyCodex({ repositories: (repo) => ({ [repo]: { remotes: [nedap], root: repo } }) })

    expect(printed(await w.run(codexInput(w)))).toMatchObject({ code: 'no-grant' })
    await writeGrant(w)
    expect(printed(await w.run(codexInput(w)))).toMatchObject({ client: 'codex', status: 'ok' })
  })

  it('never lets a grant of one client cover another client of the same workflow', async () => {
    const { w } = await readyCodex({
      allowlist: allowlist(['openai', 'zhipu'], true, true),
      override: {
        clients: { opencode: { provider_cost_classes: { 'opencode-go': 'metered-employer' } } },
        models: [
          {
            client: 'opencode',
            family: 'zhipu',
            model: 'opencode-go/glm-5-3',
            notes: 'the same model, on a provider this machine bills to the employer',
            prices: { cached_input: 0.14, input: 1.4, output: 4.4 },
            tier: 'hard',
          },
        ],
      },
    })
    await writeGrant(w)

    expect(printed(await w.run(w.input({ client: 'opencode', model: 'opencode-go/glm-5-3' })))).toMatchObject({
      code: 'no-grant',
      message: expect.stringContaining('no live grant covers workflow spec-293, so opencode may not run') as unknown,
    })
  })

  it('keeps a grant whose cap is only reserved live, and refuses a run that does not fit it as cap-exceeded', async () => {
    const { w } = await readyCodex()
    const grant = await writeGrant(w, { cap_usd: 0.2 })
    await w.writeLedger([
      reservationRecord({
        client: 'codex',
        cost_class: 'metered-employer',
        estimate_usd: 0.2,
        expires_at: '2026-11-04T12:00:00.000Z',
        reserved_at: '2026-11-04T10:52:15.000Z',
        run_id: 'run-live',
      }),
    ])

    expect(printed(await w.cli('grant', 'list', '--json', '{}'))).toMatchObject({
      grants: [{ id: grant.id, remaining_usd: 0, reserved_usd: 0.2 }],
      lapsed: [],
    })
    expect(printed(await w.run(codexInput(w)))).toMatchObject({
      code: 'cap-exceeded',
      message: expect.stringContaining(`does not fit grant ${String(grant.id)} of $0.20`) as unknown,
    })
  })

  it('settles a dead run’s reservation before it reads the grant, so the next run in the workflow starts', async () => {
    const { codex, w } = await readyCodex()
    await writeGrant(w)
    const deadDir = join(w.state, 'agent-kit', 'model-gateway', 'runs', 'run-dead')
    await mkdir(deadDir, { recursive: true })
    await writeFile(join(deadDir, 'result.json'), JSON.stringify({ cost_usd: 0.01 }))
    await w.writeLedger([
      reservationRecord({
        client: 'codex',
        cost_class: 'metered-employer',
        estimate_usd: 5,
        expires_at: '2026-11-04T11:00:00.000Z',
        reserved_at: '2026-11-04T10:53:00.000Z',
        run_dir: deadDir,
        run_id: 'run-dead',
      }),
    ])
    const later = defaultModelGatewayRuntime({
      clock: new FixedClock(new Date('2026-11-04T12:00:00.000Z')),
      defaultsPath: join(w.dir, 'defaults.json'),
      env: new FakeEnv({ HOME: w.home, XDG_STATE_HOME: w.state }),
      git: w.git,
      process: codex,
      routingPath: join(w.dir, 'routing.json'),
    })

    const result = await runModelGatewayCli(['run', '--json', JSON.stringify(codexInput(w, { escalate: false }))], later)

    expect(printed(result)).toMatchObject({ client: 'codex', status: 'ok' })
    expect(await ledgerOf(w, 'settled')).toContainEqual(expect.objectContaining({ cost_usd: 0.01, run_id: 'run-dead' }))
  })

  it('keeps a metered-employer model out of the fall-through pool when its grant is for another workflow or lapsed', async () => {
    const { w } = await readyCodex({
      allowlist: allowlist(['anthropic', 'openai'], true, true),
      scripts: [{ exitCode: 1, stderr: '429 Too Many Requests', stdout: '' }],
    })
    await writeGrant(w, { workflow: 'spec-294' })
    await writeGrant(w, { cap_usd: 1 })
    await w.writeLedger([
      ledgerRecord('2026-11-04T10:55:00.000Z', 'codex', { cost_class: 'metered-employer', cost_usd: 1, workflow: 'spec-293' }),
    ])

    const result = await w.run(w.input({ client: 'claude', escalate: true, model: 'claude-sonnet-5-5' }))

    expect(printed(result)).toMatchObject({ status: 'unavailable' })
    expect(attemptedModels(result)).toEqual(['claude:claude-sonnet-5-5'])
  })

  it('never retries a no-grant refusal, and never climbs to a metered-employer model', async () => {
    const { codex, w } = await readyCodex()

    expect(printed(await w.run(codexInput(w, { escalate: true })))).toMatchObject({ code: 'no-grant' })
    expect(codex.commands).toEqual([])
  })

  it('falls through to another Codex model under a grant when the first cannot start', async () => {
    const { codex, w } = await readyCodex({
      override: {
        models: [
          {
            client: 'codex',
            family: 'openai',
            model: 'gpt-5.6',
            notes: 'newer',
            prices: { cached_input: 0.125, input: 1.25, output: 10 },
            tier: 'frontier',
          },
        ],
      },
      scripts: [
        { effect: () => Promise.reject(new Error('spawn codex ENOENT')), stdout: '' },
        { stdout: codexAnswer('the answer\nSUMMARY: it is fine') },
      ],
    })
    await writeGrant(w)

    const result = await w.run(codexInput(w, { escalate: true }))

    expect(printed(result)).toMatchObject({ client: 'codex', model: 'gpt-5.6', status: 'ok' })
    expect(attemptedModels(result)).toEqual(['codex:gpt-5.5', 'codex:gpt-5.6'])
    expect(codex.commands).toHaveLength(2)
  })
})
