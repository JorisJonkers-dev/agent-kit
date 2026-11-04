import { ModelsCommand, type ModelsInput, type ModelsResult } from './models.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

/** The gateway's commands. */
export interface ModelGatewayCommands {
  readonly models: ModelGatewayCommand<ModelsInput, ModelsResult>
}

export function createModelGatewayCommands(deps: ModelGatewayDependencies): ModelGatewayCommands {
  return { models: new ModelsCommand(deps) }
}
