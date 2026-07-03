import type { JsonRecord, JsonValue } from '../../../../domain/contracts/index.js'
import type { EngineResultExtraction } from '../../../../domain/engines/index.js'
import type { EngineAdapterPorts } from './types.js'
import { isJsonRecord } from './json.js'

export type ResultExtraction =
  | {
      readonly text: string
      readonly costUsd?: number
      readonly metadata?: JsonRecord
    }
  | {
      readonly error: string
    }

export async function extractGenericResult(
  extraction: EngineResultExtraction,
  stdout: string,
  outputFile: string,
  jsonRecords: readonly JsonRecord[],
  ports: EngineAdapterPorts,
): Promise<ResultExtraction> {
  switch (extraction.mode) {
    case 'output_file':
      return { text: await ports.files.readText(outputFile) }
    case 'stdout':
      return { text: stdout }
    case 'json_path':
      return extractJsonPathResult(jsonRecords, extraction.path)
  }
}

export function resultText(value: JsonValue | undefined): string {
  if (typeof value === 'string') {
    return value
  }
  return value === undefined ? '' : JSON.stringify(value)
}

export function resultExtraction(
  text: string,
  costUsd: number | undefined,
  metadata: JsonRecord | undefined,
): ResultExtraction {
  const result: {
    text: string
    costUsd?: number
    metadata?: JsonRecord
  } = { text }

  if (costUsd !== undefined) {
    result.costUsd = costUsd
  }
  if (metadata !== undefined) {
    result.metadata = metadata
  }

  return result
}

export function readNumber(value: JsonValue | undefined): number | undefined {
  return typeof value === 'number' ? value : undefined
}

function extractJsonPathResult(
  jsonRecords: readonly JsonRecord[],
  path: readonly string[],
): ResultExtraction {
  const record = jsonRecords.at(-1)

  if (record === undefined) {
    return { error: 'json result line not found' }
  }

  const value = readJsonPath(record, path)

  if (value === undefined) {
    return { error: `json result path not found: ${path.join('.')}` }
  }

  return { text: resultText(value) }
}

function readJsonPath(record: JsonRecord, path: readonly string[]): JsonValue | undefined {
  let current: JsonValue | undefined = record

  for (const segment of path) {
    current = isJsonRecord(current) ? current[segment] : undefined
  }

  return current
}
