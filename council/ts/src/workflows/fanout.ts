import {
  applyPreFanoutGate,
  createTaskGraph,
  type PreFanoutGateViolation,
} from '../contexts/graph/index.js'

import type { RunSummary } from './status.js'

export interface FanoutInput {
  readonly dryRun: boolean
  readonly github: boolean
  readonly repoFiles?: ReadonlySet<string>
  readonly runDir: string
}

export interface ExecutionPlan {
  readonly agents?: Readonly<Record<string, string>>
  readonly github: 'disabled' | 'dry-run' | 'created'
  readonly prUrl?: string
  readonly run: string
  readonly tasks: RunSummary['tasks']
  readonly waves: RunSummary['waves']
}

export interface FanoutWorkflowDeps {
  readonly createPullRequest: (run: string) => Promise<string>
  readonly status: (input: { readonly runDir: string }) => Promise<RunSummary>
}

export class PreFanoutGateError extends Error {
  readonly gateName = 'pre-fanout-static'
  readonly violations: readonly PreFanoutGateViolation[]

  constructor(violations: readonly PreFanoutGateViolation[]) {
    super(`pre-fanout static gate failed: ${violations.map(({ message }) => message).join('; ')}`)
    this.name = 'PreFanoutGateError'
    this.violations = violations
  }
}

export async function fanoutWorkflow(input: FanoutInput, deps: FanoutWorkflowDeps): Promise<ExecutionPlan> {
  const summary = await deps.status({ runDir: input.runDir })
  const gate = applyPreFanoutGate({
    graph: createTaskGraph(summary.tasks),
    repoFiles: repoFilesForGate(summary.tasks, input.repoFiles),
  })
  assertPreFanoutGatePassed(gate.violations)
  const github = await resolveGithub(input.github, input.dryRun, summary.run, deps.createPullRequest)
  return {
    github: github.kind,
    ...(github.url ? { prUrl: github.url } : {}),
    run: summary.run,
    tasks: summary.tasks,
    waves: gate.waves,
  }
}

export function assertPreFanoutGatePassed(
  violations: readonly PreFanoutGateViolation[],
): void {
  if (violations.length === 0) return
  throw new PreFanoutGateError(violations)
}

export function repoFilesForGate(
  tasks: RunSummary['tasks'],
  repoFiles: ReadonlySet<string> | undefined,
): ReadonlySet<string> {
  return repoFiles ?? new Set(tasks.flatMap((task) => task.paths))
}

async function resolveGithub(
  github: boolean,
  dryRun: boolean,
  run: string,
  createPullRequest: (run: string) => Promise<string>,
): Promise<{ readonly kind: ExecutionPlan['github']; readonly url?: string }> {
  if (!github) return { kind: 'disabled' }
  if (dryRun) return { kind: 'dry-run' }
  return { kind: 'created', url: await createPullRequest(run) }
}
