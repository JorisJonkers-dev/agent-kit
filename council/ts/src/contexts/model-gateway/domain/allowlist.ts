import { isObject, ModelGatewayError } from './config.js'

/**
 * One repository's entry in this machine's family allowlist: the model
 * families its content may go to, and whether that content may be code and
 * its history. An entry recorded before a permission existed reads as false.
 */
export interface AllowlistEntry {
  readonly families: readonly string[]
  readonly code: boolean
  readonly history: boolean
}

/** The family allowlist critique writes and the gateway reads, keyed by `<host>/<owner>/<repo>`. */
export interface FamilyAllowlist {
  readonly repositories: Readonly<Record<string, AllowlistEntry>>
}

export const EMPTY_ALLOWLIST: FamilyAllowlist = { repositories: {} }

/** Reads parsed JSON as the family allowlist, or says what is wrong with it. */
export function parseAllowlist(raw: unknown): FamilyAllowlist {
  if (!isObject(raw)) throw invalid('the allowlist must be a JSON object')
  if (raw.version !== 1) throw invalid('version must be 1')
  if (!isObject(raw.repositories)) throw invalid('repositories must be an object keyed by repository')
  const repositories = Object.fromEntries(
    Object.entries(raw.repositories).map(([key, value]) => {
      if (!isObject(value)) throw invalid(`repositories.${key} must be an object`)
      return [key, parseEntry(key, value)]
    }),
  )
  return { repositories }
}

function parseEntry(key: string, raw: Readonly<Record<string, unknown>>): AllowlistEntry {
  const where = `repositories.${key}`
  return {
    code: raw.code === undefined ? false : requireBoolean(raw.code, `${where}.code`),
    families: requireStrings(raw.families, `${where}.families`),
    history: raw.history === undefined ? false : requireBoolean(raw.history, `${where}.history`),
  }
}

function requireStrings(value: unknown, where: string): readonly string[] {
  if (!Array.isArray(value)) throw invalid(`${where} must be a list of non-empty strings`)
  const items: readonly unknown[] = value
  const strings: string[] = []
  for (const item of items) {
    if (typeof item !== 'string' || item === '') throw invalid(`${where} must be a list of non-empty strings`)
    strings.push(item)
  }
  return strings
}

function requireBoolean(value: unknown, where: string): boolean {
  if (typeof value !== 'boolean') throw invalid(`${where} must be true or false`)
  return value
}

function invalid(message: string): ModelGatewayError {
  return new ModelGatewayError('invalid-config', message)
}
