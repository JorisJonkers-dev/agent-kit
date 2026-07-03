import type { JsonRecord } from '../shared-kernel/index.js'
import type { RunSummary } from './status.js'

export interface ReviewPackInput {
  readonly gate: '1' | 'design' | '2'
  readonly runDir: string
}

export interface ReviewPackWorkflowDeps {
  readonly status: (input: { readonly runDir: string }) => Promise<RunSummary>
}

export async function reviewPackWorkflow(input: ReviewPackInput, deps: ReviewPackWorkflowDeps): Promise<JsonRecord> {
  const summary = await deps.status({ runDir: input.runDir })
  return {
    gate: input.gate,
    run: summary.run,
    task_count: summary.tasks.length,
    waves: summary.waves,
    worker_results: summary.workerResults.length,
  }
}
