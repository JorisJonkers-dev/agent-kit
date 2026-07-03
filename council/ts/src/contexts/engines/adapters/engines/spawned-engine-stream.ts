import type { JsonRecord } from '../../../../domain/contracts/index.js'
import type {
  EngineChild,
  EngineEvent,
  EngineEventStream,
  EngineInjectionCapability,
  EngineInjectionReceipt,
  EngineSpawnCommand,
} from './types.js'
import type { ResultExtraction } from './result-extraction.js'

export interface SpawnedEngineStreamConfig {
  readonly child: EngineChild
  readonly command: EngineSpawnCommand
  readonly engine: string
  readonly injection: EngineInjectionCapability
  initialInput(): Promise<void>
  injectInput(prompt: string): Promise<EngineInjectionReceipt>
  parseStdoutLine(line: string): ParsedLine
  readResult(stdout: string): Promise<ResultExtraction> | ResultExtraction
}

export interface ParsedLine {
  readonly event?: EngineEvent
}

export function createSpawnedEngineStream(
  config: SpawnedEngineStreamConfig,
): EngineEventStream {
  let inputStarted = false

  const ensureInput = async (): Promise<void> => {
    if (!inputStarted) {
      inputStarted = true
      await config.initialInput()
    }
  }

  return {
    command: config.command,
    injection: config.injection,
    async inject(prompt: string): Promise<EngineInjectionReceipt> {
      await ensureInput()
      return config.injectInput(prompt)
    },
    async closeInput(): Promise<void> {
      await config.child.closeStdin()
    },
    async *[Symbol.asyncIterator](): AsyncIterator<EngineEvent> {
      await ensureInput()
      yield {
        type: 'started',
        engine: config.engine,
        command: config.command,
        injection: config.injection,
      }

      const stdoutLines: string[] = []
      const stderrLines: string[] = []

      for await (const line of readLines(config.child.stdout)) {
        stdoutLines.push(line)
        const parsed = config.parseStdoutLine(line)

        if (parsed.event !== undefined) {
          yield parsed.event
        }
      }

      for await (const line of readLines(config.child.stderr)) {
        stderrLines.push(line)
        yield progressEvent('stderr', line)
      }

      const exit = await config.child.exit
      const stdout = stdoutLines.join('\n')
      const stderr = stderrLines.join('\n')

      if (exit.exitCode !== 0) {
        yield {
          type: 'failed',
          exitCode: exit.exitCode,
          error: stderr.length === 0 ? `engine exited with code ${String(exit.exitCode)}` : stderr,
          stdout,
          stderr,
        }
        return
      }

      const result = await config.readResult(stdout)

      if ('error' in result) {
        yield {
          type: 'failed',
          exitCode: exit.exitCode,
          error: result.error,
          stdout,
          stderr,
        }
        return
      }

      yield resultEvent(result, exit.exitCode)
    },
  }
}

export function progressEvent(
  stream: 'stdout' | 'stderr',
  text: string,
  raw?: JsonRecord,
): Extract<EngineEvent, { type: 'progress' }> {
  return raw === undefined
    ? { type: 'progress', stream, text }
    : { type: 'progress', stream, text, raw }
}

function resultEvent(
  extraction: Extract<ResultExtraction, { text: string }>,
  exitCode: number,
): Extract<EngineEvent, { type: 'result' }> {
  const result: {
    type: 'result'
    text: string
    exitCode: number
    costUsd?: number
    metadata?: JsonRecord
  } = {
    type: 'result',
    text: extraction.text,
    exitCode,
  }

  if (extraction.costUsd !== undefined) {
    result.costUsd = extraction.costUsd
  }
  if (extraction.metadata !== undefined) {
    result.metadata = extraction.metadata
  }

  return result
}

async function* readLines(chunks: AsyncIterable<string>): AsyncGenerator<string> {
  let pending = ''

  for await (const chunk of chunks) {
    pending += chunk
    let newline = pending.indexOf('\n')

    while (newline >= 0) {
      yield stripCarriageReturn(pending.slice(0, newline))
      pending = pending.slice(newline + 1)
      newline = pending.indexOf('\n')
    }
  }

  if (pending.length > 0) {
    yield stripCarriageReturn(pending)
  }
}

function stripCarriageReturn(line: string): string {
  return line.endsWith('\r') ? line.slice(0, -1) : line
}
