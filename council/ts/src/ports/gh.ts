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

/** One issue, named so gh never chooses the repository itself. */
export interface GhIssueLocator {
  readonly cwd: string
  /** `[HOST/]OWNER/REPO`, passed to gh as `--repo`, so gh never chooses the repository itself. */
  readonly repo: string
  readonly number: number
}

export interface GhIssueRequest extends GhIssueLocator {
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

/** An issue another issue links to, such as a blocker. */
export interface GhLinkedIssue {
  readonly number: number
  /** Its web URL, which names its repository. */
  readonly url: string
}

export interface GhPort {
  createPullRequest(request: GhPrRequest): Promise<GhPr>
  viewPullRequest(cwd: string, number: number): Promise<GhPr>
  /** Reads one issue; rejects when gh fails or prints something that is not an issue. */
  viewIssue(request: GhIssueRequest): Promise<GhIssue>
  /** Reads an issue's sub-issues, in GitHub's order, without their comments. */
  subIssues(request: GhIssueLocator): Promise<readonly GhIssue[]>
  /** Reads the issues GitHub's native dependencies say block this one. */
  blockedBy(request: GhIssueLocator): Promise<readonly GhLinkedIssue[]>
}
