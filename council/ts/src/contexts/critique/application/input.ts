import { isJsonObject, isOneOf, type JsonObject } from '../domain/json.js'

/** Small readers for the JSON each command receives. A bad field throws with its name. */

export function asObject(value: unknown, what: string): JsonObject {
  if (!isJsonObject(value)) throw new Error(`${what} must be a JSON object`)
  return value
}

export function requireString(input: JsonObject, key: string): string {
  const value = input[key]
  if (typeof value !== 'string' || value.length === 0) throw new Error(`${key} must be a non-empty string`)
  return value
}

export function requireOneOf<T extends string>(input: JsonObject, key: string, allowed: readonly T[]): T {
  const value = requireString(input, key)
  if (!isOneOf(allowed, value)) throw new Error(`${key} must be one of ${allowed.join(', ')}`)
  return value
}

export function requireStringList(input: JsonObject, key: string): readonly string[] {
  const value = input[key]
  if (!Array.isArray(value) || !value.every((item): item is string => typeof item === 'string')) {
    throw new Error(`${key} must be a list of strings`)
  }
  return value
}
