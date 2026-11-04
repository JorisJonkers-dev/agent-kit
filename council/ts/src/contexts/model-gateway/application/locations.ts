import { isAbsolute, join } from 'node:path'

import type { EnvPort } from '../../../ports/index.js'

/**
 * This machine's override of the shipped defaults:
 * `$XDG_CONFIG_HOME/agent-kit/model-gateway.json`, or
 * `~/.config/agent-kit/model-gateway.json` when XDG_CONFIG_HOME is unset or
 * not an absolute path.
 */
export function overridePath(env: EnvPort): string {
  const xdg = env.get('XDG_CONFIG_HOME')
  if (xdg !== undefined && isAbsolute(xdg)) return join(xdg, 'agent-kit', 'model-gateway.json')
  return join(env.require('HOME'), '.config', 'agent-kit', 'model-gateway.json')
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
