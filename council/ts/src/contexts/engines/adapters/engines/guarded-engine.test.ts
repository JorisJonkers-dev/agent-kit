import { describe, expect, it } from 'vitest'

import { ModelGatewayError } from '../../../model-gateway/index.js'
import { FakeEngineGuard } from '../../../../../test/council/fake-engine-guard.js'
import type { EngineDef } from '../../../engines/index.js'
import { EngineGate } from './engine-gate.js'
import { runGuardedEngine } from './guarded-engine.js'
import type {
  EngineAdapterPorts,
  EngineChild,
  EngineEvent,
  EngineEventStream,
  EngineProcess,
  EngineRunRequest,
  EngineSpawnCommand,
} from './types.js'

/** A clock the test moves: every reading is 2.5 seconds after the one before. */
class StepClock {
  constructor(private ms = Date.parse('2026-11-04T10:00:00.000Z')) {}

  now(): Date {
    const at = new Date(this.ms)
    this.ms += 2_500
    return at
  }
}

class ScriptedChild implements EngineChild {
  readonly stdout: AsyncIterable<string>
  readonly stderr: AsyncIterable<string>
  readonly exit: Promise<{ exitCode: number }>

  constructor(stdout: readonly string[], stderr: readonly string[], exitCode: number) {
    this.stdout = lines(stdout)
    this.stderr = lines(stderr)
    this.exit = Promise.resolve({ exitCode })
  }

  writeStdin(): Promise<void> {
    return Promise.resolve()
  }

  closeStdin(): Promise<void> {
    return Promise.resolve()
  }
}

async function* lines(values: readonly string[]): AsyncGenerator<string> {
  await Promise.resolve()
  yield* values
}

class FakeProcess implements EngineProcess {
  readonly spawned: EngineSpawnCommand[] = []
  throwOnSpawn: Error | undefined

  constructor(private readonly child: EngineChild) {}

  spawn(command: EngineSpawnCommand): EngineChild {
    if (this.throwOnSpawn !== undefined) throw this.throwOnSpawn
    this.spawned.push(command)
    return this.child
  }
}

const ENGINE: EngineDef = {
  argv: ['x', '{model}', '{effort}', '{output}'],
  promptDelivery: 'stdin',
  resultExtraction: { mode: 'output_file' },
  streamFormat: 'text',
  name: 'codex',
}

function request(overrides: Partial<EngineRunRequest> = {}): EngineRunRequest {
  return {
    effort: 'medium',
    engine: ENGINE,
    model: 'gpt-5.5',
    outputFile: '/out/output.txt',
    prompt: 'do the work please',
    promptFile: '/out/prompt.md',
    cwd: '/repo',
    ...overrides,
  }
}

function ports(process: EngineProcess, output = 'done'): EngineAdapterPorts {
  return { files: { readText: () => Promise.resolve(output), writeText: () => Promise.resolve() }, process }
}

async function collect(stream: EngineEventStream): Promise<readonly EngineEvent[]> {
  const events: EngineEvent[] = []
  for await (const event of stream) events.push(event)
  return events
}

const CONTEXT = { repo: '/repo', workflow: 'run 1' }

describe('runGuardedEngine', () => {
  it('spawns nothing and refuses with the gateway’s reason when the gateway refuses', async () => {
    const guard = new FakeEngineGuard()
    guard.refusal = new ModelGatewayError('no-grant', 'no live grant covers workflow run-1')
    const process = new FakeProcess(new ScriptedChild([], [], 0))

    await expect(runGuardedEngine(request(), ports(process), new EngineGate(guard, new StepClock()), CONTEXT)).rejects.toThrow(
      'no live grant covers workflow run-1',
    )

    expect(process.spawned).toEqual([])
    expect(guard.recorded).toEqual([])
  })

  it('runs a codex engine as before and records it as ok when it returns a result', async () => {
    const guard = new FakeEngineGuard()
    const process = new FakeProcess(new ScriptedChild(['working\n'], [], 0))

    const stream = await runGuardedEngine(request(), ports(process, 'all done'), new EngineGate(guard, new StepClock()), CONTEXT)
    const events = await collect(stream)

    expect(process.spawned).toHaveLength(1)
    expect(events.map((event) => event.type)).toEqual(['started', 'progress', 'result'])
    expect(guard.authorized).toEqual([
      { carries_history: true, client: 'codex', model: 'gpt-5.5', prompt_chars: 18, repo: '/repo', workflow: 'run-1' },
    ])
    expect(guard.recorded).toHaveLength(1)
    expect(guard.recorded[0]).toMatchObject({ duration_s: 2.5, outcome: 'ok' })
    expect(guard.recorded[0]).not.toHaveProperty('cost_usd')
    expect(guard.recorded[0]).not.toHaveProperty('tokens')
  })

  it('records the cost and tokens a claude engine reports', async () => {
    const guard = new FakeEngineGuard()
    const result = JSON.stringify({
      cost_usd: 0.4,
      result: 'final',
      type: 'result',
      usage: { cache_read_input_tokens: 3, input_tokens: 10, output_tokens: 20 },
    })
    const process = new FakeProcess(new ScriptedChild([`${result}\n`], [], 0))

    const stream = await runGuardedEngine(
      request({ engine: { ...ENGINE, name: 'claude' }, model: 'haiku' }),
      ports(process),
      new EngineGate(guard, new StepClock()),
      CONTEXT,
    )
    await collect(stream)

    expect(guard.recorded[0]).toMatchObject({ cost_usd: 0.4, outcome: 'ok', tokens: { cached: 3, input: 13, output: 20 } })
  })

  it('takes a result whose usage is missing or malformed as one without tokens', async () => {
    const guard = new FakeEngineGuard()
    const result = JSON.stringify({ result: 'final', type: 'result', usage: { input_tokens: 'many' } })
    const process = new FakeProcess(new ScriptedChild([`${result}\n`], [], 0))

    await collect(
      await runGuardedEngine(request({ engine: { ...ENGINE, name: 'claude' } }), ports(process), new EngineGate(guard, new StepClock()), CONTEXT),
    )

    expect(guard.recorded[0]).not.toHaveProperty('tokens')
  })

  it('takes a result that reports no usage at all as one without tokens, and runs without a repository', async () => {
    const guard = new FakeEngineGuard()
    const process = new FakeProcess(new ScriptedChild([`${JSON.stringify({ result: 'final', type: 'result' })}\n`], [], 0))

    await collect(
      await runGuardedEngine(request({ engine: { ...ENGINE, name: 'claude' } }), ports(process), new EngineGate(guard, new StepClock()), { workflow: 'run-1' }),
    )

    expect(guard.authorized[0]).not.toHaveProperty('repo')
    expect(guard.recorded[0]).not.toHaveProperty('tokens')
  })

  it('records a run that exits non-zero as failed with its exit code', async () => {
    const guard = new FakeEngineGuard()
    const process = new FakeProcess(new ScriptedChild([], ['boom\n'], 3))

    const events = await collect(await runGuardedEngine(request(), ports(process), new EngineGate(guard, new StepClock()), CONTEXT))

    expect(events.at(-1)?.type).toBe('failed')
    expect(guard.recorded[0]).toMatchObject({ outcome: 'failed', reason: 'exit-3' })
  })

  it('records a run its consumer abandons as failed, so the reservation never lingers', async () => {
    const guard = new FakeEngineGuard()
    const process = new FakeProcess(new ScriptedChild(['a\n', 'b\n'], [], 0))

    const stream = await runGuardedEngine(request(), ports(process), new EngineGate(guard, new StepClock()), CONTEXT)
    for await (const event of stream) {
      if (event.type === 'progress') break
    }

    expect(guard.recorded[0]).toMatchObject({ outcome: 'failed', reason: 'aborted' })
  })

  it('records a run that cannot be spawned as failed to launch and raises its error', async () => {
    const guard = new FakeEngineGuard()
    const process = new FakeProcess(new ScriptedChild([], [], 0))
    process.throwOnSpawn = new Error('spawn codex ENOENT')

    await expect(runGuardedEngine(request(), ports(process), new EngineGate(guard, new StepClock()), CONTEXT)).rejects.toThrow('spawn codex ENOENT')

    expect(guard.recorded[0]).toMatchObject({ outcome: 'failed', reason: 'launch-failed' })
  })

  it('leaves an engine the gateway does not guard exactly as it was', async () => {
    const guard = new FakeEngineGuard()
    const process = new FakeProcess(new ScriptedChild(['out\n'], [], 0))

    const stream = await runGuardedEngine(
      request({ engine: { ...ENGINE, name: 'generic' } }),
      ports(process),
      new EngineGate(guard, new StepClock()),
      CONTEXT,
    )
    await collect(stream)

    expect(guard.authorized).toEqual([])
    expect(guard.recorded).toEqual([])
    expect(process.spawned).toHaveLength(1)
  })

  it('passes injection and input closing through to the engine it guards', async () => {
    const guard = new FakeEngineGuard()
    const process = new FakeProcess(new ScriptedChild(['a\n'], [], 0))

    const stream = await runGuardedEngine(request(), ports(process), new EngineGate(guard, new StepClock()), CONTEXT)

    expect(stream.command.command).toBe('codex')
    expect(stream.injection.mode).toBe('checkpoint_resume')
    expect((await stream.inject('more')).mode).toBe('checkpoint_resume')
    await stream.closeInput()
    await collect(stream)
  })
})
