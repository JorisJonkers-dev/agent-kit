/** Small readers for the JSON each command receives. A bad field throws with its name. */

export type JsonObject = Readonly<Record<string, unknown>>

export function asObject(value: unknown, what: string): JsonObject {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error(`${what} must be a JSON object`)
  }
  return value as JsonObject
}

export function requireString(input: JsonObject, key: string): string {
  const value = input[key]
  if (typeof value !== 'string' || value.length === 0) throw new Error(`${key} must be a non-empty string`)
  return value
}

export function requireStringList(input: JsonObject, key: string): readonly string[] {
  const value = input[key]
  if (!Array.isArray(value) || !value.every((item): item is string => typeof item === 'string')) {
    throw new Error(`${key} must be a list of strings`)
  }
  return value
}
