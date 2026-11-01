import { createHash } from 'node:crypto'

import { isJsonObject, type JsonObject } from './json.js'

/** The events the critique hooks run on, in Claude Code's and Codex's spelling. */
export type HookEvent = 'PreToolUse' | 'SessionStart'

/**
 * The three hooks ADR-0002 allows. Setup wires each one as a matcher group
 * of its own whose one handler runs `<name>.sh` from the critique skill, so
 * a hook is found by its script's name.
 */
export const CRITIQUE_HOOKS = [
  { event: 'PreToolUse', matcher: 'Bash', name: 'critique-commit-gate' },
  { event: 'PreToolUse', matcher: 'Bash', name: 'critique-pr-gate' },
  { event: 'SessionStart', name: 'critique-session-start' },
] as const satisfies readonly { readonly name: string; readonly event: HookEvent; readonly matcher?: string }[]

export type CritiqueHookName = (typeof CRITIQUE_HOOKS)[number]['name']

/** One critique hook as an agent's hook file wires it. */
export interface WiredHook {
  readonly name: CritiqueHookName
  readonly event: HookEvent
  readonly matcher: string | undefined
  /** The position of its matcher group under its event, and of its handler in the group: Codex keys its trust by both. */
  readonly group_index: number
  readonly handler_index: number
  readonly handler: JsonObject
  /** The script the command runs, as the command names it. */
  readonly script: string
}

/**
 * Where an agent's hook file wires each critique hook: the first handler
 * under the hook's event whose command names `<name>.sh`, in a group that
 * matches `Bash` for a `PreToolUse` hook. A hook it does not wire is absent.
 */
export function wiredHooks(file: unknown): ReadonlyMap<CritiqueHookName, WiredHook> {
  const found = new Map<CritiqueHookName, WiredHook>()
  const hooks = isJsonObject(file) && isJsonObject(file.hooks) ? file.hooks : {}
  for (const hook of CRITIQUE_HOOKS) {
    const groups = hooks[hook.event]
    for (const [group_index, group] of (Array.isArray(groups) ? groups : []).entries()) {
      if (!isJsonObject(group)) continue
      const matcher = typeof group.matcher === 'string' ? group.matcher : undefined
      if ('matcher' in hook && matcher !== hook.matcher) continue
      const handlers: readonly unknown[] = Array.isArray(group.hooks) ? group.hooks : []
      const handler_index = handlers.findIndex((handler) => scriptOf(handler, hook.name) !== undefined)
      const handler = handlers[handler_index]
      const script = scriptOf(handler, hook.name)
      if (!isJsonObject(handler) || script === undefined) continue
      found.set(hook.name, { event: hook.event, group_index, handler, handler_index, matcher, name: hook.name, script })
      break
    }
  }
  return found
}

/** The token of a handler's command whose last path segment is `<name>.sh`, quotes removed, as the retired-hook purge reads one. */
function scriptOf(handler: unknown, name: string): string | undefined {
  if (!isJsonObject(handler) || typeof handler.command !== 'string') return undefined
  return handler.command
    .replaceAll('"', ' ')
    .split(/\s+/)
    .find((token) => token.split('/').at(-1) === `${name}.sh`)
}

const CODEX_EVENT_LABELS: Readonly<Record<HookEvent, string>> = { PreToolUse: 'pre_tool_use', SessionStart: 'session_start' }

/** Codex's default handler timeout, in seconds. */
const CODEX_DEFAULT_TIMEOUT_S = 600
/** Codex's default `additionalContextLimit`, which it leaves out of the hash. */
const CODEX_DEFAULT_CONTEXT_LIMIT = 2500

/**
 * The key Codex records a user hook's state under in config.toml's
 * `[hooks.state]`: the hook file, the event, and the hook's position. A
 * hook that moves to another position is a new hook to Codex.
 */
export function codexHookKey(hooksFile: string, hook: Pick<WiredHook, 'event' | 'group_index' | 'handler_index'>): string {
  return `${hooksFile}:${CODEX_EVENT_LABELS[hook.event]}:${String(hook.group_index)}:${String(hook.handler_index)}`
}

/**
 * The hash Codex trusts a command hook by (Codex 0.166.0,
 * `codex-rs/hooks/src/engine/discovery.rs` `hook_hash`): the sha256 of the
 * event, the matcher and the handler as Codex normalizes it, serialized as
 * JSON with sorted keys. A hook whose hash differs from the trusted one is
 * "modified" and does not run until it is trusted again.
 */
export function codexHookHash(event: HookEvent, matcher: string | undefined, handler: JsonObject): string {
  const timeout = typeof handler.timeout === 'number' ? Math.max(1, handler.timeout) : CODEX_DEFAULT_TIMEOUT_S
  const limit = handler.additionalContextLimit
  const normalized = {
    async: handler.async === true,
    command: handler.command,
    timeout,
    type: 'command',
    ...(typeof handler.statusMessage === 'string' ? { statusMessage: handler.statusMessage } : {}),
    ...(typeof limit === 'number' && limit !== CODEX_DEFAULT_CONTEXT_LIMIT ? { additionalContextLimit: limit } : {}),
  }
  const identity = { event_name: CODEX_EVENT_LABELS[event], hooks: [normalized], ...(matcher === undefined ? {} : { matcher }) }
  return `sha256:${createHash('sha256').update(JSON.stringify(sortKeys(identity))).digest('hex')}`
}

function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortKeys)
  if (!isJsonObject(value)) return value
  return Object.fromEntries(
    Object.keys(value)
      .sort()
      .map((key) => [key, sortKeys(value[key])]),
  )
}

/** One hook's state in Codex's config.toml: the hash the owner trusted, and whether `/hooks` turned it off. */
export interface CodexHookState {
  readonly trusted_hash?: string
  readonly enabled?: boolean
}

/**
 * The `[hooks.state."<key>"]` tables of a Codex config.toml, the form Codex
 * itself writes when the owner trusts a hook in `/hooks`. Only the two keys
 * the check needs are read; anything else in the file is ignored.
 */
export function codexHookStates(toml: string): ReadonlyMap<string, CodexHookState> {
  const states = new Map<string, { trusted_hash?: string; enabled?: boolean }>()
  let current: { trusted_hash?: string; enabled?: boolean } | undefined
  for (const raw of toml.split('\n')) {
    const line = raw.trim()
    const table = /^\[\s*hooks\.state\."((?:[^"\\]|\\.)*)"\s*\]$/.exec(line)
    if (table !== null) {
      current = {}
      states.set(String(table[1]), current)
      continue
    }
    if (line.startsWith('[')) current = undefined
    if (current === undefined) continue
    const trusted = /^trusted_hash\s*=\s*"([^"]*)"/.exec(line)
    const enabled = /^enabled\s*=\s*(true|false)\b/.exec(line)
    if (trusted !== null) current.trusted_hash = String(trusted[1])
    if (enabled !== null) current.enabled = enabled[1] === 'true'
  }
  return states
}
