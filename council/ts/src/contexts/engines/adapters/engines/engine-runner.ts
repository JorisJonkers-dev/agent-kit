import type { EngineAdapterPorts, EngineDriver, EngineEventStream, EngineRunRequest } from './types.js'
import { runClaudeEngine } from './claude-driver.js'
import { runCodexEngine } from './codex-driver.js'
import { runGenericCommandEngine } from './generic-driver.js'

class DelegatingEngineDriver implements EngineDriver {
  constructor(
    private readonly runEngine: (
      request: EngineRunRequest,
      ports: EngineAdapterPorts,
    ) => EngineEventStream,
  ) {}

  run(request: EngineRunRequest, ports: EngineAdapterPorts): EngineEventStream {
    return this.runEngine(request, ports)
  }
}

const ENGINE_DRIVERS: Readonly<Record<string, EngineDriver>> = Object.freeze({
  claude: new DelegatingEngineDriver(runClaudeEngine),
  codex: new DelegatingEngineDriver(runCodexEngine),
})
const GENERIC_ENGINE_DRIVER: EngineDriver = new DelegatingEngineDriver(runGenericCommandEngine)

export function runEngine(
  request: EngineRunRequest,
  ports: EngineAdapterPorts,
): EngineEventStream {
  return selectEngineDriver(request.engine.name).run(request, ports)
}

function selectEngineDriver(engineName: string): EngineDriver {
  return ENGINE_DRIVERS[engineName] ?? GENERIC_ENGINE_DRIVER
}
