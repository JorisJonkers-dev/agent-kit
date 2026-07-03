import type { EngineDef, EngineResultExtraction } from '../../domain/engines/index.js'
import type { JsonRecord, JsonValue } from '../../domain/contracts/index.js'

export interface EngineRunRequest {
  readonly engine: EngineDef
  readonly model: string
  readonly effort: string
  readonly prompt: string
  readonly promptFile: string
  readonly outputFile: string
  readonly cwd?: string
}

export interface EngineSpawnCommand {
  readonly command: string
  readonly args: readonly string[]
  readonly cwd?: string
}

export interface EngineProcessExit {
  readonly exitCode: number
}

export interface EngineChild {
  readonly stdout: AsyncIterable<string>
  readonly stderr: AsyncIterable<string>
  readonly exit: Promise<EngineProcessExit>
  writeStdin(chunk: string): Promise<void>
  closeStdin(): Promise<void>
}

export interface EngineProcess {
  spawn(command: EngineSpawnCommand): EngineChild
}

export interface EngineFileStore {
  writeText(path: string, data: string): Promise<void>
  readText(path: string): Promise<string>
}

export type EngineInjectionMode = 'live_stdin' | 'checkpoint_resume'

export interface EngineInjectionCapability {
  readonly mode: EngineInjectionMode
  readonly evidence: string
}

export interface EngineInjectionReceipt {
  readonly accepted: boolean
  readonly mode: EngineInjectionMode
}

export type EngineEvent =
  | {
      readonly type: 'started'
      readonly engine: string
      readonly command: EngineSpawnCommand
      readonly injection: EngineInjectionCapability
    }
  | {
      readonly type: 'progress'
      readonly stream: 'stdout' | 'stderr'
      readonly text: string
      readonly raw?: JsonRecord
    }
  | {
      readonly type: 'result'
      readonly text: string
      readonly exitCode: number
      readonly costUsd?: number
      readonly metadata?: JsonRecord
    }
  | {
      readonly type: 'failed'
      readonly exitCode: number
      readonly error: string
      readonly stdout: string
      readonly stderr: string
    }

export interface EngineEventStream extends AsyncIterable<EngineEvent> {
  readonly command: EngineSpawnCommand
  readonly injection: EngineInjectionCapability
  inject(prompt: string): Promise<EngineInjectionReceipt>
  closeInput(): Promise<void>
}

export interface EngineAdapterPorts {
  readonly process: EngineProcess
  readonly files: EngineFileStore
}

export const CLAUDE_STDIN_INJECTION: EngineInjectionCapability = {
  mode: 'live_stdin',
  evidence:
    'local claude -p --help documents --input-format stream-json as realtime streaming input',
}

export const CHECKPOINT_RESUME_INJECTION: EngineInjectionCapability = {
  mode: 'checkpoint_resume',
  evidence: 'v1 non-Claude drivers accept new guidance by restarting from checkpointed state',
}

export function runEngine(request: EngineRunRequest, ports: EngineAdapterPorts): EngineEventStream {
  if (request.engine.name === 'claude') {
    return runClaudeEngine(request, ports)
  }
  if (request.engine.name === 'codex') {
    return runCodexEngine(request, ports)
  }
  return runGenericCommandEngine(request, ports)
}

export function runClaudeEngine(
  request: EngineRunRequest,
  ports: EngineAdapterPorts,
): EngineEventStream {
  const command: EngineSpawnCommand = {
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
      await child.writeStdin(`${JSON.stringify(toClaudeInputMessage(request.prompt))}\n`)
    },
    injectInput: async (prompt) => {
      await child.writeStdin(`${JSON.stringify(toClaudeInputMessage(prompt))}\n`)
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

export function runCodexEngine(
  request: EngineRunRequest,
  ports: EngineAdapterPorts,
): EngineEventStream {
  const command: EngineSpawnCommand = {
    ...commandParts('codex', [
      'exec',
      '-m',
      request.model,
      '-c',
      `model_reasoning_effort=${request.effort}`,
      '--skip-git-repo-check',
      '-o',
      request.outputFile,
      '-',
    ]),
    ...optionalCwd(request.cwd),
  }
  const child = ports.process.spawn(command)

  return createSpawnedEngineStream({
    child,
    command,
    engine: request.engine.name,
    injection: CHECKPOINT_RESUME_INJECTION,
    initialInput: async () => {
      await child.writeStdin(request.prompt)
      await child.closeStdin()
    },
    injectInput: checkpointResumeInjection,
    parseStdoutLine: (line) => ({ event: progressEvent('stdout', line) }),
    readResult: async () => ({ text: await ports.files.readText(request.outputFile) }),
  })
}

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
      if (request.engine.promptDelivery === 'prompt_file') {
        await ports.files.writeText(request.promptFile, request.prompt)
      } else {
        await child.writeStdin(request.prompt)
        await child.closeStdin()
      }
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

interface SpawnedEngineStreamConfig {
  readonly child: EngineChild
  readonly command: EngineSpawnCommand
  readonly engine: string
  readonly injection: EngineInjectionCapability
  initialInput(): Promise<void>
  injectInput(prompt: string): Promise<EngineInjectionReceipt>
  parseStdoutLine(line: string): ParsedLine
  readResult(stdout: string): Promise<ResultExtraction> | ResultExtraction
}

interface ParsedLine {
  readonly event?: EngineEvent
}

type ResultExtraction =
  | {
      readonly text: string
      readonly costUsd?: number
      readonly metadata?: JsonRecord
    }
  | {
      readonly error: string
    }

function createSpawnedEngineStream(config: SpawnedEngineStreamConfig): EngineEventStream {
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

function checkpointResumeInjection(): Promise<EngineInjectionReceipt> {
  return Promise.resolve({ accepted: false, mode: 'checkpoint_resume' })
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

function progressEvent(
  stream: 'stdout' | 'stderr',
  text: string,
  raw?: JsonRecord,
): Extract<EngineEvent, { type: 'progress' }> {
  return raw === undefined
    ? { type: 'progress', stream, text }
    : { type: 'progress', stream, text, raw }
}

function expandCommand(engine: EngineDef, request: EngineRunRequest): EngineSpawnCommand {
  const [command, ...args] = engine.argv.map((arg) =>
    arg
      .replaceAll('{model}', request.model)
      .replaceAll('{effort}', request.effort)
      .replaceAll('{output}', request.outputFile)
      .replaceAll('{prompt_file}', request.promptFile),
  )

  return {
    ...commandParts(command ?? engine.name, args),
    ...optionalCwd(request.cwd),
  }
}

function commandParts(
  command: string,
  args: readonly string[],
): Pick<EngineSpawnCommand, 'args' | 'command'> {
  return { command, args }
}

function optionalCwd(
  cwd: string | undefined,
): Pick<EngineSpawnCommand, 'cwd'> | Record<string, never> {
  return cwd === undefined ? {} : { cwd }
}

function parseJsonRecord(line: string): JsonRecord | null {
  try {
    const parsed: unknown = JSON.parse(line)
    return isJsonRecord(parsed) ? parsed : null
  } catch {
    return null
  }
}

async function extractGenericResult(
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

function resultText(value: JsonValue | undefined): string {
  if (typeof value === 'string') {
    return value
  }
  return value === undefined ? '' : JSON.stringify(value)
}

function resultExtraction(
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

function readNumber(value: JsonValue | undefined): number | undefined {
  return typeof value === 'number' ? value : undefined
}

function toClaudeInputMessage(prompt: string): JsonRecord {
  return {
    type: 'user',
    message: {
      role: 'user',
      content: [{ type: 'text', text: prompt }],
    },
  }
}

function isJsonRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
