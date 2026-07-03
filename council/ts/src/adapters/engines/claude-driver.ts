import type { JsonRecord } from '../../domain/contracts/index.js'
import type { EngineAdapterPorts, EngineEventStream, EngineRunRequest } from './types.js'
import { commandParts, optionalCwd } from './commands.js'
import { CLAUDE_STDIN_INJECTION } from './injection.js'
import { parseJsonRecord } from './json.js'
import { writeClaudePrompt } from './prompt-delivery.js'
import { readNumber, resultExtraction, resultText } from './result-extraction.js'
import { createSpawnedEngineStream, progressEvent } from './spawned-engine-stream.js'

export function runClaudeEngine(
  request: EngineRunRequest,
  ports: EngineAdapterPorts,
): EngineEventStream {
  const command = {
    ...commandParts('claude', [
      '-p',
      '--model',
      request.model,
      '--effort',
      request.effort,
      '--output-format',
      'stream-json',
      '--verbose',
      '--input-format',
      'stream-json',
    ]),
    ...optionalCwd(request.cwd),
  }
  const child = ports.process.spawn(command)
  let lastResult: JsonRecord | undefined

  return createSpawnedEngineStream({
    child,
    command,
    engine: request.engine.name,
    injection: CLAUDE_STDIN_INJECTION,
    initialInput: async () => {
      await writeClaudePrompt(child, request.prompt)
    },
    injectInput: async (prompt) => {
      await writeClaudePrompt(child, prompt)
      return { accepted: true, mode: 'live_stdin' }
    },
    parseStdoutLine: (line) => {
      const raw = parseJsonRecord(line)

      if (raw === null) {
        return { event: progressEvent('stdout', line) }
      }

      if (raw.type === 'result') {
        lastResult = raw
        return {}
      }

      return { event: progressEvent('stdout', line, raw) }
    },
    readResult: () => {
      if (lastResult === undefined) {
        return { error: 'claude stream-json result event not found' }
      }

      return resultExtraction(
        resultText(lastResult.result),
        readNumber(lastResult.total_cost_usd) ?? readNumber(lastResult.cost_usd),
        lastResult,
      )
    },
  })
}
