import { applyOverride, ModelGatewayError, parseDefaults, type GatewayConfig } from '../domain/config.js'
import type { ModelGatewayFilesPort } from './ports.js'

/**
 * The shipped defaults with this machine's override applied over them. A
 * missing override is no override; an unreadable or invalid one is an error
 * that names the file.
 */
export async function loadConfig(files: ModelGatewayFilesPort, defaultsPath: string, override: string): Promise<GatewayConfig> {
  const defaults = parseFile(defaultsPath, await readText(files, defaultsPath, 'the shipped defaults'), parseDefaults)
  if (!(await overrideExists(files, override))) return defaults
  return parseFile(override, await readText(files, override, 'the override'), (raw) => applyOverride(defaults, raw))
}

async function overrideExists(files: ModelGatewayFilesPort, path: string): Promise<boolean> {
  try {
    return await files.exists(path)
  } catch (error) {
    throw new ModelGatewayError('invalid-config', `${path}: cannot read the override: ${messageOf(error)}`)
  }
}

async function readText(files: ModelGatewayFilesPort, path: string, what: string): Promise<string> {
  try {
    return await files.readText(path)
  } catch (error) {
    throw new ModelGatewayError('invalid-config', `${path}: cannot read ${what}: ${messageOf(error)}`)
  }
}

/** One configuration file, parsed and validated, with its path in front of anything wrong with it. */
function parseFile(path: string, text: string, parse: (raw: unknown) => GatewayConfig): GatewayConfig {
  try {
    return parse(parseJson(text))
  } catch (error) {
    throw new ModelGatewayError('invalid-config', `${path}: ${messageOf(error)}`)
  }
}

function parseJson(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch (error) {
    throw new ModelGatewayError('invalid-config', `not JSON: ${messageOf(error)}`)
  }
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
