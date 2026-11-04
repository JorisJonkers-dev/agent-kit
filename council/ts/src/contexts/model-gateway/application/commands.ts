import { ModelsCommand, type ModelsInput, type ModelsResult } from './models.js'
import { RunCommand, type RunResult } from './run.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'
import {
  WorkflowEndCommand,
  WorkflowListCommand,
  WorkflowStartCommand,
  type WorkflowListResult,
  type WorkflowResult,
} from './workflows.js'

/** The gateway's commands. */
export interface ModelGatewayCommands {
  readonly models: ModelGatewayCommand<ModelsInput, ModelsResult>
  readonly run: ModelGatewayCommand<unknown, RunResult>
  readonly workflowStart: ModelGatewayCommand<unknown, WorkflowResult>
  readonly workflowEnd: ModelGatewayCommand<unknown, WorkflowResult>
  readonly workflowList: ModelGatewayCommand<unknown, WorkflowListResult>
}

export function createModelGatewayCommands(deps: ModelGatewayDependencies): ModelGatewayCommands {
  return {
    models: new ModelsCommand(deps),
    run: new RunCommand(deps),
    workflowEnd: new WorkflowEndCommand(deps),
    workflowList: new WorkflowListCommand(deps),
    workflowStart: new WorkflowStartCommand(deps),
  }
}
