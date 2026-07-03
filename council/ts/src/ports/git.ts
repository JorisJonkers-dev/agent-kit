export interface GitWorktree {
  readonly path: string
  readonly branch: string
}

export interface GitCommitAllRequest {
  readonly message: string
  readonly author?: string
  readonly allow_empty?: boolean
}

export interface GitCommitAllResult {
  readonly branch: string
  readonly commit: string
  readonly files_changed: readonly string[]
  readonly message: string
}

export interface GitReconcileRequest {
  readonly integrationBranch: string
  readonly sourceBranch: string
  readonly baseBranch?: string
}

export interface GitReconcileResult {
  readonly integrationBranch: string
  readonly sourceBranch: string
  readonly head: string
}

export interface GitPort {
  root(cwd: string): Promise<string>
  currentBranch(cwd: string): Promise<string>
  createWorktree(cwd: string, branch: string, path: string): Promise<GitWorktree>
  removeWorktree(cwd: string, path: string): Promise<void>
  changedFiles(cwd: string): Promise<readonly string[]>
  commitAll?(cwd: string, request: GitCommitAllRequest): Promise<GitCommitAllResult>
  reconcileIntegrationBranch?(
    cwd: string,
    request: GitReconcileRequest,
  ): Promise<GitReconcileResult>
}

export interface GitDagExecutorPort extends GitPort {
  commitAll(cwd: string, request: GitCommitAllRequest): Promise<GitCommitAllResult>
  reconcileIntegrationBranch(cwd: string, request: GitReconcileRequest): Promise<GitReconcileResult>
}
