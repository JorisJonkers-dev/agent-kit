import type {
  ExistingGithubIssue,
  ExistingGithubLabel,
  ExistingGithubMilestone,
} from '../../../github/index.js'
import type { GhPr, ProcessPort } from '../../../../ports/index.js'
import { addComment as addGithubComment } from './comments.js'
import { GhCommandClient } from './gh-client.js'
import { listIssues as listGithubIssues, mirrorTaskIssue } from './issues.js'
import { bootstrapLabels, listVerifiedLabels as listGithubLabels } from './labels.js'
import {
  ensureMilestone as ensureGithubMilestone,
  listMilestones as listGithubMilestones,
} from './milestones.js'
import {
  createPullRequest as createGithubPullRequest,
  detectDefaultBranch as detectGithubDefaultBranch,
  viewPullRequest as viewGithubPullRequest,
} from './pull-requests.js'
import type {
  GithubAdapterContext,
  GithubCliAdapterOptions,
  GithubCommentRequest,
  GithubPullRequestMirrorRequest,
  GithubTaskIssueMirrorRequest,
  GithubTaskIssueMirrorResult,
  MethodOptions,
} from './types.js'

export class GithubCliAdapter {
  private readonly context: GithubAdapterContext

  constructor(process: ProcessPort, options: GithubCliAdapterOptions = {}) {
    this.context = {
      client: new GhCommandClient(process),
      dryRun: options.dryRun ?? false,
      githubBootstrap: options.githubBootstrap ?? false,
      log:
        options.log ??
        ((message) => {
          console.warn(message)
        }),
    }
  }

  async detectDefaultBranch(cwd: string): Promise<string | undefined> {
    return detectGithubDefaultBranch(this.context, cwd)
  }

  async listVerifiedLabels(cwd: string): Promise<readonly ExistingGithubLabel[]> {
    return listGithubLabels(this.context, cwd)
  }

  async bootstrapLabels(
    cwd: string,
    labels: readonly string[],
    options: MethodOptions = {},
  ): Promise<void> {
    return bootstrapLabels(this.context, cwd, labels, options)
  }

  async listMilestones(cwd: string): Promise<readonly ExistingGithubMilestone[]> {
    return listGithubMilestones(this.context, cwd)
  }

  async ensureMilestone(
    cwd: string,
    title: string,
    options: MethodOptions = {},
  ): Promise<ExistingGithubMilestone | undefined> {
    return ensureGithubMilestone(this.context, cwd, title, options)
  }

  async listIssues(cwd: string): Promise<readonly ExistingGithubIssue[]> {
    return listGithubIssues(this.context, cwd)
  }

  async mirrorTaskIssue(
    cwd: string,
    request: GithubTaskIssueMirrorRequest,
    options: MethodOptions = {},
  ): Promise<GithubTaskIssueMirrorResult> {
    return mirrorTaskIssue(this.context, cwd, request, options)
  }

  async createPullRequest(
    request: GithubPullRequestMirrorRequest,
    options: MethodOptions = {},
  ): Promise<GhPr | undefined> {
    return createGithubPullRequest(this.context, request, options)
  }

  async viewPullRequest(cwd: string, number: number): Promise<GhPr | undefined> {
    return viewGithubPullRequest(this.context, cwd, number)
  }

  async addComment(
    cwd: string,
    request: GithubCommentRequest,
    options: MethodOptions = {},
  ): Promise<void> {
    return addGithubComment(this.context, cwd, request, options)
  }
}
