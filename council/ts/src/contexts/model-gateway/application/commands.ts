import { ModelsCommand, type ModelsInput, type ModelsResult } from './models.js'
import { RunCommand, type RunResult } from './run.js'
import { WorktreesCleanCommand, type WorktreesCleanResult } from './worktrees.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

/** The gateway's commands. */
export interface ModelGatewayCommands {
  readonly models: ModelGatewayCommand<ModelsInput, ModelsResult>
  readonly run: ModelGatewayCommand<unknown, RunResult>
  readonly worktreesClean: ModelGatewayCommand<unknown, WorktreesCleanResult>
}

export function createModelGatewayCommands(deps: ModelGatewayDependencies): ModelGatewayCommands {
  return { models: new ModelsCommand(deps), run: new RunCommand(deps), worktreesClean: new WorktreesCleanCommand(deps) }
}
