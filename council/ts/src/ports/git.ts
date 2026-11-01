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

/**
 * Reading the index: the staged change the next commit would record. A code
 * review of work not yet committed reviews a commit made of the index, and a
 * commit gate tells one staged change from another by its entries.
 */
export interface GitIndexPort {
  /**
   * Every staged entry as `git ls-files --stage -z` prints it: mode, object
   * id, stage number and path, each entry ending in NUL. Reads the index and
   * writes nothing.
   */
  stagedEntries(cwd: string): Promise<string>
  /** The id of the tree a revision's commit holds; throws when the revision names no commit. */
  treeOf(cwd: string, revision: string): Promise<string>
  /** Writes the index's tree objects and returns the top tree's id; the staged entries, every ref and the working tree stay as they are. */
  writeIndexTree(cwd: string): Promise<string>
  /** Writes a commit object holding `tree`, with `parent` as its parent, and returns its id; no ref moves. */
  commitTree(cwd: string, tree: string, parent: string, message: string): Promise<string>
  /**
   * The unified diff from HEAD to the index, or to the working tree's
   * tracked files, in the form `diffCommits` prints. Takes no lock and
   * writes nothing, so a hook may call it before every commit.
   */
  diffAgainstHead(cwd: string, from: GitChangeSource): Promise<string>
  /** Every path that diff touches, a rename as both of its paths. */
  pathsAgainstHead(cwd: string, from: GitChangeSource): Promise<readonly string[]>
  /** Every file in the working tree that the index does not hold and no ignore rule hides. */
  untrackedPaths(cwd: string): Promise<readonly string[]>
}

/** What a change against HEAD is read from: the staged change, or the tracked files as they are on disk. */
export type GitChangeSource = 'index' | 'working-tree'

export interface GitDagExecutorPort extends GitPort {
  commitAll(cwd: string, request: GitCommitAllRequest): Promise<GitCommitAllResult>
  reconcileIntegrationBranch(cwd: string, request: GitReconcileRequest): Promise<GitReconcileResult>
}
