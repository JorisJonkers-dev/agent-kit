import type { JsonRecord } from '../../../../shared-kernel/index.js'

export function parseJsonRecord(line: string): JsonRecord | null {
  try {
    const parsed: unknown = JSON.parse(line)
    return isJsonRecord(parsed) ? parsed : null
  } catch {
    return null
  }
}

export function isJsonRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
