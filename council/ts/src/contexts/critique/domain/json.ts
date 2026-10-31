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

/** A JSON value without the shape its reader expects. The message names the path of the wrong field. */
export class JsonShapeError extends Error {
  override readonly name = 'JsonShapeError'
}

function isText(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0
}

/**
 * Reads the fields of one JSON object. Every reader checks the field's type
 * and throws a JsonShapeError naming its path, so nothing read from a
 * command's input or a file is trusted by a cast.
 */
export class JsonFields {
  private constructor(
    private readonly value: JsonObject,
    private readonly path: string,
  ) {}

  /** Parses JSON text and reads it as an object. */
  static parse(text: string): JsonFields {
    return JsonFields.of(JSON.parse(text) as unknown, '')
  }

  static of(value: unknown, path: string): JsonFields {
    JsonFields.ensure(isJsonObject(value), path === '' ? 'the value' : path, 'a JSON object')
    return new JsonFields(value, path)
  }

  private static ensure(ok: boolean, path: string, expected: string): asserts ok {
    if (!ok) throw new JsonShapeError(`${path} must be ${expected}`)
  }

  has(key: string): boolean {
    return this.value[key] !== undefined
  }

  /** A string with at least one character that is not white space. */
  string(key: string): string {
    const value = this.value[key]
    JsonFields.ensure(isText(value), this.at(key), 'a non-empty string')
    return value
  }

  optionalString(key: string): string | undefined {
    return this.has(key) ? this.string(key) : undefined
  }

  /** Any string, the empty one included. */
  optionalText(key: string): string | undefined {
    const value = this.value[key]
    JsonFields.ensure(value === undefined || typeof value === 'string', this.at(key), 'a string')
    return value
  }

  number(key: string): number {
    const value = this.value[key]
    JsonFields.ensure(typeof value === 'number' && Number.isFinite(value), this.at(key), 'a number')
    return value
  }

  nullableNumber(key: string): number | null {
    return this.value[key] === null ? null : this.number(key)
  }

  boolean(key: string): boolean {
    const value = this.value[key]
    JsonFields.ensure(typeof value === 'boolean', this.at(key), 'true or false')
    return value
  }

  literal<T extends string | number>(key: string, expected: T): T {
    JsonFields.ensure(this.value[key] === expected, this.at(key), JSON.stringify(expected))
    return expected
  }

  oneOf<T extends string>(key: string, allowed: readonly T[]): T {
    const value = this.value[key]
    JsonFields.ensure(isOneOf(allowed, value), this.at(key), `one of ${allowed.join(', ')}`)
    return value
  }

  /** A string `accept` recognises, or nothing. */
  optionalMatching<T extends string>(key: string, accept: (value: string) => value is T, expected: string): T | undefined {
    const value = this.optionalString(key)
    JsonFields.ensure(value === undefined || accept(value), this.at(key), expected)
    return value
  }

  strings(key: string): readonly string[] {
    return this.listOf(key, (item): item is string => typeof item === 'string', 'a string')
  }

  oneOfList<T extends string>(key: string, allowed: readonly T[]): readonly T[] {
    return this.listOf(key, (item): item is T => isOneOf(allowed, item), `one of ${allowed.join(', ')}`)
  }

  list<T>(key: string, read: (item: JsonFields) => T): readonly T[] {
    return this.items(key).map((item, index) => read(JsonFields.of(item, `${this.at(key)}[${String(index)}]`)))
  }

  object(key: string): JsonFields {
    return JsonFields.of(this.value[key], this.at(key))
  }

  /** An object used as a map: every value is read with `read`, under its key. */
  entries<T>(key: string, read: (item: JsonFields, name: string) => T): Readonly<Record<string, T>> {
    const map = this.object(key)
    return Object.fromEntries(Object.keys(map.value).map((name) => [name, read(map.object(name), name)]))
  }

  /** An object whose every value is a non-empty string. */
  stringMap(key: string): Readonly<Record<string, string>> {
    const map = this.object(key)
    return Object.fromEntries(Object.keys(map.value).map((name) => [name, map.string(name)]))
  }

  private listOf<T>(key: string, accept: (item: unknown) => item is T, expected: string): readonly T[] {
    return this.items(key).map((item, index) => {
      JsonFields.ensure(accept(item), `${this.at(key)}[${String(index)}]`, expected)
      return item
    })
  }

  private items(key: string): readonly unknown[] {
    const value = this.value[key]
    JsonFields.ensure(Array.isArray(value), this.at(key), 'a list')
    return value
  }

  private at(key: string): string {
    return this.path === '' ? key : `${this.path}.${key}`
  }
}
