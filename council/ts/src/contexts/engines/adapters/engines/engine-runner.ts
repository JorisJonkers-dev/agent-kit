import type { EngineAdapterPorts, EngineEventStream, EngineRunRequest } from './types.js'
import { runClaudeEngine } from './claude-driver.js'
import { runCodexEngine } from './codex-driver.js'
import { runGenericCommandEngine } from './generic-driver.js'

export function runEngine(
  request: EngineRunRequest,
  ports: EngineAdapterPorts,
): EngineEventStream {
  if (request.engine.name === 'claude') {
    return runClaudeEngine(request, ports)
  }
  if (request.engine.name === 'codex') {
    return runCodexEngine(request, ports)
  }
  return runGenericCommandEngine(request, ports)
}
