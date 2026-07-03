export interface GitWorktree {
  readonly path: string
  readonly branch: string
}

export interface GitPort {
  root(cwd: string): Promise<string>
  currentBranch(cwd: string): Promise<string>
  createWorktree(cwd: string, branch: string, path: string): Promise<GitWorktree>
  removeWorktree(cwd: string, path: string): Promise<void>
  changedFiles(cwd: string): Promise<readonly string[]>
}
