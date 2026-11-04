import type { EngineDef } from './domain/index.js'

export * from './domain/index.js'
export * from './domain/guard.js'
export {
  EngineGate,
  engineLeaseFileText,
  parseEngineLeaseFile,
  type EngineLeaseFile,
  type PersistedEngineLease,
  type EngineEnd,
  type EngineGateClock,
  type EngineLaunch,
  type EngineLease,
  type EngineRunGuard,
} from './adapters/engines/engine-gate.js'

export interface EngineSpawnCommand {
  readonly args: readonly string[]
  readonly command: string
  readonly cwd?: string
}

export interface ExpandEngineCommandRequest {
  readonly cwd?: string
  readonly effort: string
  readonly engine: EngineDef
  readonly model: string
  readonly outputFile: string
  readonly promptFile: string
}

export function expandEngineCommand(request: ExpandEngineCommandRequest): EngineSpawnCommand {
  const [command, ...args] = request.engine.argv.map((arg) =>
    arg
      .replaceAll('{model}', request.model)
      .replaceAll('{effort}', request.effort)
      .replaceAll('{output}', request.outputFile)
      .replaceAll('{prompt_file}', request.promptFile),
  )

  return request.cwd === undefined
    ? { args, command: command ?? request.engine.name }
    : { args, command: command ?? request.engine.name, cwd: request.cwd }
}
