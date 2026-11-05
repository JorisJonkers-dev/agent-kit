import { applyOverride, parseDefaults, type GatewayConfig } from '../domain/config.js'
import { fileExists, readJsonFile } from './json-file.js'
import type { ModelGatewayFilesPort } from './ports.js'

/**
 * The shipped defaults with this machine's override applied over them. A
 * missing override is no override; an unreadable or invalid one is an error
 * that names the file.
 */
export async function loadConfig(files: ModelGatewayFilesPort, defaultsPath: string, override: string): Promise<GatewayConfig> {
  const defaults = await readJsonFile(files, defaultsPath, 'the shipped defaults', parseDefaults)
  if (!(await fileExists(files, override, 'the override'))) return defaults
  return readJsonFile(files, override, 'the override', (raw) => applyOverride(defaults, raw))
}
