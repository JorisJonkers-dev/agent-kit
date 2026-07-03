export type JsonPrimitive = string | number | boolean | null

export type JsonValue = JsonPrimitive | JsonRecord | readonly JsonValue[]

export interface JsonRecord {
  readonly [key: string]: JsonValue
}

export interface ContentAddressed {
  readonly content_hash?: string
}

export interface ContextLinked {
  readonly context_refs?: readonly string[]
}
