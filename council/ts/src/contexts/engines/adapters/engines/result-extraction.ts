import type { JsonRecord, JsonValue } from '../../../../shared-kernel/index.js'
import type { EngineResultExtraction } from '../../../engines/index.js'
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

interface GenericResultExtractionContext {
  readonly stdout: string
  readonly outputFile: string
  readonly jsonRecords: readonly JsonRecord[]
  readonly ports: EngineAdapterPorts
}

interface GenericResultExtractionStrategy {
  extract(context: GenericResultExtractionContext): Promise<ResultExtraction> | ResultExtraction
}

type ResultExtractionFactory = (
  extraction: EngineResultExtraction,
) => GenericResultExtractionStrategy

const RESULT_EXTRACTION_FACTORIES: Readonly<
  Record<EngineResultExtraction['mode'], ResultExtractionFactory>
> = Object.freeze({
  json_path: (extraction) =>
    new JsonPathResultExtractionStrategy(
      (extraction as Extract<EngineResultExtraction, { readonly mode: 'json_path' }>).path,
    ),
  output_file: () => OUTPUT_FILE_RESULT_EXTRACTION,
  stdout: () => STDOUT_RESULT_EXTRACTION,
})

export async function extractGenericResult(
  extraction: EngineResultExtraction,
  stdout: string,
  outputFile: string,
  jsonRecords: readonly JsonRecord[],
  ports: EngineAdapterPorts,
): Promise<ResultExtraction> {
  return RESULT_EXTRACTION_FACTORIES[extraction.mode](extraction).extract({
    stdout,
    outputFile,
    jsonRecords,
    ports,
  })
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

class OutputFileResultExtractionStrategy implements GenericResultExtractionStrategy {
  async extract(context: GenericResultExtractionContext): Promise<ResultExtraction> {
    return { text: await context.ports.files.readText(context.outputFile) }
  }
}

class StdoutResultExtractionStrategy implements GenericResultExtractionStrategy {
  extract(context: GenericResultExtractionContext): ResultExtraction {
    return { text: context.stdout }
  }
}

class JsonPathResultExtractionStrategy implements GenericResultExtractionStrategy {
  constructor(private readonly path: readonly string[]) {}

  extract(context: GenericResultExtractionContext): ResultExtraction {
    return extractJsonPathResult(context.jsonRecords, this.path)
  }
}

const OUTPUT_FILE_RESULT_EXTRACTION = new OutputFileResultExtractionStrategy()
const STDOUT_RESULT_EXTRACTION = new StdoutResultExtractionStrategy()

function readJsonPath(record: JsonRecord, path: readonly string[]): JsonValue | undefined {
  let current: JsonValue | undefined = record

  for (const segment of path) {
    current = isJsonRecord(current) ? current[segment] : undefined
  }

  return current
}
