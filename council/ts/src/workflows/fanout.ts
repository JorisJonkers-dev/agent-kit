import type { RunSummary } from './status.js'

export interface FanoutInput {
  readonly dryRun: boolean
  readonly github: boolean
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

export async function fanoutWorkflow(input: FanoutInput, deps: FanoutWorkflowDeps): Promise<ExecutionPlan> {
  const summary = await deps.status({ runDir: input.runDir })
  const github = await resolveGithub(input.github, input.dryRun, summary.run, deps.createPullRequest)
  return {
    github: github.kind,
    ...(github.url ? { prUrl: github.url } : {}),
    run: summary.run,
    tasks: summary.tasks,
    waves: summary.waves,
  }
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
