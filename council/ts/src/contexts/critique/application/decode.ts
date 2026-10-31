import { JsonFields, JsonShapeError } from '../domain/json.js'
import { CritiqueError, type CritiqueErrorCode } from '../domain/outcomes.js'
import type { CritiqueFilesPort } from './ports.js'

/** Runs `read`; JSON that does not parse or has the wrong shape becomes a CritiqueError with `code`. */
export function decoded<T>(code: CritiqueErrorCode, where: string, read: () => T): T {
  try {
    return read()
  } catch (error) {
    throw error instanceof JsonShapeError || error instanceof SyntaxError
      ? new CritiqueError(code, `${where}: ${error.message}`)
      : error
  }
}

/** Reads the JSON object a command was given. */
export function readInput<T>(raw: unknown, command: string, read: (fields: JsonFields) => T): T {
  return decoded('invalid-input', `${command} input`, () => read(JsonFields.of(raw, '')))
}

/** Reads one JSON file the runner relies on; a missing or malformed file is a CritiqueError with `code`. */
export async function readJsonFile<T>(
  files: CritiqueFilesPort,
  path: string,
  code: CritiqueErrorCode,
  parse: (text: string) => T,
): Promise<T> {
  if (!(await files.exists(path))) throw new CritiqueError(code, `${path} does not exist`)
  const text = await files.readText(path)
  return decoded(code, path, () => parse(text))
}
