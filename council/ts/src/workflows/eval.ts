import {
  scoreEvalRun,
  type EvalCategoryScores,
  type EvalFinding,
  type EvalReportData,
  type EvalRunSummary,
  type EvalScorecard,
  type EvalStatus,
} from '../contexts/eval/index.js'
import type { RunStoreEvent, WorkerLifecycleEvent } from '../contexts/runstore/index.js'
import type { RunSummary } from './status.js'

export interface EvalWorkflowInput {
  readonly runDir: string
}

export interface EvalWorkflowDeps {
  readonly readEvents: (runId: string) => Promise<readonly RunStoreEvent[]>
  readonly status: (input: { readonly runDir: string }) => Promise<RunSummary>
}

export interface EvalWorkflowSummary {
  readonly status: EvalStatus
  readonly task_count: number
  readonly worker_result_count: number
  readonly report_task_count: number
  readonly wave_count: number
  readonly finding_count: number
  readonly critical_finding_count: number
  readonly warning_finding_count: number
  readonly retry_count: number
  readonly no_op_count: number
  readonly out_of_bounds_count: number
  readonly missing_worker_result_count: number
}

export interface EvalWorkflowResult {
  readonly run: string
  readonly score: number
  readonly status: EvalStatus
  readonly categories: EvalCategoryScores
  readonly findings: readonly EvalFinding[]
  readonly summary: EvalWorkflowSummary
}

export async function evalWorkflow(
  input: EvalWorkflowInput,
  deps: EvalWorkflowDeps,
): Promise<EvalWorkflowResult> {
  const summary = await deps.status({ runDir: input.runDir })
  const events = await deps.readEvents(summary.run)
  const report = evalReportFrom(summary)
  const scorecard = scoreEvalRun({
    events: events.filter(isWorkerLifecycleEvent),
    ...(report === undefined ? {} : { report }),
    tasks: summary.tasks,
    worker_results: summary.workerResults,
  })

  return workflowResult(summary.run, scorecard)
}

function evalReportFrom(summary: RunSummary): EvalReportData | undefined {
  if (summary.report === undefined) {
    return undefined
  }

  return {
    task_reports: summary.report.tasks,
    waves: summary.waves,
  }
}

function workflowResult(runId: string, scorecard: EvalScorecard): EvalWorkflowResult {
  return {
    categories: scorecard.categories,
    findings: scorecard.findings,
    run: runId,
    score: scorecard.total_score,
    status: scorecard.status,
    summary: workflowSummary(scorecard.summary, scorecard.findings),
  }
}

function workflowSummary(
  summary: EvalRunSummary,
  findings: readonly EvalFinding[],
): EvalWorkflowSummary {
  return {
    critical_finding_count: findings.filter((finding) => finding.severity === 'critical').length,
    finding_count: findings.length,
    missing_worker_result_count: summary.missing_worker_result_count,
    no_op_count: summary.no_op_count,
    out_of_bounds_count: summary.out_of_bounds_count,
    report_task_count: summary.report_task_count,
    retry_count: summary.retry_count,
    status: summary.status,
    task_count: summary.task_count,
    warning_finding_count: findings.filter((finding) => finding.severity === 'warning').length,
    wave_count: summary.wave_count,
    worker_result_count: summary.worker_result_count,
  }
}

function isWorkerLifecycleEvent(event: RunStoreEvent): event is WorkerLifecycleEvent {
  return (
    event.type === 'worker_detected' ||
    event.type === 'worker_exited' ||
    event.type === 'worker_finished' ||
    event.type === 'worker_output' ||
    event.type === 'worker_restarted' ||
    event.type === 'worker_started'
  )
}
