import type { EngineDef } from '../../../../domain/engines/index.js'
import type { EngineRunRequest, EngineSpawnCommand } from './types.js'

export function expandCommand(engine: EngineDef, request: EngineRunRequest): EngineSpawnCommand {
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

export function commandParts(
  command: string,
  args: readonly string[],
): Pick<EngineSpawnCommand, 'args' | 'command'> {
  return { command, args }
}

export function optionalCwd(
  cwd: string | undefined,
): Pick<EngineSpawnCommand, 'cwd'> | Record<string, never> {
  return cwd === undefined ? {} : { cwd }
}
