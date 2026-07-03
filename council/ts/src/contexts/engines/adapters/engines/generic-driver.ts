import type { JsonRecord } from '../../../../shared-kernel/index.js'
import type { EngineStreamFormat } from '../../../engines/index.js'
import type { EngineAdapterPorts, EngineDriver, EngineEventStream, EngineRunRequest } from './types.js'
import { expandCommand } from './commands.js'
import { CHECKPOINT_RESUME_INJECTION, checkpointResumeInjection } from './injection.js'
import { parseJsonRecord } from './json.js'
import { deliverGenericPrompt } from './prompt-delivery.js'
import { extractGenericResult } from './result-extraction.js'
import {
  type ParsedLine,
  createSpawnedEngineStream,
  progressEvent,
} from './spawned-engine-stream.js'

type GenericStdoutParserFactory = () => GenericStdoutParser

const STDOUT_PARSER_FACTORIES: Readonly<Record<EngineStreamFormat, GenericStdoutParserFactory>> =
  Object.freeze({
    json: () => new JsonGenericStdoutParser(),
    text: () => new TextGenericStdoutParser(),
  })

export class GenericCommandEngineDriver implements EngineDriver {
  run(request: EngineRunRequest, ports: EngineAdapterPorts): EngineEventStream {
    const command = expandCommand(request.engine, request)
    const child = ports.process.spawn(command)
    const stdoutParser = STDOUT_PARSER_FACTORIES[request.engine.streamFormat]()

    return createSpawnedEngineStream({
      child,
      command,
      engine: request.engine.name,
      injection: CHECKPOINT_RESUME_INJECTION,
      initialInput: async () => {
        await deliverGenericPrompt(request, ports, child)
      },
      injectInput: checkpointResumeInjection,
      parseStdoutLine: (line) => stdoutParser.parse(line),
      readResult: async (stdout) =>
        extractGenericResult(
          request.engine.resultExtraction,
          stdout,
          request.outputFile,
          stdoutParser.jsonRecords,
          ports,
        ),
    })
  }
}

export const GENERIC_COMMAND_ENGINE_DRIVER: EngineDriver = new GenericCommandEngineDriver()

export function runGenericCommandEngine(
  request: EngineRunRequest,
  ports: EngineAdapterPorts,
): EngineEventStream {
  return GENERIC_COMMAND_ENGINE_DRIVER.run(request, ports)
}

interface GenericStdoutParser {
  readonly jsonRecords: readonly JsonRecord[]
  parse(line: string): ParsedLine
}

class TextGenericStdoutParser implements GenericStdoutParser {
  readonly jsonRecords: readonly JsonRecord[] = []

  parse(line: string): ParsedLine {
    return { event: progressEvent('stdout', line) }
  }
}

class JsonGenericStdoutParser implements GenericStdoutParser {
  private readonly records: JsonRecord[] = []

  get jsonRecords(): readonly JsonRecord[] {
    return this.records
  }

  parse(line: string): ParsedLine {
    const raw = parseJsonRecord(line)

    if (raw !== null) {
      this.records.push(raw)
      return { event: progressEvent('stdout', line, raw) }
    }

    return { event: progressEvent('stdout', line) }
  }
}
