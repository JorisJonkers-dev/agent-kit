import { createHash } from 'node:crypto'
import { dirname, join } from 'node:path'

import {
  codexHookHash,
  codexHookKey,
  codexHookStates,
  CRITIQUE_HOOKS,
  wiredHooks,
  type CodexHookState,
  type CritiqueHookName,
  type WiredHook,
} from '../domain/agent-hooks.js'
import { isJsonObject } from '../domain/json.js'
import { CritiqueError } from '../domain/outcomes.js'
import type { SetupOutcome } from '../domain/setup.js'
import { GateCommand } from './gate.js'
import { claudeHomePath, codexHomePath, critiqueConfigPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'

/** What a machine check found, in one line. */
export interface MachineCheckOutcome {
  readonly result: SetupOutcome
  readonly detail: string
}

/** One agent home the hooks are installed into, and the file that wires them. */
interface AgentHome {
  readonly label: string
  readonly home: string
  readonly hooksFile: string
  /** Codex's config.toml, which holds the owner's trust per hook; none for Claude Code, which runs hooks as configured. */
  readonly trustFile?: string
}

/**
 * Whether the three critique hooks are installed: in Claude Code's
 * settings.json and Codex's hooks.json, each naming a script that exists,
 * and in Codex also trusted by the owner in `/hooks` at the position it now
 * holds. It reads files only; no agent is launched.
 */
export class InstalledHooks {
  constructor(private readonly deps: Pick<CritiqueDeps, 'env' | 'files'>) {}

  /** sha256 of every file the check reads, so the item is due again when one changes. */
  async filesHash(): Promise<string> {
    const read: (string | boolean | null)[] = []
    for (const agent of this.homes()) {
      read.push(await this.deps.files.exists(agent.home))
      for (const path of [agent.hooksFile, agent.trustFile]) if (path !== undefined) read.push(await this.text(path))
    }
    return sha256(read)
  }

  async check(): Promise<MachineCheckOutcome> {
    const parts: string[] = []
    let present = 0
    let failed = false
    for (const agent of this.homes()) {
      if (!(await this.deps.files.exists(agent.home))) {
        parts.push(`${agent.label}: not set up here.`)
        continue
      }
      present += 1
      const problems = await this.problems(agent)
      failed ||= problems.length > 0
      const fine = agent.trustFile === undefined ? 'all three hooks installed' : 'all three hooks installed and trusted'
      parts.push(`${agent.label}: ${problems.length === 0 ? fine : problems.join('; ')}.`)
    }
    if (present === 0) {
      return { detail: 'Neither a Claude Code home nor a Codex home is set up on this machine, so no hook can run.', result: 'fail' }
    }
    return { detail: parts.join(' '), result: failed ? 'fail' : 'pass' }
  }

  private homes(): readonly AgentHome[] {
    const claude = claudeHomePath(this.deps.env)
    const codex = codexHomePath(this.deps.env)
    return [
      { home: claude, hooksFile: join(claude, 'settings.json'), label: 'Claude Code' },
      { home: codex, hooksFile: join(codex, 'hooks.json'), label: 'Codex', trustFile: join(codex, 'config.toml') },
    ]
  }

  /** What keeps the hooks from running in one agent home, in the order the owner fixes them; none when they run. */
  private async problems(agent: AgentHome): Promise<readonly string[]> {
    const text = await this.text(agent.hooksFile)
    if (text === null) return [`${agent.hooksFile} does not exist`]
    let file: unknown
    try {
      file = JSON.parse(text)
    } catch {
      return [`${agent.hooksFile} does not parse`]
    }
    const wired = wiredHooks(file)
    const missing: CritiqueHookName[] = []
    const scriptless: CritiqueHookName[] = []
    const running: WiredHook[] = []
    for (const { name } of CRITIQUE_HOOKS) {
      const hook = wired.get(name)
      if (hook === undefined) missing.push(name)
      else if (!(await this.deps.files.exists(hook.script))) scriptless.push(name)
      else running.push(hook)
    }
    const problems = [
      clause(missing, 'is not installed', 'are not installed'),
      clause(scriptless, 'names a script that does not exist', 'name scripts that do not exist'),
    ]
    if (agent.trustFile !== undefined) {
      const states = codexHookStates((await this.text(agent.trustFile)) ?? '')
      const trust = (status: CodexTrust) =>
        running.filter((hook) => codexTrust(states.get(codexHookKey(agent.hooksFile, hook)), codexHookHash(hook.event, hook.matcher, hook.handler)) === status).map((hook) => hook.name)
      problems.push(
        clause(trust('modified'), 'changed since it was trusted', 'changed since they were trusted'),
        clause(trust('untrusted'), 'is not trusted yet', 'are not trusted yet'),
        clause(trust('disabled'), 'is turned off in /hooks', 'are turned off in /hooks'),
      )
    }
    return problems.filter((problem) => problem !== '')
  }

  private async text(path: string): Promise<string | null> {
    return (await this.deps.files.exists(path)) ? this.deps.files.readText(path) : null
  }
}

type CodexTrust = 'trusted' | 'untrusted' | 'modified' | 'disabled'

/** Codex runs a user hook only while it is enabled and its trusted hash is its current one. */
function codexTrust(state: CodexHookState | undefined, hash: string): CodexTrust {
  if (state?.enabled === false) return 'disabled'
  if (state?.trusted_hash === undefined) return 'untrusted'
  return state.trusted_hash === hash ? 'trusted' : 'modified'
}

/** "a is …" or "a, b and c are …"; empty for no names. */
function clause(names: readonly string[], one: string, many: string): string {
  if (names.length === 0) return ''
  if (names.length === 1) return `${names[0] ?? ''} ${one}`
  return `${names.slice(0, -1).join(', ')} and ${names.at(-1) ?? ''} ${many}`
}

/**
 * The kill switch: `CRITIQUE_OFF=1`, or `"disabled": true` in this
 * machine's critique configuration, makes every hook a no-op. The check asks
 * the gate itself, with a command it never gates, so it reads the switch
 * exactly as the hooks do.
 */
export class KillSwitch {
  private readonly gate: GateCommand

  constructor(private readonly deps: CritiqueDeps) {
    this.gate = new GateCommand(deps)
  }

  async filesHash(): Promise<string> {
    const path = critiqueConfigPath(this.deps.env)
    const config = (await this.deps.files.exists(path)) ? await this.deps.files.readText(path) : null
    return sha256({ config, off: this.deps.env.get('CRITIQUE_OFF') ?? null })
  }

  async check(): Promise<MachineCheckOutcome> {
    const path = critiqueConfigPath(this.deps.env)
    const probe = await this.gate.run({ cwd: dirname(path), tool_input: { command: 'true' }, tool_name: 'Bash' })
    if (probe.reason === 'off') return { detail: 'Off: CRITIQUE_OFF=1 is set in this environment.', result: 'pass' }
    if (probe.reason === 'disabled') return { detail: `Off: ${path} disables every hook.`, result: 'pass' }
    if (probe.reason === 'failed-open') {
      return { detail: `${path} does not parse, so every hook fails open and logs: ${probe.detail ?? ''}`, result: 'fail' }
    }
    return { detail: `On: the hooks run. CRITIQUE_OFF=1, or {"disabled": true} in ${path}, turns every one of them off.`, result: 'pass' }
  }

  /** Writes the disabled flag, keeping the configuration's other keys. A file that is not a JSON object is left as it is. */
  async set(disabled: boolean): Promise<string> {
    const path = critiqueConfigPath(this.deps.env)
    let config: Readonly<Record<string, unknown>> = {}
    if (await this.deps.files.exists(path)) {
      const value = parseOrUndefined(await this.deps.files.readText(path))
      if (!isJsonObject(value)) {
        throw new CritiqueError('invalid-input', `setup input: ${path} is not a JSON object; fix or remove it before turning the kill switch`)
      }
      config = value
    }
    await this.deps.files.writeText(path, `${JSON.stringify({ ...config, disabled }, null, 2)}\n`)
    return path
  }
}

function parseOrUndefined(text: string): unknown {
  try {
    return JSON.parse(text) as unknown
  } catch {
    return undefined
  }
}

function sha256(value: unknown): string {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex')
}
