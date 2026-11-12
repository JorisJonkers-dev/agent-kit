import { BatchCommand, type BatchResult } from './batch.js'
import {
  GrantListCommand,
  GrantReserveCommand,
  GrantWriteCommand,
  type GrantListResult,
  type GrantReserveResult,
  type GrantWriteResult,
} from './grants.js'
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
  readonly batch: ModelGatewayCommand<unknown, BatchResult>
  readonly grantList: ModelGatewayCommand<unknown, GrantListResult>
  readonly grantReserve: ModelGatewayCommand<unknown, GrantReserveResult>
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
  const run = new RunCommand(deps)
  return {
    batch: new BatchCommand(deps, run),
    grantList: new GrantListCommand(deps),
    grantReserve: new GrantReserveCommand(deps),
    grantWrite: new GrantWriteCommand(deps),
    models: new ModelsCommand(deps),
    run,
    usage: new UsageCommand(deps),
    worktreesClean: new WorktreesCleanCommand(deps),
    workflowEnd: new WorkflowEndCommand(deps),
    workflowList: new WorkflowListCommand(deps),
    workflowStart: new WorkflowStartCommand(deps),
  }
}
