import { isObject, ModelGatewayError } from './config.js'
import { readRunInput, type RunInput } from './run.js'
import { requireWorkflowName } from './workflows.js'

/**
 * A batch plan: a whole set of delegations handed over in one call, each with
 * an id, the delegations it waits for, and the run it is. The plan is checked
 * as a whole before anything runs, and a delegation that fails for good skips
 * only what depends on it.
 */

/** The characters a delegation's id may hold: letters, digits and `._-`, 1 to 40 of them. */
export const DELEGATION_ID = /^[A-Za-z0-9._-]{1,40}$/

/** The keys a delegation of a batch may not carry: the workflow is the batch's, and a delegation is never a batch. */
const BATCH_ONLY_KEYS = ['workflow', 'delegations', 'batch'] as const

/** One delegation of a plan: its id, what it waits for, and the run it is (in the batch's workflow). */
export interface BatchDelegation {
  readonly id: string
  readonly depends_on: readonly string[]
  readonly input: RunInput
}

/** A batch plan as the caller sends it, read and checked for shape. */
export interface BatchInput {
  readonly workflow: string
  readonly delegations: readonly BatchDelegation[]
}

/** One thing wrong with a plan, naming the delegation it is wrong with. */
export interface PlanProblem {
  readonly id: string
  readonly code: string
  readonly message: string
}

/** A plan the gateway refused before launching anything, with every problem it found. */
export class PlanRefusedError extends ModelGatewayError {
  constructor(readonly problems: readonly PlanProblem[]) {
    super(
      'plan-refused',
      `the plan is refused, and nothing was launched: ${problems.map((problem) => `${problem.id}: ${problem.message}`).join('; ')}`,
    )
    this.name = 'PlanRefusedError'
  }
}

/** Reads a plan, or says what is wrong with its shape: the workflow, the ids and each delegation's own fields. */
export function readBatchInput(raw: unknown, maxDelegations: number): BatchInput {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
  const workflow = requireWorkflowName(raw.workflow, 'a batch must name its workflow')
  if (!Array.isArray(raw.delegations) || raw.delegations.length === 0) {
    throw new ModelGatewayError('invalid-input', 'a batch must list its delegations: a non-empty list')
  }
  const items: readonly unknown[] = raw.delegations
  if (items.length > maxDelegations) {
    throw new PlanRefusedError([
      {
        code: 'too-many-delegations',
        id: '(plan)',
        message: `the plan holds ${String(items.length)} delegations and this machine allows at most ${String(maxDelegations)} (settings.max_batch_delegations)`,
      },
    ])
  }
  const delegations: BatchDelegation[] = []
  for (const [index, item] of items.entries()) {
    const delegation = readDelegation(item, index, workflow)
    if (delegations.some((earlier) => earlier.id === delegation.id)) {
      throw new ModelGatewayError('invalid-input', `delegation id ${delegation.id} is used twice: ids are unique in a plan`)
    }
    delegations.push(delegation)
  }
  return { delegations, workflow }
}

function readDelegation(raw: unknown, index: number, workflow: string): BatchDelegation {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', `delegations[${String(index)}] must be an object`)
  const { id, depends_on: dependsOn, ...rest } = raw
  if (typeof id !== 'string' || !DELEGATION_ID.test(id)) {
    throw new ModelGatewayError(
      'invalid-input',
      `delegations[${String(index)}].id must be 1 to 40 characters of letters, digits and . _ - only`,
    )
  }
  const forbidden = BATCH_ONLY_KEYS.find((key) => rest[key] !== undefined)
  if (forbidden !== undefined) {
    throw new ModelGatewayError(
      'invalid-input',
      `delegation ${id} names ${forbidden}: the workflow is the batch's, and a delegation inside a batch cannot be a batch`,
    )
  }
  try {
    return { depends_on: readDependsOn(dependsOn), id, input: readRunInput({ ...rest, workflow }) }
  } catch (error) {
    throw error instanceof ModelGatewayError ? new ModelGatewayError(error.code, `delegation ${id}: ${error.message}`) : error
  }
}

function readDependsOn(value: unknown): readonly string[] {
  if (value === undefined) return []
  if (!Array.isArray(value) || value.some((entry) => typeof entry !== 'string')) {
    throw new ModelGatewayError('invalid-input', 'depends_on must be a list of delegation ids')
  }
  return [...new Set(value as readonly string[])]
}

/**
 * What is wrong with the plan's dependencies: a `depends_on` that names no
 * delegation, and every delegation that is part of a cycle (a delegation that
 * waits on itself, directly or through others, can never start).
 */
export function dependencyProblems(delegations: readonly BatchDelegation[]): readonly PlanProblem[] {
  const ids = new Set(delegations.map((delegation) => delegation.id))
  const problems: PlanProblem[] = []
  for (const delegation of delegations) {
    for (const dependency of delegation.depends_on) {
      if (!ids.has(dependency)) {
        problems.push({ code: 'unknown-dependency', id: delegation.id, message: `depends_on names ${dependency}, which is not in the plan` })
      }
    }
  }
  for (const delegation of delegations) {
    if (reachableFrom(delegation.id, delegations).has(delegation.id)) {
      problems.push({ code: 'cycle', id: delegation.id, message: 'is part of a dependency cycle, so it could never start' })
    }
  }
  return problems
}

/** Every delegation the one named waits for, directly or through others. */
function reachableFrom(start: string, delegations: readonly BatchDelegation[]): ReadonlySet<string> {
  const reached = new Set<string>()
  const queue = [start]
  while (queue.length > 0) {
    const current = queue.pop()
    for (const dependency of delegations.find((delegation) => delegation.id === current)?.depends_on ?? []) {
      if (reached.has(dependency)) continue
      reached.add(dependency)
      queue.push(dependency)
    }
  }
  return reached
}

/** What a dependent is told of one delegation it waits for: where its full output is, and its capped summary. */
export interface DependencyOutput {
  readonly id: string
  /** Null for a delegation that is done and left no output file. */
  readonly output_path: string | null
  readonly summary: string
}

/** The heading of the block a dependent's prompt ends with. */
export const OUTPUTS_HEADING = 'Outputs of the delegations this one depends on:'

/**
 * The prompt a dependent is launched with: its own, and then, for each
 * delegation it waits for, the id, the path of the full output and the capped
 * summary. Never the full output.
 */
export function promptWithOutputs(prompt: string, outputs: readonly DependencyOutput[]): string {
  if (outputs.length === 0) return prompt
  const lines = outputs.map(
    (output) => `- ${output.id}: ${output.output_path === null ? 'no output file' : `output at ${output.output_path}`}\n  summary: ${output.summary}`,
  )
  return `${prompt}\n\n${OUTPUTS_HEADING}\n${lines.join('\n')}`
}

/** What a path in the block is estimated to take, since the plan cannot know where a run's directory will be. */
const ESTIMATED_PATH_CHARS = 300

/**
 * The prompt a dependent is estimated at: its own, and for each delegation it
 * waits for a block as long as the longest it can be (an id, a long path and
 * a summary at its cap), so the reserved estimate is never below what the
 * run's prompt will cost.
 */
export function estimatedPrompt(prompt: string, dependsOn: readonly string[], summaryMaxChars: number): string {
  return promptWithOutputs(
    prompt,
    dependsOn.map((id) => ({ id, output_path: 'p'.repeat(ESTIMATED_PATH_CHARS), summary: 's'.repeat(summaryMaxChars) })),
  )
}

/** One delegation's place in the result. */
export type DelegationStatus = 'done' | 'failed' | 'skipped'

/** `ok` when every delegation is done, `failed` when none is, `partial` otherwise. */
export type BatchStatus = 'ok' | 'partial' | 'failed'

export function batchStatus(statuses: readonly DelegationStatus[]): BatchStatus {
  const done = statuses.filter((status) => status === 'done').length
  if (done === statuses.length) return 'ok'
  return done === 0 ? 'failed' : 'partial'
}
