import { ModelGatewayError } from '../domain/config.js'
import type { ModelGatewayFilesPort } from './ports.js'

/** Whether a file is there; any other failure to look is an error that names the file. */
export async function fileExists(files: ModelGatewayFilesPort, path: string, what: string): Promise<boolean> {
  try {
    return await files.exists(path)
  } catch (error) {
    throw new ModelGatewayError('invalid-config', `${path}: cannot read ${what}: ${messageOf(error)}`)
  }
}

/** One JSON file, parsed and validated, with its path in front of anything wrong with it. */
export async function readJsonFile<T>(
  files: ModelGatewayFilesPort,
  path: string,
  what: string,
  parse: (raw: unknown) => T,
): Promise<T> {
  let text: string
  try {
    text = await files.readText(path)
  } catch (error) {
    throw new ModelGatewayError('invalid-config', `${path}: cannot read ${what}: ${messageOf(error)}`)
  }
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

export function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
