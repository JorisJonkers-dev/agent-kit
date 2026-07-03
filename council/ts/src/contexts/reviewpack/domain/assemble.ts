import type {
  Amendment,
  DesignLedgerEntry,
  ReviewVerdict,
  RoutingVerdict,
  Story,
  Task,
  TaskId,
} from '../../../domain/contracts/index.js'
import type {
  CheckpointOneInput,
  CheckpointOnePack,
  CheckpointTwoInput,
  CheckpointTwoPack,
  CheckpointTwoSummary,
  DagDepth,
  DesignCheckpointInput,
  DesignCheckpointPack,
  DesignVoteCount,
  DesignVoteRow,
  DiffStats,
  FileDiffStat,
  GrillLedgerEntry,
  LockedSpec,
  PruningProposal,
  SectionIndexEntry,
  SettledContestedLedger,
  SpendEstimate,
  StoryCard,
  TaskCard,
  TaskExecutionResult,
  TaskOutcomeRow,
} from './types.js'

const settledStatuses = new Set(['accepted', 'approved', 'locked', 'settled'])

export function assembleCheckpointOnePack(input: CheckpointOneInput): CheckpointOnePack {
  const dag_by_depth = buildDagByDepth(input.tasks)

  return {
    kind: 'checkpoint-1',
    routing_verdicts: normalizeRoutingVerdicts(input.routing_verdict),
    dag_by_depth,
    story_cards: (input.stories ?? []).map(toStoryCard),
    grill_ledger: (input.grill_verdicts ?? []).map(toGrillLedgerEntry),
    spend_estimate: estimateSpend(input.tasks.length, dag_by_depth.length, input.spend),
  }
}

export function assembleDesignCheckpointPack(
  input: DesignCheckpointInput,
): DesignCheckpointPack {
  const locked_spec = buildLockedSpec(input.locked_spec)

  return {
    kind: 'design-checkpoint',
    locked_spec,
    ledger: splitDesignLedger(input.ledger_entries),
    vote_table: {
      rows: buildVoteRows(input.votes),
      counts: buildVoteCounts(input.votes),
    },
    section_index: locked_spec.section_index,
  }
}

export function assembleCheckpointTwoPack(input: CheckpointTwoInput): CheckpointTwoPack {
  const task_outcomes = buildTaskOutcomes(input.tasks, input.task_results, input.waves)
  const summary = summarizeTaskOutcomes(task_outcomes)
  const no_verify_task_ids = task_outcomes
    .filter((row) => !row.has_verify)
    .map((row) => row.task_id)

  return {
    kind: 'checkpoint-2',
    run: input.run,
    integration_branch: input.integration_branch,
    integration_worktree: input.integration_worktree ?? null,
    pr: input.pr ?? null,
    summary,
    task_outcomes,
    diff_stats: buildDiffStats(input.task_results, input.file_diff_stats ?? []),
    discovered_work: sortAmendments(input.discovered_work ?? []),
    pruning_proposals: sortPruningProposals(input.pruning_proposals ?? []),
    no_verify_task_ids,
  }
}

export function buildDagByDepth(tasks: readonly Task[]): readonly DagDepth[] {
  const taskById = new Map<TaskId, Task>()
  for (const task of tasks) {
    if (taskById.has(task.id)) {
      throw new Error(`duplicate task id: ${task.id}`)
    }
    taskById.set(task.id, task)
  }

  for (const task of tasks) {
    for (const dependency of task.depends_on) {
      if (!taskById.has(dependency)) {
        throw new Error(`task ${task.id} depends on unknown task ${dependency}`)
      }
    }
  }

  const remaining = new Map(taskById)
  const done = new Set<TaskId>()
  const depths: DagDepth[] = []

  while (remaining.size > 0) {
    const ready = [...remaining.values()]
      .filter(
        (task) => task.depends_on.every((dependency) => done.has(dependency)),
      )
      .sort((left, right) => compareStrings(left.id, right.id))

    if (ready.length === 0) {
      throw new Error(
        `dependency cycle among tasks: ${[...remaining.keys()].sort(compareStrings).join(', ')}`,
      )
    }

    depths.push({
      depth: depths.length + 1,
      task_ids: ready.map((task) => task.id),
      tasks: ready.map(toTaskCard),
    })

    for (const task of ready) {
      done.add(task.id)
      remaining.delete(task.id)
    }
  }

  return depths
}

export function buildSectionIndex(markdown: string): readonly SectionIndexEntry[] {
  const entries: SectionIndexEntry[] = []
  const lines = markdown.split(/\r?\n/u)

  for (const [index, line] of lines.entries()) {
    const match = /^(#{1,6})\s+(.+?)\s*#*\s*$/u.exec(line)
    if (!isHeadingMatch(match)) {
      continue
    }

    const levelMarker = match[1]
    const title = match[2]

    entries.push({
      section_ref: sectionRef(entries.length + 1),
      level: levelMarker.length,
      title,
      anchor: slugifyHeading(title),
      line: index + 1,
    })
  }

  return entries
}

function isHeadingMatch(
  match: RegExpExecArray | null,
): match is RegExpExecArray & { readonly 1: string; readonly 2: string } {
  return match !== null
}

function sectionRef(index: number): `§${number}` {
  return `§${String(index)}` as `§${number}`
}

export function splitDesignLedger(
  entries: readonly DesignLedgerEntry[],
): SettledContestedLedger {
  const settled: DesignLedgerEntry[] = []
  const contested: DesignLedgerEntry[] = []

  for (const entry of entries) {
    const status = (entry.status ?? '').trim().toLowerCase()
    if (settledStatuses.has(status)) {
      settled.push(entry)
    } else {
      contested.push(entry)
    }
  }

  return {
    settled: sortLedgerEntries(settled),
    contested: sortLedgerEntries(contested),
  }
}

export function buildDiffStats(
  taskResults: readonly TaskExecutionResult[],
  fileDiffStats: readonly FileDiffStat[],
): DiffStats {
  const by_file =
    fileDiffStats.length > 0
      ? [...fileDiffStats].sort((left, right) => compareStrings(left.path, right.path))
      : deriveZeroDiffStats(taskResults)

  return {
    files_changed: by_file.length,
    additions: by_file.reduce((total, stat) => total + stat.additions, 0),
    deletions: by_file.reduce((total, stat) => total + stat.deletions, 0),
    by_file,
  }
}

function normalizeRoutingVerdicts(
  routingVerdict: RoutingVerdict | readonly RoutingVerdict[],
): readonly RoutingVerdict[] {
  return isRoutingVerdictArray(routingVerdict) ? [...routingVerdict] : [routingVerdict]
}

function isRoutingVerdictArray(
  routingVerdict: RoutingVerdict | readonly RoutingVerdict[],
): routingVerdict is readonly RoutingVerdict[] {
  return Array.isArray(routingVerdict)
}

function toTaskCard(task: Task): TaskCard {
  return {
    id: task.id,
    title: task.title,
    objective: task.objective,
    difficulty: task.difficulty,
    model: task.model,
    depends_on: [...task.depends_on],
    paths: [...task.paths],
    verify_present: task.verify.trim().length > 0,
    boundaries: task.boundaries,
    acceptance_criteria: [...(task.acceptance_criteria ?? [])],
  }
}

function toStoryCard(story: Story): StoryCard {
  return {
    title: story.title,
    status: story.status,
    goal: story.goal,
    actor: story.user_value.actor,
    capability: story.user_value.capability,
    outcome: story.user_value.outcome,
    acceptance_criteria: [...story.acceptance_criteria],
    in_scope: [...story.scope.in_scope],
    out_of_scope: [...story.scope.out_of_scope],
  }
}

function toGrillLedgerEntry(verdict: ReviewVerdict): GrillLedgerEntry {
  return {
    task_id: verdict.task_id ?? null,
    reviewer: verdict.reviewer ?? null,
    satisfied: verdict.satisfied,
    reasons: verdict.reasons,
    issues: [...verdict.issues],
  }
}

function estimateSpend(
  taskCount: number,
  waveCount: number,
  input: CheckpointOneInput['spend'],
): SpendEstimate {
  const rounds = input?.rounds ?? 2
  const verifierCallsPerTask = input?.fanout_verifier_calls_per_task ?? 1
  const planModelCalls = 2 + rounds * 4 + 1
  const fanoutWorkerCalls = taskCount
  const fanoutVerifierCalls = taskCount * verifierCallsPerTask

  return {
    plan_model_calls: planModelCalls,
    fanout_worker_calls: fanoutWorkerCalls,
    fanout_verifier_calls: fanoutVerifierCalls,
    total_model_calls: planModelCalls + fanoutWorkerCalls + fanoutVerifierCalls,
    task_count: taskCount,
    wave_count: waveCount,
  }
}

function buildLockedSpec(input: DesignCheckpointInput['locked_spec']): LockedSpec {
  return {
    title: input.title,
    markdown: input.markdown,
    spec_ref: input.spec_ref ?? null,
    locked_by: input.locked_by ?? null,
    section_index: buildSectionIndex(input.markdown),
  }
}

function buildVoteRows(votes: DesignCheckpointInput['votes']): readonly DesignVoteRow[] {
  return [...votes]
    .sort(
      (left, right) =>
        compareStrings(left.voter, right.voter) || compareStrings(left.option, right.option),
    )
    .map((vote) => ({
      voter: vote.voter,
      vote: vote.vote,
      option: vote.option,
      rationale: vote.rationale ?? null,
      conditions: [...(vote.conditions ?? [])],
    }))
}

function buildVoteCounts(votes: DesignCheckpointInput['votes']): readonly DesignVoteCount[] {
  const counts = new Map<string, number>()
  for (const vote of votes) {
    counts.set(vote.option, (counts.get(vote.option) ?? 0) + 1)
  }

  return [...counts.entries()]
    .sort((left, right) => right[1] - left[1] || compareStrings(left[0], right[0]))
    .map(([option, count]) => ({ option, count }))
}

function buildTaskOutcomes(
  tasks: readonly Task[],
  taskResults: readonly TaskExecutionResult[],
  waves: readonly (readonly string[])[],
): readonly TaskOutcomeRow[] {
  const taskById = new Map<string, Task>(tasks.map((task) => [task.id, task]))
  const order = new Map<string, number>()
  for (const [waveIndex, wave] of waves.entries()) {
    for (const [taskIndex, taskId] of wave.entries()) {
      order.set(taskId, waveIndex * tasks.length + taskIndex)
    }
  }

  return [...taskResults]
    .sort((left, right) => compareTaskResultOrder(left, right, order))
    .map((result) => toTaskOutcomeRow(result, taskById.get(result.task_id)))
}

function toTaskOutcomeRow(result: TaskExecutionResult, task: Task | undefined): TaskOutcomeRow {
  const merge = result.merge ?? null
  const status = result.status

  return {
    task_id: result.task_id,
    title: task?.title ?? result.task_id,
    status,
    merge,
    model: result.model ?? null,
    files_changed_count: (result.files_changed ?? []).length,
    verify_rc: result.verify_rc ?? null,
    verifier_satisfied: result.verdict?.satisfied ?? null,
    out_of_bounds: [...(result.out_of_bounds ?? [])],
    branch: result.branch ?? null,
    good: status === 'ok' && (merge === null || merge === 'ok'),
    has_verify: (task?.verify ?? '').trim().length > 0,
  }
}

function summarizeTaskOutcomes(rows: readonly TaskOutcomeRow[]): CheckpointTwoSummary {
  return {
    total: rows.length,
    ok: rows.filter((row) => row.status === 'ok').length,
    failed: rows.filter((row) => row.status !== 'ok').length,
    merged: rows.filter((row) => row.merge === 'ok').length,
    conflicts: rows.filter((row) => row.merge === 'conflict').length,
    no_verify: rows.filter((row) => !row.has_verify).length,
  }
}

function deriveZeroDiffStats(taskResults: readonly TaskExecutionResult[]): readonly FileDiffStat[] {
  const paths = new Set<string>()
  for (const result of taskResults) {
    for (const path of result.files_changed ?? []) {
      paths.add(path)
    }
  }

  return [...paths]
    .sort(compareStrings)
    .map((path) => ({
      path,
      additions: 0,
      deletions: 0,
    }))
}

function sortAmendments(amendments: readonly Amendment[]): readonly Amendment[] {
  return [...amendments].sort((left, right) => compareStrings(left.id, right.id))
}

function sortPruningProposals(
  proposals: readonly PruningProposal[],
): readonly PruningProposal[] {
  return [...proposals].sort((left, right) => compareStrings(left.id, right.id))
}

function sortLedgerEntries(entries: readonly DesignLedgerEntry[]): readonly DesignLedgerEntry[] {
  return [...entries].sort((left, right) => compareStrings(left.id, right.id))
}

function compareTaskResultOrder(
  left: TaskExecutionResult,
  right: TaskExecutionResult,
  order: ReadonlyMap<string, number>,
): number {
  return (
    (order.get(left.task_id) ?? Number.MAX_SAFE_INTEGER)
    - (order.get(right.task_id) ?? Number.MAX_SAFE_INTEGER)
    || compareStrings(left.task_id, right.task_id)
  )
}

function slugifyHeading(title: string): string {
  return title
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/gu, '-')
    .replace(/^-|-$/gu, '')
}

function compareStrings(left: string, right: string): number {
  return left.localeCompare(right)
}
