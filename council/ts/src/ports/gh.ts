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

export interface GhPort {
  createPullRequest(request: GhPrRequest): Promise<GhPr>
  viewPullRequest(cwd: string, number: number): Promise<GhPr>
}
