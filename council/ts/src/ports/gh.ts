export interface GhPrRequest {
  readonly cwd: string
  readonly title: string
  readonly body: string
  readonly base?: string
  readonly head?: string
  readonly draft?: boolean
}

export interface GhPr {
  readonly number: number
  readonly url: string
}

export interface GhIssueRequest {
  readonly cwd: string
  /** `[HOST/]OWNER/REPO`, passed to gh as `--repo`, so gh never chooses the repository itself. */
  readonly repo: string
  readonly number: number
  /** Whether the issue's comments are read as well. */
  readonly comments: boolean
}

export interface GhIssueComment {
  readonly author: string
  readonly createdAt: string
  readonly body: string
}

export interface GhIssue {
  readonly number: number
  readonly title: string
  readonly body: string
  readonly url: string
  readonly labels: readonly string[]
  /** Empty unless the request asked for comments. */
  readonly comments: readonly GhIssueComment[]
}

export interface GhPort {
  createPullRequest(request: GhPrRequest): Promise<GhPr>
  viewPullRequest(cwd: string, number: number): Promise<GhPr>
  /** Reads one issue; rejects when gh fails or prints something that is not an issue. */
  viewIssue(request: GhIssueRequest): Promise<GhIssue>
}
