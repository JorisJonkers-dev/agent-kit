import type { JsonRecord } from '../../../../domain/contracts/index.js'
import type { EngineAdapterPorts, EngineEventStream, EngineRunRequest } from './types.js'
import { expandCommand } from './commands.js'
import { CHECKPOINT_RESUME_INJECTION, checkpointResumeInjection } from './injection.js'
import { parseJsonRecord } from './json.js'
import { deliverGenericPrompt } from './prompt-delivery.js'
import { extractGenericResult } from './result-extraction.js'
import { createSpawnedEngineStream, progressEvent } from './spawned-engine-stream.js'

export function runGenericCommandEngine(
  request: EngineRunRequest,
  ports: EngineAdapterPorts,
): EngineEventStream {
  const command = expandCommand(request.engine, request)
  const child = ports.process.spawn(command)
  const jsonRecords: JsonRecord[] = []

  return createSpawnedEngineStream({
    child,
    command,
    engine: request.engine.name,
    injection: CHECKPOINT_RESUME_INJECTION,
    initialInput: async () => {
      await deliverGenericPrompt(request, ports, child)
    },
    injectInput: checkpointResumeInjection,
    parseStdoutLine: (line) => {
      const raw = request.engine.streamFormat === 'json' ? parseJsonRecord(line) : null

      if (raw !== null) {
        jsonRecords.push(raw)
        return { event: progressEvent('stdout', line, raw) }
      }

      return { event: progressEvent('stdout', line) }
    },
    readResult: async (stdout) =>
      extractGenericResult(
        request.engine.resultExtraction,
        stdout,
        request.outputFile,
        jsonRecords,
        ports,
      ),
  })
}
