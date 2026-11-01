import type { GitRemote } from '../shared-kernel/index.js'

export type { GitRemote }

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

/** One entry of `git status --porcelain`: its two-letter status code and the path it is about. */
export interface GitPorcelainEntry {
  readonly code: string
  /** The path relative to the working tree's top level; for a rename or copy, the new path. */
  readonly path: string
}

export interface GitPort {
  /** Absolute path of the working tree's top level. */
  root(cwd: string): Promise<string>
  /** Absolute path of the git common directory, shared by every worktree. */
  commonDir(cwd: string): Promise<string>
  /** Every remote with its URL; empty when the repository has none. */
  remotes(cwd: string): Promise<readonly GitRemote[]>
  currentBranch(cwd: string): Promise<string>
  createWorktree(cwd: string, branch: string, path: string): Promise<GitWorktree>
  removeWorktree(cwd: string, path: string): Promise<void>
  changedFiles(cwd: string): Promise<readonly string[]>
  /** The working tree's status, listing every untracked file rather than its directory. */
  porcelainStatus(cwd: string): Promise<readonly GitPorcelainEntry[]>
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
