import {
  buildStatusLabelTransition,
  findIssueByTaskMarker,
  issueStateAfterMirror,
  renderTaskIssueBody,
  selectBestFitLabels,
} from '../../../github/index.js'
import type {
  ExistingGithubIssue,
  ExistingGithubMilestone,
} from '../../../github/index.js'
import {
  addJoinedOption,
  existingLabelName,
  isDryRun,
  logFailure,
  parseJsonArray,
  uniqueStrings,
} from './helpers.js'
import { listVerifiedLabels } from './labels.js'
import { ensureMilestone } from './milestones.js'
import type {
  GithubAdapterContext,
  GithubTaskIssueMirrorRequest,
  GithubTaskIssueMirrorResult,
  MethodOptions,
} from './types.js'

export async function listIssues(
  context: GithubAdapterContext,
  cwd: string,
): Promise<readonly ExistingGithubIssue[]> {
  try {
    const result = await context.client.gh(cwd, [
      'issue',
      'list',
      '--state',
      'all',
      '--json',
      'number,title,body,labels,state',
      '--limit',
      '1000',
    ])
    return parseJsonArray(result.stdout).filter(hasIssueShape)
  } catch (error) {
    logFailure(context, 'listIssues', error)
    return []
  }
}

export async function mirrorTaskIssue(
  context: GithubAdapterContext,
  cwd: string,
  request: GithubTaskIssueMirrorRequest,
  options: MethodOptions = {},
): Promise<GithubTaskIssueMirrorResult> {
  try {
    const issues = await listIssues(context, cwd)
    const labels = await listVerifiedLabels(context, cwd)
    const match = findIssueByTaskMarker(request.runId, request.task.id, issues)
    const body = renderTaskIssueBody({
      runId: request.runId,
      task: request.task,
      ...(request.story ? { story: request.story } : {}),
      ...(request.edgeIssueNumbers ? { edgeIssueNumbers: request.edgeIssueNumbers } : {}),
      ...(request.specRef ? { specRef: request.specRef } : {}),
    })
    const selectedLabels = selectBestFitLabels({
      existingLabels: labels,
      ...(request.status ? { status: request.status } : {}),
      ...(request.preferredLabels ? { preferred: request.preferredLabels } : {}),
      task: request.task,
    })
    const milestone = request.milestoneTitle
      ? await ensureMilestone(context, cwd, request.milestoneTitle, options)
      : undefined

    if (isDryRun(context, options)) {
      return {
        duplicates: match.duplicates,
        marker: match.marker,
        ...(match.issue ? { number: match.issue.number } : {}),
      }
    }

    const number = match.issue
      ? await updateTaskIssue(context, cwd, match.issue, request, body, selectedLabels, milestone)
      : await createTaskIssue(context, cwd, request, body, selectedLabels, milestone)

    return {
      duplicates: match.duplicates,
      marker: match.marker,
      ...(number ? { number } : {}),
    }
  } catch (error) {
    logFailure(context, 'mirrorTaskIssue', error)
    return {
      duplicates: [],
      marker: '',
    }
  }
}

async function createTaskIssue(
  context: GithubAdapterContext,
  cwd: string,
  request: GithubTaskIssueMirrorRequest,
  body: string,
  labels: readonly string[],
  milestone: ExistingGithubMilestone | undefined,
): Promise<number | undefined> {
  const args = ['issue', 'create', '--title', request.task.title, '--body', body]
  addJoinedOption(args, '--label', labels)
  if (milestone) {
    args.push('--milestone', milestone.title)
  }

  const result = await context.client.gh(cwd, args)
  return issueNumberFromOutput(result.stdout)
}

async function updateTaskIssue(
  context: GithubAdapterContext,
  cwd: string,
  issue: ExistingGithubIssue,
  request: GithubTaskIssueMirrorRequest,
  body: string,
  labels: readonly string[],
  milestone: ExistingGithubMilestone | undefined,
): Promise<number> {
  const args = [
    'issue',
    'edit',
    String(issue.number),
    '--title',
    request.task.title,
    '--body',
    body,
  ]
  const statusTransition = request.status
    ? buildStatusLabelTransition(issue.labels ?? [], labels, request.status)
    : { add: labels, remove: [] }
  const currentLabels = new Set((issue.labels ?? []).map(existingLabelName))
  const addedLabels = uniqueStrings([
    ...labels.filter((label) => !currentLabels.has(label)),
    ...statusTransition.add,
  ])

  addJoinedOption(args, '--add-label', addedLabels)
  addJoinedOption(args, '--remove-label', statusTransition.remove)
  if (request.status) {
    args.push('--state', issueStateAfterMirror(request.status, request.landed ?? false))
  }
  if (milestone) {
    args.push('--milestone', milestone.title)
  }

  await context.client.gh(cwd, args)
  return issue.number
}

function hasIssueShape(candidate: unknown): candidate is ExistingGithubIssue {
  return (
    candidate !== null &&
    typeof candidate === 'object' &&
    typeof (candidate as Partial<ExistingGithubIssue>).number === 'number' &&
    typeof (candidate as Partial<ExistingGithubIssue>).title === 'string'
  )
}

function issueNumberFromOutput(output: string): number | undefined {
  const match = /\/issues\/(?<number>\d+)/u.exec(output)
  return match?.groups?.number ? Number(match.groups.number) : undefined
}
