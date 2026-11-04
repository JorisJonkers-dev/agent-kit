import { GrantListCommand, GrantWriteCommand, type GrantListResult, type GrantWriteResult } from './grants.js'
import { ModelsCommand, type ModelsInput, type ModelsResult } from './models.js'
import { RunCommand, type RunOutcome } from './run.js'
import { UsageCommand } from './usage.js'
import { WorktreesCleanCommand, type WorktreesCleanResult } from './worktrees.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'
import type { UsageResult } from '../domain/usage.js'
import {
  WorkflowEndCommand,
  WorkflowListCommand,
  WorkflowStartCommand,
  type WorkflowListResult,
  type WorkflowResult,
} from './workflows.js'

/** The gateway's commands. */
export interface ModelGatewayCommands {
  readonly grantList: ModelGatewayCommand<unknown, GrantListResult>
  readonly grantWrite: ModelGatewayCommand<unknown, GrantWriteResult>
  readonly models: ModelGatewayCommand<ModelsInput, ModelsResult>
  readonly run: ModelGatewayCommand<unknown, RunOutcome>
  readonly usage: ModelGatewayCommand<unknown, UsageResult>
  readonly worktreesClean: ModelGatewayCommand<unknown, WorktreesCleanResult>
  readonly workflowStart: ModelGatewayCommand<unknown, WorkflowResult>
  readonly workflowEnd: ModelGatewayCommand<unknown, WorkflowResult>
  readonly workflowList: ModelGatewayCommand<unknown, WorkflowListResult>
}

export function createModelGatewayCommands(deps: ModelGatewayDependencies): ModelGatewayCommands {
  return {
    grantList: new GrantListCommand(deps),
    grantWrite: new GrantWriteCommand(deps),
    models: new ModelsCommand(deps),
    run: new RunCommand(deps),
    usage: new UsageCommand(deps),
    worktreesClean: new WorktreesCleanCommand(deps),
    workflowEnd: new WorkflowEndCommand(deps),
    workflowList: new WorkflowListCommand(deps),
    workflowStart: new WorkflowStartCommand(deps),
  }
}
