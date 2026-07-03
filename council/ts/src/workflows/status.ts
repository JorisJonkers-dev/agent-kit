import { planWaves } from '../contexts/graph/index.js'
import type { LegacyRunNormalizerPort, LegacyRunReport, WorkerResult } from '../ports/index.js'
import type { RunState, Task } from '../shared-kernel/index.js'

export interface RunSummary {
  readonly report?: LegacyRunReport
  readonly run: string
  readonly state: RunState
  readonly tasks: readonly Task[]
  readonly waves: readonly (readonly string[])[]
  readonly workerResults: readonly WorkerResult[]
}

export async function statusWorkflow(
  input: { readonly runDir: string },
  deps: LegacyRunNormalizerPort,
): Promise<RunSummary> {
  const normalized = await deps.normalizeRunDir(input.runDir)
  return {
    ...(normalized.report ? { report: normalized.report } : {}),
    run: normalized.runId,
    state: normalized.state,
    tasks: normalized.tasks,
    waves: normalized.report?.waves ?? planWaves(normalized.tasks),
    workerResults: [...normalized.workerResults.values()],
  }
}
