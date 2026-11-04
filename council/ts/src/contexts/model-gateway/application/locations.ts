import { isAbsolute, join } from 'node:path'

import type { EnvPort } from '../../../ports/index.js'

/**
 * This machine's override of the shipped defaults:
 * `$XDG_CONFIG_HOME/agent-kit/model-gateway.json`, or
 * `~/.config/agent-kit/model-gateway.json` when XDG_CONFIG_HOME is unset or
 * not an absolute path.
 */
export function overridePath(env: EnvPort): string {
  return join(configDir(env), 'model-gateway.json')
}

/**
 * The family allowlist critique writes and the gateway reads, resolved the
 * same way critique resolves it: `$XDG_CONFIG_HOME/agent-kit/`, else
 * `~/.config/agent-kit/`.
 */
export function allowlistPath(env: EnvPort): string {
  return join(configDir(env), 'critique-allowlist.json')
}

function configDir(env: EnvPort): string {
  const xdg = env.get('XDG_CONFIG_HOME')
  if (xdg !== undefined && isAbsolute(xdg)) return join(xdg, 'agent-kit')
  return join(env.require('HOME'), '.config', 'agent-kit')
}

/**
 * The configuration Vibe declares its own models in: `$VIBE_HOME/config.toml`,
 * or `~/.vibe/config.toml` when VIBE_HOME is unset or not an absolute path.
 */
export function vibeConfigPath(env: EnvPort): string {
  const home = env.get('VIBE_HOME')
  if (home !== undefined && isAbsolute(home)) return join(home, 'config.toml')
  return join(env.require('HOME'), '.vibe', 'config.toml')
}

/**
 * This machine's routing override, applied client by client over the routing
 * critique ships. It is resolved the way critique resolves it, so the gateway
 * reads the routing critique really runs.
 */
export function routingOverridePath(env: EnvPort): string {
  return join(env.require('HOME'), '.config', 'agent-kit', 'critique-routing.json')
}

/** Setup's recorded results, where critique's setup writes them: one result per check id. */
export function setupResultsPath(env: EnvPort): string {
  return join(env.require('HOME'), '.local', 'share', 'agent-kit', 'critique', 'machine-setup.json')
}

/**
 * The gateway's own state, outside every repository:
 * `$XDG_STATE_HOME/agent-kit/model-gateway`, or
 * `~/.local/state/agent-kit/model-gateway` when XDG_STATE_HOME is unset or
 * not an absolute path.
 */
export function stateDir(env: EnvPort): string {
  const xdg = env.get('XDG_STATE_HOME')
  if (xdg !== undefined && isAbsolute(xdg)) return join(xdg, 'agent-kit', 'model-gateway')
  return join(env.require('HOME'), '.local', 'state', 'agent-kit', 'model-gateway')
}

/** The machine-wide ledger: one JSON line per run, whichever repository it ran in. */
export function ledgerPath(env: EnvPort): string {
  return join(stateDir(env), 'ledger.jsonl')
}

/** The writing worktrees the gateway made and has not removed, machine-wide. */
export function keptWorktreesPath(env: EnvPort): string {
  return join(stateDir(env), 'kept-worktrees.json')
}

/** Where a run outside any repository keeps its directory. */
export function runsDir(env: EnvPort): string {
  return join(stateDir(env), 'runs')
}

/** The workflows this machine has started, and the moments each one started and ended. */
export function workflowsPath(env: EnvPort): string {
  return join(stateDir(env), 'workflows.json')
}
