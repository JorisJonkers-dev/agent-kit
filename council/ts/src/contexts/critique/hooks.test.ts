import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { harness, type Harness } from '../../../test/critique/harness.js'
import { codexHookHash, type SessionStartResult, type SetupCheckOutput, type SetupItem, type SetupKillSwitchOutput } from './index.js'

/**
 * The critique hooks on this machine (#198): `/flow setup`'s two hook items,
 * "the hooks are installed" (in Claude Code and Codex, and trusted in Codex)
 * and "the kill switch turns them off", and the session-start hook's
 * decision, which the runner makes so the rules live in one tested place.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

type HarnessOptions = NonNullable<Parameters<typeof harness>[0]>

async function machine(options: HarnessOptions = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  return h
}

const HOOKS = ['critique-commit-gate', 'critique-pr-gate', 'critique-session-start'] as const
type HookName = (typeof HOOKS)[number]

function claudeHome(h: Harness): string {
  return join(h.home, '.claude')
}

function codexHome(h: Harness): string {
  return join(h.home, '.codex')
}

/** The command setup wires for one hook: its script in the agent home's critique skill, run by sh. */
function command(home: string, name: HookName): string {
  return `/bin/sh "${join(home, 'skills', 'critique', 'hooks', `${name}.sh`)}"`
}

function group(home: string, name: HookName): Record<string, unknown> {
  const handler = { command: command(home, name), timeout: 5, type: 'command' }
  return name === 'critique-session-start' ? { hooks: [handler] } : { hooks: [handler], matcher: 'Bash' }
}

const OPERATOR_GROUP = { hooks: [{ command: 'node "/opt/own/audit-hook.js"', type: 'command' }], matcher: 'Bash' }

/** The hooks section setup writes: one group per hook, after any `before` groups the operator already had. */
function hooksSection(home: string, names: readonly HookName[] = HOOKS, before: readonly unknown[] = []): Record<string, unknown> {
  const pre = names.filter((name) => name !== 'critique-session-start').map((name) => group(home, name))
  const start = names.filter((name) => name === 'critique-session-start').map((name) => group(home, name))
  return {
    ...(pre.length + before.length > 0 ? { PreToolUse: [...before, ...pre] } : {}),
    ...(start.length > 0 ? { SessionStart: start } : {}),
  }
}

/** Installs the hooks into an agent home as setup does: the scripts in its critique skill and the hook file naming them. */
async function install(h: Harness, home: string, file: string, options: { names?: readonly HookName[]; before?: readonly unknown[]; scripts?: boolean } = {}): Promise<void> {
  if (options.scripts ?? true) {
    for (const name of HOOKS) await h.writeText(join(home, 'skills', 'critique', 'hooks', `${name}.sh`), '#!/bin/sh\n')
  }
  await h.writeText(join(home, file), `${JSON.stringify({ hooks: hooksSection(home, options.names, options.before), model: 'opus' }, null, 2)}\n`)
}

async function installClaude(h: Harness, options: Parameters<typeof install>[3] = {}): Promise<void> {
  await install(h, claudeHome(h), 'settings.json', options)
}

async function installCodex(h: Harness, options: Parameters<typeof install>[3] = {}): Promise<void> {
  await install(h, codexHome(h), 'hooks.json', options)
}

/** Codex's per-hook state, as `/hooks` writes it into config.toml: keyed by the hook file, event and position. */
function trustTable(h: Harness, event: 'pre_tool_use' | 'session_start', groupIndex: number, fields: string): string {
  return `[hooks.state."${join(codexHome(h), 'hooks.json')}:${event}:${String(groupIndex)}:0"]\n${fields}\n`
}

/** Trusts each of the three hooks at the position `install` put it, as the owner does in Codex's `/hooks`. */
async function trustAll(h: Harness, offset = 0): Promise<void> {
  const hash = (name: HookName) => {
    const g = group(codexHome(h), name) as { hooks: Record<string, unknown>[]; matcher?: string }
    return codexHookHash(name === 'critique-session-start' ? 'SessionStart' : 'PreToolUse', g.matcher, g.hooks[0] ?? {})
  }
  await h.writeText(
    join(codexHome(h), 'config.toml'),
    [
      'model = "gpt-5.5"\n',
      '[hooks.state]\n',
      trustTable(h, 'pre_tool_use', offset, `trusted_hash = "${hash('critique-commit-gate')}"`),
      trustTable(h, 'pre_tool_use', offset + 1, `trusted_hash = "${hash('critique-pr-gate')}"`),
      trustTable(h, 'session_start', 0, `trusted_hash = "${hash('critique-session-start')}"`),
    ].join('\n'),
  )
}

async function check(h: Harness, id: string, extra: Record<string, unknown> = {}): Promise<SetupCheckOutput> {
  const result = await h.commands.setup.run({ action: 'check', check: id, ...extra })
  if (result.status === 'listed' || result.status === 'quota-set' || result.status === 'kill-switch-set') throw new Error(`not a check: ${JSON.stringify(result)}`)
  return result
}

async function hooksItem(h: Harness): Promise<SetupItem> {
  return (await check(h, 'hooks-installed', { force: true })).item
}

async function listItem(h: Harness, id: string): Promise<SetupItem> {
  const result = await h.commands.setup.run({ action: 'list' })
  if (result.status !== 'listed') throw new Error('not listed')
  const found = result.items.find((entry) => entry.id === id)
  if (found === undefined) throw new Error(`no ${id}`)
  return found
}

describe("Codex's hook trust hash", () => {
  // The hashes Codex 0.166.0's `hooks/list` reported for these exact hooks
  // (`codex app-server` against a throwaway CODEX_HOME, 2026-11-01).
  it('is the hash Codex trusts for a PreToolUse hook on Bash', () => {
    const handler = { command: '/bin/sh "/h/.codex/skills/critique/hooks/critique-commit-gate.sh"', timeout: 5, type: 'command' }

    expect(codexHookHash('PreToolUse', 'Bash', handler)).toBe('sha256:3676f9a340541f3acf1cdcd55338ff583125a13b57999bb68753df578c47913f')
  })

  it('is the hash Codex trusts for a SessionStart hook with no matcher', () => {
    const handler = { command: '/bin/sh "/h/.codex/skills/critique/hooks/critique-session-start.sh"', timeout: 5, type: 'command' }

    expect(codexHookHash('SessionStart', undefined, handler)).toBe('sha256:3a6665365aed320d7ddc6d6b29933134aaa457e4737764c3b082dcba5f690d47')
  })

  it('counts a missing timeout as the 600 seconds Codex defaults to, and drops the default context limit', () => {
    const bare = { command: 'x', type: 'command' }

    expect(codexHookHash('PreToolUse', 'Bash', bare)).toBe(codexHookHash('PreToolUse', 'Bash', { ...bare, additionalContextLimit: 2500, async: false, timeout: 600 }))
    expect(codexHookHash('PreToolUse', 'Bash', bare)).not.toBe(codexHookHash('PreToolUse', 'Bash', { ...bare, statusMessage: 'gating', timeout: 0 }))
    expect(codexHookHash('PreToolUse', 'Bash', bare)).not.toBe(codexHookHash('PreToolUse', 'Bash', { ...bare, additionalContextLimit: 100 }))
  })
})

describe('the hooks-installed item', () => {
  it('is free, and comes after the read-only proofs with the kill switch', async () => {
    const h = await machine()

    const item = await listItem(h, 'hooks-installed')

    expect([item.cost, item.kind, item.status, item.due_because]).toEqual(['free', 'hooks', 'due', 'never-checked'])
  })

  it('fails on a machine with neither a Claude Code nor a Codex home', async () => {
    const h = await machine()

    expect(await hooksItem(h)).toMatchObject({
      last: { detail: 'Neither a Claude Code home nor a Codex home is set up on this machine, so no hook can run.', result: 'fail' },
    })
  })

  it('passes once Claude Code wires all three hooks to scripts that exist, with no Codex home to check', async () => {
    const h = await machine()
    await installClaude(h)

    expect((await hooksItem(h)).last).toMatchObject({ detail: 'Claude Code: all three hooks installed. Codex: not set up here.', result: 'pass' })
  })

  it('names the hook Claude Code is missing', async () => {
    const h = await machine()
    await installClaude(h, { names: ['critique-commit-gate', 'critique-session-start'] })

    expect((await hooksItem(h)).last).toMatchObject({ detail: 'Claude Code: critique-pr-gate is not installed. Codex: not set up here.', result: 'fail' })
  })

  it('fails when a hook names a script that is not there', async () => {
    const h = await machine()
    await installClaude(h, { scripts: false })

    expect((await hooksItem(h)).last?.detail).toBe(
      'Claude Code: critique-commit-gate, critique-pr-gate and critique-session-start name scripts that do not exist. Codex: not set up here.',
    )
  })

  it('fails when a home has no hook file', async () => {
    const h = await machine()
    await h.writeText(join(claudeHome(h), 'CLAUDE.md'), '')

    expect((await hooksItem(h)).last?.detail).toBe(`Claude Code: ${join(claudeHome(h), 'settings.json')} does not exist. Codex: not set up here.`)
  })

  it('fails when the hook file does not parse', async () => {
    const h = await machine()
    await h.writeText(join(claudeHome(h), 'settings.json'), '{"hooks": ')

    expect((await hooksItem(h)).last?.detail).toBe(`Claude Code: ${join(claudeHome(h), 'settings.json')} does not parse. Codex: not set up here.`)
  })

  it('finds no hook in a file with no hooks section, or a PreToolUse hook not on Bash', async () => {
    const h = await machine()
    await h.writeText(
      join(claudeHome(h), 'settings.json'),
      JSON.stringify({ hooks: { PreToolUse: [null, { matcher: 'Bash' }, { hooks: ['x'], matcher: 'Bash' }], SessionStart: 'x' } }),
    )
    await installCodex(h)
    const codex = join(codexHome(h), 'hooks.json')
    const file = JSON.parse(await h.readText(codex)) as { hooks: { PreToolUse: { matcher?: string }[] } }
    for (const entry of file.hooks.PreToolUse) entry.matcher = 'Edit'
    await h.writeText(codex, JSON.stringify(file))

    expect((await hooksItem(h)).last?.detail).toBe(
      'Claude Code: critique-commit-gate, critique-pr-gate and critique-session-start are not installed. Codex: critique-commit-gate and critique-pr-gate are not installed; critique-session-start is not trusted yet.',
    )
  })

  it("fails until the owner trusts Codex's hooks in /hooks, naming them", async () => {
    const h = await machine()
    await installClaude(h)
    await installCodex(h)

    expect((await hooksItem(h)).last).toMatchObject({
      detail: 'Claude Code: all three hooks installed. Codex: critique-commit-gate, critique-pr-gate and critique-session-start are not trusted yet.',
      result: 'fail',
    })
  })

  it('passes once Codex trusts the three hooks at the positions it found them', async () => {
    const h = await machine()
    await installClaude(h)
    await installCodex(h)
    await trustAll(h)

    expect((await hooksItem(h)).last).toMatchObject({ detail: 'Claude Code: all three hooks installed. Codex: all three hooks installed and trusted.', result: 'pass' })
  })

  it("reads Codex's trust at each hook's own position among the operator's hooks", async () => {
    const h = await machine()
    await installCodex(h, { before: [OPERATOR_GROUP] })
    await trustAll(h, 1)

    expect((await hooksItem(h)).last?.detail).toBe('Claude Code: not set up here. Codex: all three hooks installed and trusted.')
  })

  it('says a hook changed since Codex trusted it, and one the owner turned off in /hooks', async () => {
    const h = await machine()
    await installCodex(h)
    await trustAll(h)
    const config = join(codexHome(h), 'config.toml')
    const toml = (await h.readText(config))
      .replace(/(pre_tool_use:0:0"\]\ntrusted_hash = ")sha256:[0-9a-f]+/, '$1sha256:0000')
      .replace(/(session_start:0:0"\]\n)/, '$1enabled = false\n')
    await h.writeText(config, toml)

    expect((await hooksItem(h)).last?.detail).toBe(
      'Claude Code: not set up here. Codex: critique-commit-gate changed since it was trusted; critique-session-start is turned off in /hooks.',
    )
  })

  it("needs Codex's trust again when the hook was trusted at another position", async () => {
    const h = await machine()
    await installCodex(h, { before: [OPERATOR_GROUP] })
    await trustAll(h)

    expect((await hooksItem(h)).last?.detail).toBe('Claude Code: not set up here. Codex: critique-commit-gate changed since it was trusted; critique-pr-gate is not trusted yet.')
  })

  it('reads the homes CLAUDE_CONFIG_DIR and CODEX_HOME name', async () => {
    const h = await machine()
    const elsewhere = join(h.home, 'elsewhere')
    await install(h, join(elsewhere, 'claude'), 'settings.json')
    await install(h, join(elsewhere, 'codex'), 'hooks.json')
    const moved = await machine({ env: { CLAUDE_CONFIG_DIR: join(elsewhere, 'claude'), CODEX_HOME: join(elsewhere, 'codex') } })
    const shared = { ...moved, home: h.home }

    expect((await hooksItem(shared)).last?.detail).toBe('Claude Code: all three hooks installed. Codex: critique-commit-gate, critique-pr-gate and critique-session-start are not trusted yet.')
  })

  it('is current once it passed, and due again after a hook file changed', async () => {
    const h = await machine()
    await installClaude(h)
    await hooksItem(h)

    const current = (await listItem(h, 'hooks-installed')).status
    await installClaude(h, { names: ['critique-commit-gate'] })

    expect([current, (await listItem(h, 'hooks-installed')).due_because]).toEqual(['current', 'inputs-changed'])
  })
})

async function setKillSwitch(h: Harness, disabled: unknown): Promise<SetupKillSwitchOutput> {
  const result = await h.commands.setup.run({ action: 'kill-switch', disabled })
  if (result.status !== 'kill-switch-set') throw new Error(`not set: ${JSON.stringify(result)}`)
  return result
}

function configPath(h: Harness): string {
  return join(h.home, '.config', 'agent-kit', 'critique.json')
}

describe('the kill-switch item', () => {
  it('passes and says the hooks run while nothing turns them off', async () => {
    const h = await machine()

    expect((await check(h, 'kill-switch')).item).toMatchObject({
      cost: 'free',
      kind: 'kill-switch',
      last: {
        detail: `On: the hooks run. CRITIQUE_OFF=1, or {"disabled": true} in ${configPath(h)}, turns every one of them off.`,
        result: 'pass',
      },
    })
  })

  it("turns the hooks off through the per-machine configuration, keeping the file's other keys, and the gate reads it", async () => {
    const h = await machine()
    await h.writeText(configPath(h), '{"note": "mine"}\n')

    const result = await setKillSwitch(h, true)

    expect(result).toMatchObject({ config_path: configPath(h), disabled: true, item: { last: { detail: `Off: ${configPath(h)} disables every hook.`, result: 'pass' } } })
    expect(JSON.parse(await h.readText(configPath(h)))).toEqual({ disabled: true, note: 'mine' })
    expect(await h.commands.gate.run({ cwd: h.repo, tool_input: { command: 'git commit -m x' }, tool_name: 'Bash' })).toMatchObject({ reason: 'disabled' })
  })

  it('turns them back on', async () => {
    const h = await machine()
    await setKillSwitch(h, true)

    const result = await setKillSwitch(h, false)

    expect([result.item.last?.detail.startsWith('On: the hooks run.'), JSON.parse(await h.readText(configPath(h)))]).toEqual([true, { disabled: false }])
  })

  it('says CRITIQUE_OFF=1 has them off in this environment', async () => {
    const h = await machine({ env: { CRITIQUE_OFF: '1' } })

    expect((await check(h, 'kill-switch')).item.last?.detail).toBe('Off: CRITIQUE_OFF=1 is set in this environment.')
  })

  it('fails while the configuration does not parse, since every hook then fails open and logs', async () => {
    const h = await machine()
    await h.writeText(configPath(h), '{"disabled": "yes"}')

    expect((await check(h, 'kill-switch')).item.last).toMatchObject({
      detail: expect.stringContaining(`${configPath(h)} does not parse, so every hook fails open and logs:`) as unknown,
      result: 'fail',
    })
  })

  it('writes nothing over a configuration that is not a JSON object', async () => {
    const h = await machine()
    await h.writeText(configPath(h), '[1]')

    await expect(h.commands.setup.run({ action: 'kill-switch', disabled: true })).rejects.toMatchObject({
      code: 'invalid-input',
      message: `setup input: ${configPath(h)} is not a JSON object; fix or remove it before turning the kill switch`,
    })
    expect(await h.readText(configPath(h))).toBe('[1]')
  })

  it('writes nothing over a configuration that is not JSON', async () => {
    const h = await machine()
    await h.writeText(configPath(h), '{')

    await expect(h.commands.setup.run({ action: 'kill-switch', disabled: true })).rejects.toMatchObject({ code: 'invalid-input' })
    expect(await h.readText(configPath(h))).toBe('{')
  })

  it('needs a boolean', async () => {
    const h = await machine()

    await expect(h.commands.setup.run({ action: 'kill-switch', disabled: 'yes' })).rejects.toMatchObject({ code: 'invalid-input' })
  })

  it('is due again after the configuration changed', async () => {
    const h = await machine()
    await check(h, 'kill-switch')
    await h.writeText(configPath(h), '{"disabled": true}')

    expect((await listItem(h, 'kill-switch')).due_because).toBe('inputs-changed')
  })
})

/** A setup results file in which every listed check but the read-only proofs has the given result. */
async function recordSetup(h: Harness, result: (id: string) => 'pass' | 'fail' | undefined): Promise<void> {
  const listed = await h.commands.setup.run({ action: 'list' })
  if (listed.status !== 'listed') throw new Error('not listed')
  const checks: Record<string, unknown> = {}
  for (const entry of listed.items) {
    const outcome = entry.kind === 'read-only' ? undefined : result(entry.id)
    if (outcome !== undefined) checks[entry.id] = { checked_at: '2026-10-30T11:00:00.000Z', detail: 'x', inputs: {}, result: outcome }
  }
  await h.writeText(listed.results_path, JSON.stringify({ checks, version: 1 }))
}

async function optedIn(options: HarnessOptions = {}): Promise<Harness> {
  const h = await machine(options)
  await h.commands.allow.run({ code: false, families: ['mistral'], repo: h.repo })
  return h
}

function sessionStart(h: Harness, cwd = h.repo): Promise<SessionStartResult> {
  return h.commands.sessionStart.run({ cwd, hook_event_name: 'SessionStart', source: 'startup' })
}

describe('the session-start hook', () => {
  it('suggests /flow setup in one line in an opted-in repository this machine has not set up', async () => {
    const h = await optedIn()

    expect(await sessionStart(h)).toEqual({
      line: 'Critique: /flow setup has not checked this machine yet (10 of 10 checks never ran). Run /flow setup.',
      reason: 'setup-missing',
      status: 'suggest',
    })
  })

  it('counts the checks that never ran', async () => {
    const h = await optedIn()
    await recordSetup(h, (id) => (id.startsWith('critic:') ? undefined : 'pass'))

    expect((await sessionStart(h)).line).toBe('Critique: /flow setup has not checked this machine yet (5 of 10 checks never ran). Run /flow setup.')
  })

  it('names a failing check first', async () => {
    const h = await optedIn()
    await recordSetup(h, (id) => (id === 'hooks-installed' || id === 'agy-quota' ? 'fail' : undefined))

    expect(await sessionStart(h)).toEqual({
      line: "Critique: /flow setup's agy-quota check failed on this machine, and 1 more. Run /flow setup.",
      reason: 'setup-failing',
      status: 'suggest',
    })
  })

  it('names a single failing check alone', async () => {
    const h = await optedIn()
    await recordSetup(h, (id) => (id === 'kill-switch' ? 'fail' : 'pass'))

    expect((await sessionStart(h)).line).toBe("Critique: /flow setup's kill-switch check failed on this machine. Run /flow setup.")
  })

  it('stays silent once every check passed', async () => {
    const h = await optedIn()
    await recordSetup(h, () => 'pass')

    expect(await sessionStart(h)).toEqual({ line: null, reason: 'setup-current', status: 'silent' })
  })

  it('treats a repository with no allowlist entry like an opted-in one, and stays silent on no remote or a nedap remote', async () => {
    const plain = await machine()
    const lone = await machine({ remotes: [] })
    const nedap = await machine({ remotes: [{ name: 'origin', url: 'git@github.com:nedap/widgets.git' }] })

    expect([(await sessionStart(plain)).reason, (await sessionStart(lone)).reason, (await sessionStart(nedap)).reason]).toEqual(['setup-missing', 'no-remote', 'nedap'])
  })

  it('stays silent under the kill switch', async () => {
    const off = await optedIn({ env: { CRITIQUE_OFF: '1' } })
    const disabled = await optedIn()
    await setKillSwitch(disabled, true)

    expect([await sessionStart(off), (await sessionStart(disabled)).reason]).toEqual([{ line: null, reason: 'off', status: 'silent' }, 'disabled'])
  })

  it('fails open, silent, and logs when the setup results do not parse', async () => {
    const h = await optedIn()
    await h.writeText(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'machine-setup.json'), '{')

    const result = await sessionStart(h)
    const log = await h.readText(join(h.home, '.local', 'share', 'agent-kit', 'critique', 'gate.log.jsonl'))

    expect(result).toMatchObject({ failure: 'error', line: null, reason: 'failed-open', status: 'silent' })
    expect(JSON.parse(log)).toMatchObject({ cwd: h.repo, failure: 'error', gate: 'session-start' })
  })

  it('fails open when the payload names no directory', async () => {
    const h = await optedIn()

    expect(await h.commands.sessionStart.run('not a payload')).toMatchObject({ reason: 'failed-open', status: 'silent' })
  })

  it('fails open, silent, when git does not answer before the deadline', async () => {
    const h = await optedIn()
    h.git.remotesBehaviour = 'hang'
    const result = sessionStart(h)
    h.clock.wake()

    expect(await result).toMatchObject({ failure: 'deadline', reason: 'failed-open', status: 'silent' })
  })
})
