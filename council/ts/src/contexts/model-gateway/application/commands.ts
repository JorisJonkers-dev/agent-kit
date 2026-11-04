import { ModelsCommand, type ModelsInput, type ModelsResult } from './models.js'
import { RunCommand, type RunResult } from './run.js'
import { UsageCommand } from './usage.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'
import type { UsageResult } from '../domain/usage.js'

/** The gateway's commands. */
export interface ModelGatewayCommands {
  readonly models: ModelGatewayCommand<ModelsInput, ModelsResult>
  readonly run: ModelGatewayCommand<unknown, RunResult>
  readonly usage: ModelGatewayCommand<unknown, UsageResult>
}

export function createModelGatewayCommands(deps: ModelGatewayDependencies): ModelGatewayCommands {
  return { models: new ModelsCommand(deps), run: new RunCommand(deps), usage: new UsageCommand(deps) }
}
