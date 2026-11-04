import { join } from 'node:path'

import type { EnvPort } from '../../../ports/index.js'
import { readToml } from '../../../shared-kernel/toml.js'

/** What reading a client's home needs: its files, and the environment its home is named in. */
export interface HomePorts {
  readonly files: { readText(path: string): Promise<string>; exists(path: string): Promise<boolean> }
  readonly env: EnvPort
}

/**
 * The MCP servers Vibe's home declares, by name, so a run can switch each of
 * them off: `~/.vibe/config.toml`. Vibe is the one client whose home the
 * gateway reads, and a home with no configuration declares none.
 */
export async function homeMcpServers(ports: HomePorts): Promise<readonly string[]> {
  const path = join(ports.env.require('HOME'), '.vibe', 'config.toml')
  if (!(await ports.files.exists(path))) return []
  return serverNames(await ports.files.readText(path))
}

/**
 * The names of the MCP servers a Vibe `config.toml` declares, whether as
 * `[[mcp_servers]]` tables or an `mcp_servers = [...]` array of inline
 * tables; none for a file that is not TOML, which the caller's own check has
 * already refused.
 */
function serverNames(toml: string): readonly string[] {
  try {
    const servers = readToml(toml).mcp_servers
    return servers === undefined ? [] : namesOf(servers)
  } catch {
    return []
  }
}

/** The `name` of each server in an `mcp_servers` value, leaving out one without a name. */
function namesOf(servers: unknown): readonly string[] {
  return (Array.isArray(servers) ? servers : [servers])
    .map((server) => (typeof server === 'object' && server !== null ? (server as Record<string, unknown>).name : undefined))
    .filter((name): name is string => typeof name === 'string')
}
