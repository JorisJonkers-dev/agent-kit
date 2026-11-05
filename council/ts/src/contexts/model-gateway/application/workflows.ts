import { isObject, ModelGatewayError } from '../domain/config.js'
import {
  endWorkflow,
  requireWorkflowName,
  startWorkflow,
  workflowView,
  workflowViews,
  type WorkflowView,
} from '../domain/workflows.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

/** `workflow start` and `workflow end` take the name of the workflow they act on. */
export interface WorkflowInput {
  readonly name: string
}

/** One workflow, as `workflow start` and `workflow end` report it. */
export interface WorkflowResult {
  readonly status: 'ok'
  readonly workflow: WorkflowView
}

/** Every workflow this machine has started, as `workflow list` reports them. */
export interface WorkflowListResult {
  readonly status: 'ok'
  readonly workflows: readonly WorkflowView[]
}

/**
 * Starts a workflow, or reports the one that is already running. A workflow
 * that ended is refused: its caps and grants lapsed with it, so a new piece of
 * work needs a new name.
 */
export class WorkflowStartCommand implements ModelGatewayCommand<unknown, WorkflowResult> {
  constructor(private readonly deps: ModelGatewayDependencies) {}

  async run(raw: unknown): Promise<WorkflowResult> {
    const input = readWorkflowInput(raw)
    const now = this.deps.clock.now()
    const workflow = await this.deps.workflows.update((current) => startWorkflow(current, input.name, now))
    return { status: 'ok', workflow: workflowView(workflow, now) }
  }
}

/** Ends a workflow at this moment; one that already ended keeps the moment it ended. */
export class WorkflowEndCommand implements ModelGatewayCommand<unknown, WorkflowResult> {
  constructor(private readonly deps: ModelGatewayDependencies) {}

  async run(raw: unknown): Promise<WorkflowResult> {
    const input = readWorkflowInput(raw)
    const now = this.deps.clock.now()
    const workflow = await this.deps.workflows.update((current) => endWorkflow(current, input.name, now))
    return { status: 'ok', workflow: workflowView(workflow, now) }
  }
}

/**
 * Lists every workflow this machine has started, with the 24-hour end worked
 * out from the clock: one nobody ended within a day counts as ended.
 */
export class WorkflowListCommand implements ModelGatewayCommand<unknown, WorkflowListResult> {
  constructor(private readonly deps: ModelGatewayDependencies) {}

  async run(raw: unknown): Promise<WorkflowListResult> {
    readListInput(raw)
    return { status: 'ok', workflows: workflowViews(await this.deps.workflows.read(), this.deps.clock.now()) }
  }
}

function readWorkflowInput(raw: unknown): WorkflowInput {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
  return { name: requireWorkflowName(raw.name, 'a workflow command must name its workflow') }
}

/** `workflow list` takes no field: it lists every workflow this machine has started. */
function readListInput(raw: unknown): void {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
}
