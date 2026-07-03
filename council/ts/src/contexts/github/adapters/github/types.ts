import type { Story, Task } from '../../../../domain/contracts/index.js'
import type { ExistingGithubIssue } from '../../../../domain/github/index.js'
import type { GhCommandClient } from './gh-client.js'

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

export type GithubLog = (message: string, error: unknown) => void

export interface MethodOptions {
  readonly dryRun?: boolean
  readonly githubBootstrap?: boolean
}

export interface GithubAdapterContext {
  readonly client: GhCommandClient
  readonly dryRun: boolean
  readonly githubBootstrap: boolean
  readonly log: GithubLog
}
