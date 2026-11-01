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

/**
 * Reading commits rather than the working tree: what a code review freezes.
 * Every method takes a commit, never the working tree or the index.
 */
export interface GitRevisionPort {
  /** The full id of the commit a revision names; throws when it names none. */
  resolveCommit(cwd: string, revision: string): Promise<string>
  /** The best common ancestor of two commits. */
  mergeBase(cwd: string, a: string, b: string): Promise<string>
  /** The path of every regular file in the commit's tree; symbolic links and submodules are not files. */
  treeFiles(cwd: string, commit: string): Promise<readonly string[]>
  /** One file's contents at a commit. */
  fileAt(cwd: string, commit: string, path: string): Promise<string>
  /** The unified diff from one commit to another, with no external diff driver or text conversion. */
  diffCommits(cwd: string, base: string, head: string): Promise<string>
  /** Every path the diff from one commit to another adds, changes or deletes, a rename as both of its paths. */
  changedPaths(cwd: string, base: string, head: string): Promise<readonly string[]>
}

export interface GitDagExecutorPort extends GitPort {
  commitAll(cwd: string, request: GitCommitAllRequest): Promise<GitCommitAllResult>
  reconcileIntegrationBranch(cwd: string, request: GitReconcileRequest): Promise<GitReconcileResult>
}
