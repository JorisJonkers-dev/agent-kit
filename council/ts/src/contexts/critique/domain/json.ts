export type JsonObject = Readonly<Record<string, unknown>>

export function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** The value when it is a JSON object, otherwise undefined. */
export function jsonObjectOrUndefined(value: unknown): JsonObject | undefined {
  return isJsonObject(value) ? value : undefined
}

export function isOneOf<T extends string>(allowed: readonly T[], value: unknown): value is T {
  return typeof value === 'string' && (allowed as readonly string[]).includes(value)
}
