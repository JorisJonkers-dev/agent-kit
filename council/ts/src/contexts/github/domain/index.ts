import type { Story, Task } from '../../../domain/contracts/index.js'

export interface ExistingGithubLabel {
  readonly name: string
}

export interface ExistingGithubMilestone {
  readonly number: number
  readonly title: string
}

export interface ExistingGithubIssue {
  readonly number: number
  readonly title: string
  readonly body?: string | null
  readonly labels?: readonly (ExistingGithubLabel | string)[]
  readonly state?: 'OPEN' | 'CLOSED' | 'open' | 'closed'
}

export interface MilestoneDecision {
  readonly kind: 'reuse' | 'create'
  readonly title: string
  readonly milestone?: ExistingGithubMilestone
  readonly create?: {
    readonly title: string
  }
}

export interface TaskIssueMatch {
  readonly marker: string
  readonly issue?: ExistingGithubIssue
  readonly duplicates: readonly ExistingGithubIssue[]
}

export interface LabelSelectionInput {
  readonly existingLabels: readonly (ExistingGithubLabel | string)[]
  readonly status?: string
  readonly preferred?: readonly string[]
  readonly task?: Pick<
    Task,
    'archetype' | 'context_profile' | 'difficulty' | 'model' | 'paths'
  >
}

export interface StatusLabelTransition {
  readonly add: readonly string[]
  readonly remove: readonly string[]
  readonly labels: readonly string[]
}

export interface TaskIssueBodyInput {
  readonly runId: string
  readonly task: Task
  readonly story?: Story
  readonly edgeIssueNumbers?: ReadonlyMap<string, number>
  readonly specRef?: string
}

export interface PullRequestBodyInput {
  readonly summary: string
  readonly closingIssueNumbers?: readonly number[]
  readonly referenceIssueNumbers?: readonly number[]
  readonly extraSections?: readonly string[]
}

export function chooseMilestone(
  title: string,
  milestones: readonly ExistingGithubMilestone[],
): MilestoneDecision {
  const milestone = milestones.find((candidate) => candidate.title === title)

  if (milestone) {
    return { kind: 'reuse', title, milestone }
  }

  return { kind: 'create', title, create: { title } }
}

export function taskMarker(runId: string, taskId: string): string {
  return `<!-- council-task-id: ${runId}/${taskId} -->`
}

export function findIssueByTaskMarker(
  runId: string,
  taskId: string,
  issues: readonly ExistingGithubIssue[],
): TaskIssueMatch {
  const marker = taskMarker(runId, taskId)
  const matches = issues.filter((issue) => issue.body?.includes(marker) ?? false)
  const [issue, ...duplicates] = matches

  return issue ? { marker, issue, duplicates } : { marker, duplicates }
}

export function renderTaskIssueBody(input: TaskIssueBodyInput): string {
  const specRef = input.specRef ?? input.task.spec_ref
  const sections = [
    taskMarker(input.runId, input.task.id),
    `# ${input.task.id}: ${input.task.title}`,
    renderTaskSummary(input.task),
    renderStory(input.story),
    renderEdges(input.task, input.edgeIssueNumbers),
    specRef ? `## Spec\n\nspec_ref: ${specRef}` : '',
  ].filter((section) => section.length > 0)

  return `${sections.join('\n\n')}\n`
}

export function selectBestFitLabels(input: LabelSelectionInput): readonly string[] {
  const candidates = [
    ...(input.preferred ?? []),
    ...statusLabelCandidates(input.status),
    ...taskLabelCandidates(input.task),
  ]

  return unique(
    candidates.flatMap((candidate) => existingLabel(candidate, input.existingLabels)),
  )
}

export function buildStatusLabelTransition(
  currentLabels: readonly (ExistingGithubLabel | string)[],
  existingLabels: readonly (ExistingGithubLabel | string)[],
  nextStatus: string,
): StatusLabelTransition {
  const currentStatusLabels = currentLabels
    .map(labelName)
    .filter((name) => name.startsWith('council/status:'))
  const nextStatusLabel = findExistingLabel(
    `council/status:${slug(nextStatus)}`,
    existingLabels,
  )
  const remove = currentStatusLabels.filter((name) => name !== nextStatusLabel)
  const add =
    nextStatusLabel && !currentStatusLabels.includes(nextStatusLabel)
      ? [nextStatusLabel]
      : []
  const labels = unique([
    ...currentLabels.map(labelName).filter((name) => !remove.includes(name)),
    ...add,
  ])

  return { add, remove, labels }
}

export function issueStateAfterMirror(status: string, landed: boolean): 'open' | 'closed' {
  return landed && slug(status) === 'landed' ? 'closed' : 'open'
}

export function renderPullRequestBody(input: PullRequestBodyInput): string {
  const closingIssueNumbers = uniqueNumbers(input.closingIssueNumbers ?? [])
  const referenceIssueNumbers = uniqueNumbers(input.referenceIssueNumbers ?? []).filter(
    (number) => !closingIssueNumbers.includes(number),
  )
  const issueLines = [
    ...closingIssueNumbers.map((number) => `Closes #${String(number)}`),
    ...referenceIssueNumbers.map((number) => `refs #${String(number)}`),
  ]
  const sections = [
    input.summary.trim(),
    issueLines.join('\n'),
    ...(input.extraSections ?? []).map((section) => section.trim()),
  ].filter((section) => section.length > 0)

  return `${sections.join('\n\n')}\n`
}

function renderTaskSummary(task: Task): string {
  const lines = [
    '## Task',
    '',
    `Objective: ${task.objective}`,
    `Output: ${task.output_format}`,
    `Paths: ${task.paths.join(', ') || 'none'}`,
    `Verify: ${task.verify}`,
    `Boundaries: ${task.boundaries}`,
    ...renderList('Acceptance criteria', task.acceptance_criteria),
  ]

  if (task.dev_notes) {
    lines.push(`Dev notes: ${task.dev_notes}`)
  }

  return lines.join('\n')
}

function renderStory(story: Story | undefined): string {
  if (!story) {
    return ''
  }

  return [
    '## Story',
    '',
    `Status: ${story.status}`,
    `Goal: ${story.goal}`,
    [
      `User value: As ${story.user_value.actor},`,
      `${story.user_value.capability},`,
      `so ${story.user_value.outcome}.`,
    ].join(' '),
    `Context: ${story.context}`,
    ...renderList('Story acceptance criteria', story.acceptance_criteria),
    ...renderList('In scope', story.scope.in_scope),
    ...renderList('Out of scope', story.scope.out_of_scope),
    ...renderList('Implementation files', story.implementation_notes.files),
    ...renderList('Implementation patterns', story.implementation_notes.patterns),
    ...renderList('Dependencies', story.implementation_notes.dependencies),
    ...renderList(
      'Data/config/migration',
      story.implementation_notes.data_config_migration,
    ),
    ...renderList('Unit tests', story.tests.unit),
    ...renderList('Integration tests', story.tests.integration),
    ...renderList('Manual/workflow tests', story.tests.manual_or_workflow),
    ...renderList('Definition of done', story.definition_of_done),
  ].join('\n')
}

function renderEdges(
  task: Task,
  issueNumbers: ReadonlyMap<string, number> | undefined,
): string {
  const lines = [
    ...task.depends_on.flatMap((taskId) =>
      renderEdge('Blocked by', taskId, issueNumbers),
    ),
    ...renderOptionalEdge('discovered-from', task.discovered_from, issueNumbers),
    ...(task.supersedes ?? []).flatMap((taskId) =>
      renderEdge('supersedes', taskId, issueNumbers),
    ),
  ]

  return lines.length > 0 ? ['## Edges', '', ...lines].join('\n') : ''
}

function renderOptionalEdge(
  label: string,
  taskId: string | undefined,
  issueNumbers: ReadonlyMap<string, number> | undefined,
): readonly string[] {
  return taskId ? renderEdge(label, taskId, issueNumbers) : []
}

function renderEdge(
  label: string,
  taskId: string,
  issueNumbers: ReadonlyMap<string, number> | undefined,
): readonly string[] {
  const issueNumber = issueNumbers?.get(taskId)
  return [`${label} ${issueNumber ? `#${String(issueNumber)}` : taskId}`]
}

function renderList(label: string, values: readonly string[] | undefined): readonly string[] {
  if (!values || values.length === 0) {
    return []
  }

  return ['', `${label}:`, ...values.map((value) => `- ${value}`)]
}

function statusLabelCandidates(status: string | undefined): readonly string[] {
  return status ? [`council/status:${slug(status)}`] : []
}

function taskLabelCandidates(
  task: LabelSelectionInput['task'] | undefined,
): readonly string[] {
  if (!task) {
    return []
  }

  const pathSegments = task.paths.flatMap((path) => {
    const segment = path.split('/').find((part) => part.length > 0)
    return segment ? [`area:${slug(segment)}`, `path:${slug(segment)}`] : []
  })

  return [
    task.difficulty,
    `difficulty:${task.difficulty}`,
    `model:${task.model}`,
    ...optionalLabelPair('archetype', task.archetype),
    ...optionalLabelPair('context', task.context_profile),
    ...pathSegments,
  ]
}

function optionalLabelPair(
  prefix: string,
  value: string | undefined,
): readonly string[] {
  return value
    ? [`${prefix}:${slug(value)}`, `council/${prefix}:${slug(value)}`]
    : []
}

function existingLabel(
  candidate: string,
  existingLabels: readonly (ExistingGithubLabel | string)[],
): readonly string[] {
  const label = findExistingLabel(candidate, existingLabels)

  return label ? [label] : []
}

function findExistingLabel(
  candidate: string,
  existingLabels: readonly (ExistingGithubLabel | string)[],
): string | undefined {
  const normalizedCandidate = candidate.toLowerCase()
  const label = existingLabels.find(
    (existing) => labelName(existing).toLowerCase() === normalizedCandidate,
  )

  return label ? labelName(label) : undefined
}

function labelName(label: ExistingGithubLabel | string): string {
  return typeof label === 'string' ? label : label.name
}

function slug(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
}

function unique(values: readonly string[]): readonly string[] {
  return [...new Set(values)]
}

function uniqueNumbers(values: readonly number[]): readonly number[] {
  return [...new Set(values)]
}
