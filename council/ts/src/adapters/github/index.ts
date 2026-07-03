import type { Story, Task } from '../../domain/contracts/index.js'
import {
  buildStatusLabelTransition,
  chooseMilestone,
  findIssueByTaskMarker,
  issueStateAfterMirror,
  renderPullRequestBody,
  renderTaskIssueBody,
  selectBestFitLabels,
} from '../../domain/github/index.js'
import type {
  ExistingGithubIssue,
  ExistingGithubLabel,
  ExistingGithubMilestone,
} from '../../domain/github/index.js'
import type { GhPr, ProcessCommand, ProcessPort, ProcessResult } from '../../ports/index.js'

export interface GithubCliAdapterOptions {
  readonly dryRun?: boolean
  readonly githubBootstrap?: boolean
  readonly log?: GithubLog
}

export interface GithubTaskIssueMirrorRequest {
  readonly runId: string
  readonly task: Task
  readonly story?: Story
  readonly edgeIssueNumbers?: ReadonlyMap<string, number>
  readonly specRef?: string
  readonly status?: string
  readonly landed?: boolean
  readonly milestoneTitle?: string
  readonly preferredLabels?: readonly string[]
}

export interface GithubTaskIssueMirrorResult {
  readonly marker: string
  readonly duplicates: readonly ExistingGithubIssue[]
  readonly number?: number
}

export interface GithubPullRequestMirrorRequest {
  readonly cwd: string
  readonly title: string
  readonly summary: string
  readonly closingIssueNumbers?: readonly number[]
  readonly referenceIssueNumbers?: readonly number[]
  readonly extraSections?: readonly string[]
  readonly base?: string
  readonly head?: string
  readonly draft?: boolean
}

export interface GithubCommentRequest {
  readonly kind: 'issue' | 'pr'
  readonly number: number
  readonly body: string
}

export class GithubCommandError extends Error {
  readonly args: readonly string[]
  readonly cwd: string
  readonly result: ProcessResult

  constructor(args: readonly string[], cwd: string, result: ProcessResult) {
    const detail =
      result.stderr.trim() || result.stdout.trim() || `exit code ${String(result.exitCode)}`
    super(`gh ${args.join(' ')} failed in ${cwd}: ${detail}`)
    this.name = 'GithubCommandError'
    this.args = args
    this.cwd = cwd
    this.result = result
  }
}

export type GithubLog = (message: string, error: unknown) => void

interface MethodOptions {
  readonly dryRun?: boolean
  readonly githubBootstrap?: boolean
}

export class GithubCliAdapter {
  private readonly dryRun: boolean
  private readonly githubBootstrap: boolean
  private readonly log: GithubLog
  private readonly process: ProcessPort

  constructor(process: ProcessPort, options: GithubCliAdapterOptions = {}) {
    this.process = process
    this.dryRun = options.dryRun ?? false
    this.githubBootstrap = options.githubBootstrap ?? false
    this.log =
      options.log ??
      ((message) => {
        console.warn(message)
      })
  }

  async detectDefaultBranch(cwd: string): Promise<string | undefined> {
    try {
      const result = await this.gh(cwd, [
        'repo',
        'view',
        '--json',
        'defaultBranchRef',
        '--jq',
        '.defaultBranchRef.name',
      ])
      return optionalTrimmed(result.stdout)
    } catch (error) {
      this.logFailure('detectDefaultBranch', error)
      return undefined
    }
  }

  async listVerifiedLabels(cwd: string): Promise<readonly ExistingGithubLabel[]> {
    try {
      const result = await this.gh(cwd, ['label', 'list', '--json', 'name', '--limit', '1000'])
      return parseJsonArray(result.stdout).filter(hasName)
    } catch (error) {
      this.logFailure('listVerifiedLabels', error)
      return []
    }
  }

  async bootstrapLabels(
    cwd: string,
    labels: readonly string[],
    options: MethodOptions = {},
  ): Promise<void> {
    try {
      if (!this.isBootstrapEnabled(options) || this.isDryRun(options)) {
        return
      }

      for (const label of labels) {
        await this.gh(cwd, ['label', 'create', label, '--force'])
      }
    } catch (error) {
      this.logFailure('bootstrapLabels', error)
    }
  }

  async listMilestones(cwd: string): Promise<readonly ExistingGithubMilestone[]> {
    try {
      const result = await this.gh(cwd, [
        'api',
        'repos/{owner}/{repo}/milestones',
        '--paginate',
        '-f',
        'state=all',
      ])
      return parseJsonArray(result.stdout).filter(hasMilestoneShape)
    } catch (error) {
      this.logFailure('listMilestones', error)
      return []
    }
  }

  async ensureMilestone(
    cwd: string,
    title: string,
    options: MethodOptions = {},
  ): Promise<ExistingGithubMilestone | undefined> {
    try {
      const decision = chooseMilestone(title, await this.listMilestones(cwd))
      if (decision.kind === 'reuse') {
        return decision.milestone
      }
      if (this.isDryRun(options)) {
        return undefined
      }

      const result = await this.gh(cwd, [
        'api',
        'repos/{owner}/{repo}/milestones',
        '-f',
        `title=${title}`,
      ])
      return parseJsonObject<ExistingGithubMilestone>(result.stdout, hasMilestoneShape)
    } catch (error) {
      this.logFailure('ensureMilestone', error)
      return undefined
    }
  }

  async listIssues(cwd: string): Promise<readonly ExistingGithubIssue[]> {
    try {
      const result = await this.gh(cwd, [
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
      this.logFailure('listIssues', error)
      return []
    }
  }

  async mirrorTaskIssue(
    cwd: string,
    request: GithubTaskIssueMirrorRequest,
    options: MethodOptions = {},
  ): Promise<GithubTaskIssueMirrorResult> {
    try {
      const issues = await this.listIssues(cwd)
      const labels = await this.listVerifiedLabels(cwd)
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
        ? await this.ensureMilestone(cwd, request.milestoneTitle, options)
        : undefined

      if (this.isDryRun(options)) {
        return {
          duplicates: match.duplicates,
          marker: match.marker,
          ...(match.issue ? { number: match.issue.number } : {}),
        }
      }

      const number = match.issue
        ? await this.updateTaskIssue(cwd, match.issue, request, body, selectedLabels, milestone)
        : await this.createTaskIssue(cwd, request, body, selectedLabels, milestone)

      return {
        duplicates: match.duplicates,
        marker: match.marker,
        ...(number ? { number } : {}),
      }
    } catch (error) {
      this.logFailure('mirrorTaskIssue', error)
      return {
        duplicates: [],
        marker: '',
      }
    }
  }

  async createPullRequest(
    request: GithubPullRequestMirrorRequest,
    options: MethodOptions = {},
  ): Promise<GhPr | undefined> {
    try {
      if (this.isDryRun(options)) {
        return undefined
      }

      const body = renderPullRequestBody({
        summary: request.summary,
        ...(request.closingIssueNumbers
          ? { closingIssueNumbers: request.closingIssueNumbers }
          : {}),
        ...(request.referenceIssueNumbers
          ? { referenceIssueNumbers: request.referenceIssueNumbers }
          : {}),
        ...(request.extraSections ? { extraSections: request.extraSections } : {}),
      })
      const base = request.base ?? (await this.detectDefaultBranch(request.cwd))
      const args = [
        'pr',
        'create',
        '--title',
        request.title,
        '--body',
        body,
        '--json',
        'number,url',
      ]
      if (base) {
        args.push('--base', base)
      }
      if (request.head) {
        args.push('--head', request.head)
      }
      if (request.draft === true) {
        args.push('--draft')
      }

      return parseJsonObject<GhPr>(await this.ghJson(request.cwd, args), hasPullRequestShape)
    } catch (error) {
      this.logFailure('createPullRequest', error)
      return undefined
    }
  }

  async viewPullRequest(cwd: string, number: number): Promise<GhPr | undefined> {
    try {
      return parseJsonObject<GhPr>(
        await this.ghJson(cwd, ['pr', 'view', String(number), '--json', 'number,url']),
        hasPullRequestShape,
      )
    } catch (error) {
      this.logFailure('viewPullRequest', error)
      return undefined
    }
  }

  async addComment(
    cwd: string,
    request: GithubCommentRequest,
    options: MethodOptions = {},
  ): Promise<void> {
    try {
      if (this.isDryRun(options)) {
        return
      }

      await this.gh(cwd, [
        request.kind,
        'comment',
        String(request.number),
        '--body',
        request.body,
      ])
    } catch (error) {
      this.logFailure('addComment', error)
    }
  }

  private async createTaskIssue(
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

    const result = await this.gh(cwd, args)
    return issueNumberFromOutput(result.stdout)
  }

  private async updateTaskIssue(
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

    await this.gh(cwd, args)
    return issue.number
  }

  private async ghJson(cwd: string, args: readonly string[]): Promise<string> {
    const result = await this.gh(cwd, args)
    return result.stdout
  }

  private async gh(cwd: string, args: readonly string[]): Promise<ProcessResult> {
    const command: ProcessCommand = {
      args,
      command: 'gh',
      cwd,
    }
    const result = await this.process.exec(command)
    if (result.exitCode !== 0) {
      throw new GithubCommandError(args, cwd, result)
    }
    return result
  }

  private isDryRun(options: MethodOptions): boolean {
    return options.dryRun ?? this.dryRun
  }

  private isBootstrapEnabled(options: MethodOptions): boolean {
    return options.githubBootstrap ?? this.githubBootstrap
  }

  private logFailure(method: string, error: unknown): void {
    this.log(`github ${method} failed`, error)
  }
}

function addJoinedOption(args: string[], option: string, values: readonly string[]): void {
  if (values.length > 0) {
    args.push(option, values.join(','))
  }
}

function optionalTrimmed(value: string): string | undefined {
  const trimmed = value.trim()
  return trimmed.length > 0 ? trimmed : undefined
}

function parseJsonArray(value: string): readonly unknown[] {
  const parsed: unknown = JSON.parse(value)
  return Array.isArray(parsed) ? parsed : []
}

function parseJsonObject<T>(
  value: string,
  guard: (candidate: unknown) => candidate is T,
): T | undefined {
  const parsed: unknown = JSON.parse(value)
  return guard(parsed) ? parsed : undefined
}

function hasName(candidate: unknown): candidate is ExistingGithubLabel {
  return (
    candidate !== null &&
    typeof candidate === 'object' &&
    typeof (candidate as Partial<ExistingGithubLabel>).name === 'string'
  )
}

function hasMilestoneShape(candidate: unknown): candidate is ExistingGithubMilestone {
  return (
    candidate !== null &&
    typeof candidate === 'object' &&
    typeof (candidate as Partial<ExistingGithubMilestone>).number === 'number' &&
    typeof (candidate as Partial<ExistingGithubMilestone>).title === 'string'
  )
}

function hasIssueShape(candidate: unknown): candidate is ExistingGithubIssue {
  return (
    candidate !== null &&
    typeof candidate === 'object' &&
    typeof (candidate as Partial<ExistingGithubIssue>).number === 'number' &&
    typeof (candidate as Partial<ExistingGithubIssue>).title === 'string'
  )
}

function hasPullRequestShape(candidate: unknown): candidate is GhPr {
  if (!candidate || typeof candidate !== 'object') {
    return false
  }

  const pullRequest = candidate as Partial<GhPr>
  return typeof pullRequest.number === 'number' && typeof pullRequest.url === 'string'
}

function existingLabelName(label: ExistingGithubLabel | string): string {
  return typeof label === 'string' ? label : label.name
}

function uniqueStrings(values: readonly string[]): readonly string[] {
  return [...new Set(values)]
}

function issueNumberFromOutput(output: string): number | undefined {
  const match = /\/issues\/(?<number>\d+)/u.exec(output)
  return match?.groups?.number ? Number(match.groups.number) : undefined
}
