import type { RunTokens } from '../../../model-gateway/index.js'
import type { JsonRecord } from '../../../../shared-kernel/index.js'
import { runEngine } from './engine-runner.js'
import type { EngineEnd, EngineGate, EngineLease } from './engine-gate.js'
import { isJsonRecord } from './json.js'
import type { EngineAdapterPorts, EngineEvent, EngineEventStream, EngineRunRequest } from './types.js'

/** What a guarded engine run is launched under: the council run it belongs to, and the repository it works in. */
export interface GuardedRunContext {
  readonly workflow: string
  readonly repo?: string
}

/**
 * `runEngine`, behind the gateway: a `claude` or `codex` engine is authorised
 * before its process is spawned, and recorded when its event stream ends. A
 * refusal rejects here with the gateway's reason and spawns nothing. The stream
 * itself (events, injection, closing input) is the engine's own, unchanged.
 */
export async function runGuardedEngine(
  request: EngineRunRequest,
  ports: EngineAdapterPorts,
  gate: EngineGate,
  context: GuardedRunContext,
): Promise<EngineEventStream> {
  const lease = await gate.open({
    engine: request.engine.name,
    model: request.model,
    promptChars: request.prompt.length,
    workflow: context.workflow,
    ...(context.repo === undefined ? {} : { repo: context.repo }),
  })
  if (lease === undefined) return runEngine(request, ports)
  let stream: EngineEventStream
  try {
    stream = runEngine(request, ports)
  } catch (error) {
    await lease.end({ outcome: 'failed', reason: 'launch-failed' })
    throw error
  }
  return leased(stream, lease)
}

/** The engine's stream, which ends its lease with what the run came to. */
function leased(stream: EngineEventStream, lease: EngineLease): EngineEventStream {
  return {
    closeInput: () => stream.closeInput(),
    command: stream.command,
    inject: (prompt) => stream.inject(prompt),
    injection: stream.injection,
    async *[Symbol.asyncIterator](): AsyncGenerator<EngineEvent> {
      let end: EngineEnd = { outcome: 'failed', reason: 'aborted' }
      try {
        for await (const event of stream) {
          end = endOf(event) ?? end
          yield event
        }
      } finally {
        await lease.end(end)
      }
    },
  }
}

/** What a terminal event says the run came to; undefined for an event that is not the end. */
function endOf(event: EngineEvent): EngineEnd | undefined {
  if (event.type === 'failed') return { outcome: 'failed', reason: `exit-${String(event.exitCode)}` }
  if (event.type !== 'result') return undefined
  const tokens = event.metadata === undefined ? undefined : tokensOf(event.metadata)
  return {
    outcome: 'ok',
    ...(event.costUsd === undefined ? {} : { costUsd: event.costUsd }),
    ...(tokens === undefined ? {} : { tokens }),
  }
}

/** The tokens a result's own usage block reports (Claude's `usage`), or undefined when it reports none or a malformed count. */
function tokensOf(metadata: JsonRecord): RunTokens | undefined {
  const usage = metadata.usage
  if (!isJsonRecord(usage)) return undefined
  const input = usage.input_tokens
  const output = usage.output_tokens
  if (typeof input !== 'number' || typeof output !== 'number') return undefined
  const cached = usage.cache_read_input_tokens
  return { cached: typeof cached === 'number' ? cached : 0, input: input + (typeof cached === 'number' ? cached : 0), output }
}
