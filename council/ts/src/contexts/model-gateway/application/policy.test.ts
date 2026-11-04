import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  FakeEnv,
  FakeFiles,
  FakeGit,
  FakeKept,
  FakeLedger,
  FakeProcess,
  FakeProof,
  FakeWorkflows,
  FixedClock,
  type FakeRepository,
} from '../../../../test/model-gateway/harness.js'
import { decidePolicyFor } from './policy.js'
import { ModelsCommand } from './models.js'
import type { ModelGatewayDependencies } from './ports.js'
import { UsageCommand } from './usage.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

/** The defaults agent-kit ships, which the bundle reads from its own folder. */
const SHIPPED_DEFAULTS = join(import.meta.dirname, '../../../../../../skills/workstation/model-gateway/defaults.json')

const REPO = '/work/agent-kit'
const KEY = 'github.com/jorisjonkers-dev/agent-kit'

interface Workspace {
  readonly deps: ModelGatewayDependencies
  readonly home: string
  writeAllowlist(value: unknown): Promise<void>
}

async function workspace(repositories: Readonly<Record<string, FakeRepository>> = {}): Promise<Workspace> {
  const dir = await mkdtemp(join(tmpdir(), 'model-gateway-application-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const home = join(dir, 'home')
  const defaultsPath = join(dir, 'defaults.json')
  await writeFile(defaultsPath, await readFile(SHIPPED_DEFAULTS, 'utf8'))
  return {
    deps: {
      clock: new FixedClock(),
      defaultsPath,
      env: new FakeEnv({ HOME: home }),
      files: new FakeFiles(),
      git: new FakeGit(repositories),
      kept: new FakeKept(),
      ledger: new FakeLedger(),
      process: new FakeProcess(['claude', 'codex']),
      proof: new FakeProof(),
      sources: {},
      statuses: {},
      workflows: new FakeWorkflows(),
    },
    home,
    writeAllowlist: async (value) => {
      await mkdir(join(home, '.config', 'agent-kit'), { recursive: true })
      await writeFile(join(home, '.config', 'agent-kit', 'critique-allowlist.json'), JSON.stringify(value))
    },
  }
}

function allowlist(families: readonly string[], code = false, history = false): unknown {
  return { repositories: { [KEY]: { code, families, history, updated_at: '2026-11-04T00:00:00.000Z' } }, version: 1 }
}

/** The permitted client models of a decision, as `<client>:<model>` keys. */
function permitted(decision: Awaited<ReturnType<typeof decidePolicyFor>>): string[] {
  return decision.permissions
    .filter((permission) => permission.permitted)
    .map((permission) => `${permission.client}:${permission.model}`)
}

describe('decidePolicyFor', () => {
  it('decides for the repository the path sits in, from its allowlist entry', async () => {
    const w = await workspace({ [REPO]: { remotes: [{ name: 'origin', url: 'git@github.com:JorisJonkers-dev/agent-kit.git' }], root: REPO } })
    await w.writeAllowlist(allowlist(['anthropic'], true, false))

    const decision = await decidePolicyFor(w.deps, REPO)

    expect(permitted(decision)).toEqual(['claude:claude-opus-5-5', 'claude:claude-sonnet-5-5', 'claude:claude-haiku-5-5'])
    expect(decision.permissions.find((permission) => permission.client === 'claude')?.may_send).toEqual({ code: true, history: false })
    expect(decision.reason).toBe(`${KEY}: the entry allows anthropic`)
  })

  it('decides for inline content when it is given no repository', async () => {
    const w = await workspace()

    const decision = await decidePolicyFor(w.deps, undefined)

    expect(permitted(decision)).toContain('claude:claude-opus-5-5')
    expect(permitted(decision)).toContain('vibe:mistral-large-latest')
    expect(permitted(decision)).not.toContain('codex:gpt-5.5')
    expect(decision.permissions.every((permission) => permission.inline_only)).toBe(true)
  })

  it('decides for inline content when the path is outside any repository', async () => {
    const w = await workspace()

    expect((await decidePolicyFor(w.deps, '/tmp/nowhere')).reason).toBe(
      'no repository: only inline content, to subsidised and metered-personal clients',
    )
  })
})

describe('ModelsCommand', () => {
  it('refuses input that is not a JSON object', async () => {
    const w = await workspace()

    await expect(new ModelsCommand(w.deps).run([] as unknown as Record<string, never>)).rejects.toThrow(
      'the input must be a JSON object',
    )
  })
})

describe('UsageCommand', () => {
  it('refuses input that is not a JSON object', async () => {
    const w = await workspace()

    await expect(new UsageCommand(w.deps).run([])).rejects.toThrow('the input must be a JSON object')
  })
})
