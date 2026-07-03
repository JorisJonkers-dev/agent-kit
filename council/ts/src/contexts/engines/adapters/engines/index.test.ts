import { describe, expect, it } from 'vitest'

import type { EngineDef } from '../../../../domain/engines/index.js'
import {
  CHECKPOINT_RESUME_INJECTION,
  CLAUDE_STDIN_INJECTION,
  type EngineAdapterPorts,
  type EngineChild,
  type EngineEvent,
  type EngineEventStream,
  type EngineFileStore,
  type EngineProcess,
  type EngineProcessExit,
  type EngineRunRequest,
  type EngineSpawnCommand,
  runEngine,
} from './index.js'

class RecordingChild implements EngineChild {
  readonly stdout: AsyncIterable<string>
  readonly stderr: AsyncIterable<string>
  readonly exit: Promise<EngineProcessExit>
  readonly writes: string[] = []
  closeCount = 0

  constructor(input: {
    readonly stdout?: readonly string[]
    readonly stderr?: readonly string[]
    readonly exitCode?: number
  }) {
    this.stdout = chunks(input.stdout ?? [])
    this.stderr = chunks(input.stderr ?? [])
    this.exit = Promise.resolve({ exitCode: input.exitCode ?? 0 })
  }

  writeStdin(chunk: string): Promise<void> {
    this.writes.push(chunk)
    return Promise.resolve()
  }

  closeStdin(): Promise<void> {
    this.closeCount += 1
    return Promise.resolve()
  }
}

class RecordingProcess implements EngineProcess {
  readonly commands: EngineSpawnCommand[] = []
  private readonly child: EngineChild

  constructor(child: EngineChild) {
    this.child = child
  }

  spawn(command: EngineSpawnCommand): EngineChild {
    this.commands.push(command)
    return this.child
  }
}

class MemoryFiles implements EngineFileStore {
  readonly writes = new Map<string, string>()
  private readonly reads: ReadonlyMap<string, string>

  constructor(reads: ReadonlyMap<string, string> = new Map()) {
    this.reads = reads
  }

  writeText(path: string, data: string): Promise<void> {
    this.writes.set(path, data)
    return Promise.resolve()
  }

  readText(path: string): Promise<string> {
    return Promise.resolve(this.reads.get(path) ?? '')
  }
}

async function* chunks(values: readonly string[]): AsyncGenerator<string> {
  await Promise.resolve()
  for (const value of values) {
    yield value
  }
}

function parseJsonRecord(value: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(value)
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error('expected JSON object')
  }
  return parsed as Record<string, unknown>
}

async function collect(stream: EngineEventStream): Promise<readonly EngineEvent[]> {
  const events: EngineEvent[] = []

  for await (const event of stream) {
    events.push(event)
  }

  return events
}

function request(engine: EngineDef): EngineRunRequest {
  return {
    engine,
    model: 'model-a',
    effort: 'high',
    prompt: 'do the work',
    promptFile: '/tmp/prompt.md',
    outputFile: '/tmp/output.txt',
    cwd: '/repo',
  }
}

function engineDef(overrides: Partial<EngineDef> & Pick<EngineDef, 'name'>): EngineDef {
  return {
    argv: ['runner', '{model}', '{effort}', '{output}'],
    promptDelivery: 'stdin',
    resultExtraction: { mode: 'output_file' },
    streamFormat: 'text',
    ...overrides,
  }
}

describe('engine adapters', () => {
  it('runs Claude with stream-json IO, accepts live stdin injection, and returns the last result event', async () => {
    const assistant = JSON.stringify({
      type: 'assistant',
      message: { content: [{ type: 'text', text: 'working' }] },
    })
    const firstResult = JSON.stringify({ type: 'result', result: 'draft', total_cost_usd: 1 })
    const finalResult = JSON.stringify({ type: 'result', result: 'final', cost_usd: 2 })
    const child = new RecordingChild({
      stdout: [
        `${assistant}\n${firstResult}\n${finalResult.slice(0, 18)}`,
        `${finalResult.slice(18)}\n`,
      ],
      stderr: ['warn\r\n'],
    })
    const process = new RecordingProcess(child)
    const files = new MemoryFiles()
    const stream = runEngine(request(engineDef({ name: 'claude' })), { files, process })

    await expect(stream.inject('more guidance')).resolves.toEqual({
      accepted: true,
      mode: 'live_stdin',
    })
    await stream.closeInput()

    await expect(collect(stream)).resolves.toEqual([
      {
        type: 'started',
        engine: 'claude',
        command: {
          command: 'claude',
          args: [
            '-p',
            '--model',
            'model-a',
            '--effort',
            'high',
            '--output-format',
            'stream-json',
            '--verbose',
            '--input-format',
            'stream-json',
          ],
          cwd: '/repo',
        },
        injection: CLAUDE_STDIN_INJECTION,
      },
      { type: 'progress', stream: 'stdout', text: assistant, raw: parseJsonRecord(assistant) },
      { type: 'progress', stream: 'stderr', text: 'warn' },
      {
        type: 'result',
        text: 'final',
        exitCode: 0,
        costUsd: 2,
        metadata: parseJsonRecord(finalResult),
      },
    ])
    expect(process.commands).toEqual([stream.command])
    expect(child.closeCount).toBe(1)
    expect(child.writes.map((write) => parseJsonRecord(write))).toEqual([
      {
        type: 'user',
        message: { role: 'user', content: [{ type: 'text', text: 'do the work' }] },
      },
      {
        type: 'user',
        message: { role: 'user', content: [{ type: 'text', text: 'more guidance' }] },
      },
    ])
  })

  it('runs Codex with an output file and emits stdout progress before the file result', async () => {
    const child = new RecordingChild({ stdout: ['step 1\n$ npm test\n'] })
    const process = new RecordingProcess(child)
    const files = new MemoryFiles(new Map([['/tmp/output.txt', 'final answer\n']]))
    const stream = runEngine(request(engineDef({ name: 'codex' })), { files, process })

    await expect(stream.inject('late guidance')).resolves.toEqual({
      accepted: false,
      mode: 'checkpoint_resume',
    })
    await expect(collect(stream)).resolves.toEqual([
      {
        type: 'started',
        engine: 'codex',
        command: {
          command: 'codex',
          args: [
            'exec',
            '-m',
            'model-a',
            '-c',
            'model_reasoning_effort=high',
            '--skip-git-repo-check',
            '-o',
            '/tmp/output.txt',
            '-',
          ],
          cwd: '/repo',
        },
        injection: CHECKPOINT_RESUME_INJECTION,
      },
      { type: 'progress', stream: 'stdout', text: 'step 1' },
      { type: 'progress', stream: 'stdout', text: '$ npm test' },
      { type: 'result', text: 'final answer\n', exitCode: 0 },
    ])
    expect(child.writes).toEqual(['do the work'])
    expect(child.closeCount).toBe(1)
  })

  it('runs generic prompt-file commands and extracts output-file results', async () => {
    const child = new RecordingChild({ stdout: ['plain progress'] })
    const process = new RecordingProcess(child)
    const files = new MemoryFiles(new Map([['/tmp/output.txt', 'generic result']]))
    const stream = runEngine(
      request(
        engineDef({
          name: 'local_runner',
          argv: [
            'runner',
            '--model',
            '{model}',
            '--effort',
            '{effort}',
            '--prompt',
            '{prompt_file}',
            '--out',
            '{output}',
          ],
          promptDelivery: 'prompt_file',
        }),
      ),
      { files, process },
    )

    await expect(collect(stream)).resolves.toEqual([
      {
        type: 'started',
        engine: 'local_runner',
        command: {
          command: 'runner',
          args: [
            '--model',
            'model-a',
            '--effort',
            'high',
            '--prompt',
            '/tmp/prompt.md',
            '--out',
            '/tmp/output.txt',
          ],
          cwd: '/repo',
        },
        injection: CHECKPOINT_RESUME_INJECTION,
      },
      { type: 'progress', stream: 'stdout', text: 'plain progress' },
      { type: 'result', text: 'generic result', exitCode: 0 },
    ])
    expect(files.writes).toEqual(new Map([['/tmp/prompt.md', 'do the work']]))
    expect(child.writes).toEqual([])
  })

  it('runs generic stdin commands with stdout and json-path result extraction', async () => {
    const stdoutChild = new RecordingChild({ stdout: ['line one\nline two'] })
    const jsonChild = new RecordingChild({
      stdout: ['[]\nnot json\n', '{"message":{"content":{"ok":true}}}\n'],
    })
    const stdoutPorts: EngineAdapterPorts = {
      files: new MemoryFiles(),
      process: new RecordingProcess(stdoutChild),
    }
    const jsonPorts: EngineAdapterPorts = {
      files: new MemoryFiles(),
      process: new RecordingProcess(jsonChild),
    }

    await expect(
      collect(
        runEngine(
          request(engineDef({ name: 'stdout_runner', resultExtraction: { mode: 'stdout' } })),
          stdoutPorts,
        ),
      ),
    ).resolves.toContainEqual({ type: 'result', text: 'line one\nline two', exitCode: 0 })
    await expect(
      collect(
        runEngine(
          request(
            engineDef({
              name: 'json_runner',
              resultExtraction: { mode: 'json_path', path: ['message', 'content'] },
              streamFormat: 'json',
            }),
          ),
          jsonPorts,
        ),
      ),
    ).resolves.toContainEqual({ type: 'result', text: '{"ok":true}', exitCode: 0 })
    expect(stdoutChild.writes).toEqual(['do the work'])
    expect(jsonChild.closeCount).toBe(1)
  })

  it('normalizes missing results and non-zero exits into failed events', async () => {
    const missingClaude = runEngine(request(engineDef({ name: 'claude' })), {
      files: new MemoryFiles(),
      process: new RecordingProcess(
        new RecordingChild({ stdout: ['not json\n{"type":"assistant"}\n'] }),
      ),
    })
    const noJsonRecords = runEngine(
      request(
        engineDef({
          name: 'json_runner',
          resultExtraction: { mode: 'json_path', path: ['message', 'content'] },
          streamFormat: 'json',
        }),
      ),
      {
        files: new MemoryFiles(),
        process: new RecordingProcess(new RecordingChild({ stdout: ['not json\n'] })),
      },
    )
    const missingJson = runEngine(
      request(
        engineDef({
          name: 'json_runner',
          resultExtraction: { mode: 'json_path', path: ['message', 'content'] },
          streamFormat: 'json',
        }),
      ),
      {
        files: new MemoryFiles(),
        process: new RecordingProcess(new RecordingChild({ stdout: ['{"message":{}}\n'] })),
      },
    )
    const failedProcess = runEngine(request(engineDef({ name: 'codex' })), {
      files: new MemoryFiles(),
      process: new RecordingProcess(
        new RecordingChild({ stdout: ['partial\n'], stderr: ['fatal\n'], exitCode: 7 }),
      ),
    })

    await expect(collect(missingClaude)).resolves.toContainEqual({
      type: 'failed',
      exitCode: 0,
      error: 'claude stream-json result event not found',
      stdout: 'not json\n{"type":"assistant"}',
      stderr: '',
    })
    await expect(collect(noJsonRecords)).resolves.toContainEqual({
      type: 'failed',
      exitCode: 0,
      error: 'json result line not found',
      stdout: 'not json',
      stderr: '',
    })
    await expect(collect(missingJson)).resolves.toContainEqual({
      type: 'failed',
      exitCode: 0,
      error: 'json result path not found: message.content',
      stdout: '{"message":{}}',
      stderr: '',
    })
    await expect(collect(failedProcess)).resolves.toContainEqual({
      type: 'failed',
      exitCode: 7,
      error: 'fatal',
      stdout: 'partial',
      stderr: 'fatal',
    })
  })
})
