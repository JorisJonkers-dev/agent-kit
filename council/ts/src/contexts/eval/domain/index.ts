import type { ReviewVerdict, Task, TaskId } from '../../../shared-kernel/index.js'
import type { WorkerLifecycleEvent } from '../../runstore/index.js'

export type EvalCategory =
  | 'boundary_compliance'
  | 'verify_relevance'
  | 'retries'
  | 'no_op_rate'
  | 'out_of_bounds_edits'
  | 'lucky_pass_suspicion'
  | 'result_completeness'

export type EvalFindingSeverity = 'warning' | 'critical'

export type EvalFindingCode =
  | 'boundary-drift'
  | 'weak-verify-pass'
  | 'retry-heavy-run'
  | 'no-op-worker'
  | 'out-of-bounds-edit'
  | 'lucky-pass-no-op'
  | 'satisfied-verdict-failed-verify'
  | 'missing-worker-result'

export type EvalStatus = 'pass' | 'warn' | 'fail'

export interface EvalWorkerResult {
  readonly task_id: string
  readonly status: string
  readonly files_changed?: readonly string[]
  readonly out_of_bounds?: readonly string[]
  readonly verify_rc?: number | null
  readonly verdict?: ReviewVerdict | null
}

export interface EvalTaskReport {
  readonly task_id: string
  readonly status?: string
  readonly files_changed?: readonly string[]
  readonly verify_rc?: number | null
  readonly verifier_satisfied?: boolean
  readonly out_of_bounds?: readonly string[]
}

export interface EvalReportData {
  readonly waves?: readonly (readonly string[])[]
  readonly task_reports?: readonly EvalTaskReport[]
}

export interface EvalRunInput {
  readonly tasks: readonly Task[]
  readonly worker_results: readonly EvalWorkerResult[]
  readonly report?: EvalReportData
  readonly events?: readonly WorkerLifecycleEvent[]
}

export interface EvalFinding {
  readonly category: EvalCategory
  readonly code: EvalFindingCode
  readonly severity: EvalFindingSeverity
  readonly task_id?: string
  readonly message: string
  readonly evidence: readonly string[]
}

export interface EvalCategoryScore {
  readonly score: number
  readonly weight: number
  readonly status: EvalStatus
  readonly finding_count: number
}

export interface EvalCategoryScores {
  readonly boundary_compliance: EvalCategoryScore
  readonly verify_relevance: EvalCategoryScore
  readonly retries: EvalCategoryScore
  readonly no_op_rate: EvalCategoryScore
  readonly out_of_bounds_edits: EvalCategoryScore
  readonly lucky_pass_suspicion: EvalCategoryScore
  readonly result_completeness: EvalCategoryScore
}

export interface EvalRunSummary {
  readonly status: EvalStatus
  readonly task_count: number
  readonly worker_result_count: number
  readonly report_task_count: number
  readonly wave_count: number
  readonly completed_count: number
  readonly missing_worker_result_count: number
  readonly failed_verify_count: number
  readonly satisfied_verdict_count: number
  readonly retry_count: number
  readonly no_op_count: number
  readonly out_of_bounds_count: number
  readonly weak_verify_count: number
  readonly lucky_pass_suspicion_count: number
}

export interface EvalScorecard {
  readonly categories: EvalCategoryScores
  readonly total_score: number
  readonly status: EvalStatus
  readonly findings: readonly EvalFinding[]
  readonly summary: EvalRunSummary
}

interface TaskEvaluation {
  readonly findings: readonly EvalFinding[]
  readonly completed: boolean
  readonly missingWorkerResult: boolean
  readonly failedVerify: boolean
  readonly satisfiedVerdict: boolean
  readonly noOp: boolean
  readonly outOfBounds: boolean
  readonly weakVerify: boolean
  readonly luckyPassSuspicion: boolean
}

const CATEGORY_WEIGHTS: Readonly<Record<EvalCategory, number>> = {
  boundary_compliance: 20,
  verify_relevance: 20,
  retries: 15,
  no_op_rate: 15,
  out_of_bounds_edits: 15,
  lucky_pass_suspicion: 10,
  result_completeness: 5,
}

const CATEGORY_ORDER = [
  'boundary_compliance',
  'verify_relevance',
  'retries',
  'no_op_rate',
  'out_of_bounds_edits',
  'lucky_pass_suspicion',
  'result_completeness',
] as const satisfies readonly EvalCategory[]

const PASS_THRESHOLD = 95
const WARN_THRESHOLD = 75

type TaskEvaluationFlag =
  | 'completed'
  | 'missingWorkerResult'
  | 'failedVerify'
  | 'satisfiedVerdict'
  | 'noOp'
  | 'outOfBounds'
  | 'weakVerify'
  | 'luckyPassSuspicion'

export function scoreEvalRun(input: EvalRunInput): EvalScorecard {
  const workerResultsByTaskId = new Map(input.worker_results.map((result) => [result.task_id, result]))
  const evaluations = input.tasks.map((task) => evaluateTask(task, workerResultsByTaskId.get(task.id)))
  const retryCount = countRetries(input.events ?? [])
  const retryFindings = retryCount === 0 ? [] : [retryFinding(retryCount)]
  const taskFindings = evaluations.flatMap((evaluation) => evaluation.findings)
  const findings = [...taskFindings, ...retryFindings]
  const taskCount = input.tasks.length
  const categories = buildCategoryScores(evaluations, retryCount, taskCount, findings)
  const totalScore = totalCategoryScore(categories)
  const status = statusForScore(totalScore)

  return {
    categories,
    total_score: totalScore,
    status,
    findings,
    summary: summarizeRun(input, evaluations, retryCount, status),
  }
}

function evaluateTask(task: Task, result: EvalWorkerResult | undefined): TaskEvaluation {
  if (result === undefined) {
    return {
      findings: [
        finding(
          'result_completeness',
          'missing-worker-result',
          'critical',
          'Task has no normalized worker result.',
          [`task_id=${task.id}`],
          task.id,
        ),
      ],
      completed: false,
      missingWorkerResult: true,
      failedVerify: false,
      satisfiedVerdict: false,
      noOp: false,
      outOfBounds: false,
      weakVerify: false,
      luckyPassSuspicion: false,
    }
  }

  const changedFiles = result.files_changed ?? []
  const derivedOutOfBounds = findBoundaryDrift(task, changedFiles)
  const outOfBoundsFiles = uniqueStrings([...(result.out_of_bounds ?? []), ...derivedOutOfBounds])
  const verifyPassed = result.verify_rc === 0
  const verifyFailed = result.verify_rc !== undefined && result.verify_rc !== null && result.verify_rc !== 0
  const satisfiedVerdict = result.verdict?.satisfied === true
  const noOp = isNoOpResult(result, changedFiles)
  const weakVerify = verifyPassed && isWeakVerifyTask(task)
  const luckyNoOp = verifyPassed && satisfiedVerdict && noOp
  const satisfiedFailedVerify = verifyFailed && satisfiedVerdict
  const luckyPassSuspicion = luckyNoOp || satisfiedFailedVerify
  const findings = [
    ...boundaryFindings(task.id, derivedOutOfBounds),
    ...outOfBoundsFindings(task.id, outOfBoundsFiles),
    ...weakVerifyFindings(task, weakVerify),
    ...noOpFindings(task.id, result.status, noOp),
    ...luckyPassFindings(task.id, luckyNoOp, satisfiedFailedVerify, result.verify_rc),
  ]

  return {
    findings,
    completed: true,
    missingWorkerResult: false,
    failedVerify: verifyFailed,
    satisfiedVerdict,
    noOp,
    outOfBounds: outOfBoundsFiles.length > 0,
    weakVerify,
    luckyPassSuspicion,
  }
}

function buildCategoryScores(
  evaluations: readonly TaskEvaluation[],
  retryCount: number,
  taskCount: number,
  findings: readonly EvalFinding[],
): EvalCategoryScores {
  const boundaryCount = evaluations.filter((evaluation) =>
    hasFinding(evaluation, 'boundary_compliance'),
  ).length
  const resultCompletenessCount = evaluations.filter(
    (evaluation) => evaluation.missingWorkerResult,
  ).length

  return {
    boundary_compliance: categoryScore(
      'boundary_compliance',
      percentageScore(boundaryCount, taskCount, 100),
      findings,
    ),
    verify_relevance: categoryScore(
      'verify_relevance',
      percentageScore(countBy(evaluations, 'weakVerify'), taskCount, 60),
      findings,
    ),
    retries: categoryScore(
      'retries',
      percentageScore(retryCount, taskCount, 50),
      findings,
    ),
    no_op_rate: categoryScore(
      'no_op_rate',
      percentageScore(countBy(evaluations, 'noOp'), taskCount, 100),
      findings,
    ),
    out_of_bounds_edits: categoryScore(
      'out_of_bounds_edits',
      percentageScore(countBy(evaluations, 'outOfBounds'), taskCount, 100),
      findings,
    ),
    lucky_pass_suspicion: categoryScore(
      'lucky_pass_suspicion',
      percentageScore(countBy(evaluations, 'luckyPassSuspicion'), taskCount, 100),
      findings,
    ),
    result_completeness: categoryScore(
      'result_completeness',
      percentageScore(resultCompletenessCount, taskCount, 100),
      findings,
    ),
  }
}

function summarizeRun(
  input: EvalRunInput,
  evaluations: readonly TaskEvaluation[],
  retryCount: number,
  status: EvalStatus,
): EvalRunSummary {
  return {
    status,
    task_count: input.tasks.length,
    worker_result_count: input.worker_results.length,
    report_task_count: input.report?.task_reports?.length ?? 0,
    wave_count: input.report?.waves?.length ?? 0,
    completed_count: countBy(evaluations, 'completed'),
    missing_worker_result_count: countBy(evaluations, 'missingWorkerResult'),
    failed_verify_count: countBy(evaluations, 'failedVerify'),
    satisfied_verdict_count: countBy(evaluations, 'satisfiedVerdict'),
    retry_count: retryCount,
    no_op_count: countBy(evaluations, 'noOp'),
    out_of_bounds_count: countBy(evaluations, 'outOfBounds'),
    weak_verify_count: countBy(evaluations, 'weakVerify'),
    lucky_pass_suspicion_count: countBy(evaluations, 'luckyPassSuspicion'),
  }
}

function categoryScore(
  category: EvalCategory,
  score: number,
  findings: readonly EvalFinding[],
): EvalCategoryScore {
  return {
    score,
    weight: CATEGORY_WEIGHTS[category],
    status: statusForScore(score),
    finding_count: findings.filter((finding) => finding.category === category).length,
  }
}

function totalCategoryScore(categories: EvalCategoryScores): number {
  const weightedScore = CATEGORY_ORDER.reduce(
    (total, category) => total + categories[category].score * CATEGORY_WEIGHTS[category],
    0,
  )
  const totalWeight = CATEGORY_ORDER.reduce((total, category) => total + CATEGORY_WEIGHTS[category], 0)
  return Math.round(weightedScore / totalWeight)
}

function statusForScore(score: number): EvalStatus {
  if (score >= PASS_THRESHOLD) {
    return 'pass'
  }
  if (score >= WARN_THRESHOLD) {
    return 'warn'
  }
  return 'fail'
}

function percentageScore(count: number, taskCount: number, penaltyPerRatio: number): number {
  return Math.max(0, Math.round(100 - (count / Math.max(taskCount, 1)) * penaltyPerRatio))
}

function countBy(evaluations: readonly TaskEvaluation[], key: TaskEvaluationFlag): number {
  return evaluations.filter((evaluation) => evaluation[key]).length
}

function hasFinding(evaluation: TaskEvaluation, category: EvalCategory): boolean {
  return evaluation.findings.some((finding) => finding.category === category)
}

function countRetries(events: readonly WorkerLifecycleEvent[]): number {
  const restartedByTask = new Map<string, number>()
  const maxAttemptByTask = new Map<string, number>()

  for (const event of events) {
    const taskId = event.payload.task_id ?? event.payload.worker_id
    if (event.type === 'worker_restarted') {
      restartedByTask.set(taskId, (restartedByTask.get(taskId) ?? 0) + 1)
      maxAttemptByTask.set(taskId, Math.max(maxAttemptByTask.get(taskId) ?? 1, event.payload.attempt))
    }
    if (event.type === 'worker_started' && event.payload.attempt !== undefined) {
      maxAttemptByTask.set(taskId, Math.max(maxAttemptByTask.get(taskId) ?? 1, event.payload.attempt))
    }
  }

  return uniqueStrings([...restartedByTask.keys(), ...maxAttemptByTask.keys()])
    .map((taskId) =>
      Math.max(restartedByTask.get(taskId) ?? 0, (maxAttemptByTask.get(taskId) ?? 1) - 1),
    )
    .reduce((total, retries) => total + retries, 0)
}

function retryFinding(retryCount: number): EvalFinding {
  return finding(
    'retries',
    'retry-heavy-run',
    retryCount > 1 ? 'critical' : 'warning',
    `Worker lifecycle required ${String(retryCount)} retry attempt(s).`,
    [`retry_count=${String(retryCount)}`],
  )
}

function findBoundaryDrift(task: Task, changedFiles: readonly string[]): readonly string[] {
  const storyPath = `workers/${task.id}/story.md`
  return changedFiles.filter((file) => file !== storyPath && !isAllowedPath(file, task.paths))
}

function isAllowedPath(changedFile: string, allowedPaths: readonly string[]): boolean {
  return allowedPaths.some(
    (allowedPath) => changedFile === allowedPath || changedFile.startsWith(`${allowedPath}/`),
  )
}

function isWeakVerifyTask(task: Task): boolean {
  return (
    task.verify.trim().length === 0 ||
    (task.verify_proves ?? []).every((claim) => claim.trim().length === 0)
  )
}

function isNoOpResult(result: EvalWorkerResult, changedFiles: readonly string[]): boolean {
  const status = result.status.trim().toLowerCase()
  return status === 'no-op' || status === 'noop' || changedFiles.length === 0
}

function boundaryFindings(taskId: TaskId, files: readonly string[]): readonly EvalFinding[] {
  return files.length === 0
    ? []
    : [
        finding(
          'boundary_compliance',
          'boundary-drift',
          'critical',
          'Changed files were outside the task path boundary.',
          files,
          taskId,
        ),
      ]
}

function outOfBoundsFindings(taskId: TaskId, files: readonly string[]): readonly EvalFinding[] {
  return files.length === 0
    ? []
    : [
        finding(
          'out_of_bounds_edits',
          'out-of-bounds-edit',
          'critical',
          'Run data reported out-of-bounds edits.',
          files,
          taskId,
        ),
      ]
}

function weakVerifyFindings(task: Task, weakVerify: boolean): readonly EvalFinding[] {
  return weakVerify
    ? [
        finding(
          'verify_relevance',
          'weak-verify-pass',
          'warning',
          'Verify passed without task-level evidence describing what it proves.',
          [`verify=${task.verify}`],
          task.id,
        ),
      ]
    : []
}

function noOpFindings(taskId: TaskId, status: string, noOp: boolean): readonly EvalFinding[] {
  return noOp
    ? [
        finding(
          'no_op_rate',
          'no-op-worker',
          'warning',
          'Worker reported success without changing files.',
          [`status=${status}`],
          taskId,
        ),
      ]
    : []
}

function luckyPassFindings(
  taskId: TaskId,
  luckyNoOp: boolean,
  satisfiedFailedVerify: boolean,
  verifyRc: number | null | undefined,
): readonly EvalFinding[] {
  return [
    ...(luckyNoOp
      ? [
          finding(
            'lucky_pass_suspicion',
            'lucky-pass-no-op',
            'warning',
            'Verify and reviewer passed even though the worker changed no files.',
            ['verify_rc=0', 'verdict.satisfied=true'],
            taskId,
          ),
        ]
      : []),
    ...(satisfiedFailedVerify
      ? [
          finding(
            'lucky_pass_suspicion',
            'satisfied-verdict-failed-verify',
            'critical',
            'Verifier was satisfied even though the verify command failed.',
            [`verify_rc=${String(verifyRc)}`, 'verdict.satisfied=true'],
            taskId,
          ),
        ]
      : []),
  ]
}

function finding(
  category: EvalCategory,
  code: EvalFindingCode,
  severity: EvalFindingSeverity,
  message: string,
  evidence: readonly string[],
  taskId?: string,
): EvalFinding {
  return taskId === undefined
    ? { category, code, severity, message, evidence }
    : { category, code, severity, task_id: taskId, message, evidence }
}

function uniqueStrings(values: readonly string[]): readonly string[] {
  return [...new Set(values)]
}
