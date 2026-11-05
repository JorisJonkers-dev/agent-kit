import { join } from 'node:path'

import type { EnvPort } from '../../../ports/index.js'
import { readToml } from '../../../shared-kernel/toml.js'
import { ModelGatewayError } from '../domain/config.js'
import { messageOf } from './json-file.js'

/** What reading a client's home needs: its files, and the environment its home is named in. */
export interface HomePorts {
  readonly files: { readText(path: string): Promise<string>; exists(path: string): Promise<boolean> }
  readonly env: EnvPort
}

/**
 * The MCP servers Vibe's home declares, by name, so a run can switch each of
 * them off: `~/.vibe/config.toml`. Vibe is the one client whose home the
 * gateway reads, and a home with no configuration declares none. A home the
 * gateway cannot read or parse is refused rather than read as declaring
 * nothing: a run that asked to switch the home's servers off would otherwise
 * leave every one of them live.
 */
export async function homeMcpServers(ports: HomePorts): Promise<readonly string[]> {
  const path = join(ports.env.require('HOME'), '.vibe', 'config.toml')
  if (!(await ports.files.exists(path))) return []
  try {
    return serverNames(await ports.files.readText(path))
  } catch (error) {
    throw new ModelGatewayError('home-config-unreadable', `${path}: cannot read the home's MCP servers: ${messageOf(error)}`)
  }
}

/**
 * The names of the MCP servers a Vibe `config.toml` declares, whether as
 * `[[mcp_servers]]` tables or an `mcp_servers = [...]` array of inline
 * tables.
 */
function serverNames(toml: string): readonly string[] {
  const servers = readToml(toml).mcp_servers
  return servers === undefined ? [] : namesOf(servers)
}

/** The `name` of each server in an `mcp_servers` value, leaving out one without a name. */
function namesOf(servers: unknown): readonly string[] {
  return (Array.isArray(servers) ? servers : [servers])
    .map((server) => (typeof server === 'object' && server !== null ? (server as Record<string, unknown>).name : undefined))
    .filter((name): name is string => typeof name === 'string')
}
