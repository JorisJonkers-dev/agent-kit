import type {
  GitCommitAllRequest,
  GitCommitAllResult,
  GitDagExecutorPort,
  GitPorcelainEntry,
  GitReconcileRequest,
  GitReconcileResult,
  GitRemote,
  GitWorktree,
  ProcessCommand,
  ProcessPort,
  ProcessResult,
} from '../../../../ports/index.js'

export type {
  GitCommitAllRequest,
  GitCommitAllResult,
  GitReconcileRequest,
  GitReconcileResult,
} from '../../../../ports/index.js'

export type GitStatusCode = ' ' | '!' | '?' | 'A' | 'C' | 'D' | 'M' | 'R' | 'U'

export interface GitStatusEntry {
  readonly index: GitStatusCode
  readonly worktree: GitStatusCode
  readonly path: string
  readonly originalPath?: string
}

export interface GitDiffOptions {
  readonly base?: string
  readonly head?: string
  readonly staged?: boolean
  readonly paths?: readonly string[]
}

export interface GitPushRequest {
  readonly branch: string
  readonly remote?: string
  readonly setUpstream?: boolean
  readonly forceWithLease?: boolean
}

interface GitCommandOptions {
  readonly acceptedExitCodes?: readonly number[]
}

export class GitCommandError extends Error {
  readonly args: readonly string[]
  readonly cwd: string
  readonly result: ProcessResult

  constructor(args: readonly string[], cwd: string, result: ProcessResult) {
    const detail =
      result.stderr.trim() || result.stdout.trim() || `exit code ${String(result.exitCode)}`
    super(`git ${args.join(' ')} failed in ${cwd}: ${detail}`)
    this.name = 'GitCommandError'
    this.args = args
    this.cwd = cwd
    this.result = result
  }
}

export class GitCliAdapter implements GitDagExecutorPort {
  private integrationQueue: Promise<void> = Promise.resolve()
  private readonly process: ProcessPort

  constructor(process: ProcessPort) {
    this.process = process
  }

  async root(cwd: string): Promise<string> {
    const result = await this.git(cwd, ['rev-parse', '--show-toplevel'])
    return result.stdout.trim()
  }

  async commonDir(cwd: string): Promise<string> {
    const result = await this.git(cwd, ['rev-parse', '--path-format=absolute', '--git-common-dir'])
    return result.stdout.trim()
  }

  async remotes(cwd: string): Promise<readonly GitRemote[]> {
    // `git config --get-regexp` exits 1 when nothing matches.
    const result = await this.git(cwd, ['config', '--get-regexp', '^remote\\..*\\.url$'], {
      acceptedExitCodes: [0, 1],
    })
    return result.stdout
      .split('\n')
      .map((line) => /^remote\.(.+)\.url (.+)$/.exec(line.trim()))
      .flatMap((match) => (match === null ? [] : [{ name: match[1] ?? '', url: match[2] ?? '' }]))
  }

  async currentBranch(cwd: string): Promise<string> {
    const result = await this.git(cwd, ['branch', '--show-current'])
    return result.stdout.trim()
  }

  async createWorktree(cwd: string, branch: string, path: string): Promise<GitWorktree> {
    const branchRef = `refs/heads/${branch}`
    const existing = await this.git(cwd, ['show-ref', '--verify', '--quiet', branchRef], {
      acceptedExitCodes: [0, 1],
    })
    if (existing.exitCode === 0) {
      await this.git(cwd, ['worktree', 'add', path, branch])
      return { branch, path }
    }

    await this.git(cwd, ['worktree', 'add', '-b', branch, path, 'HEAD'])
    return { branch, path }
  }

  async removeWorktree(cwd: string, path: string): Promise<void> {
    await this.git(cwd, ['worktree', 'remove', '--force', path])
  }

  async changedFiles(cwd: string): Promise<readonly string[]> {
    const entries = await this.status(cwd)
    return entries.map((entry) => entry.path)
  }

  async status(cwd: string): Promise<readonly GitStatusEntry[]> {
    const result = await this.git(cwd, ['status', '--porcelain=v1'])
    const output = result.stdout.trimEnd()
    if (output.length === 0) {
      return []
    }

    return output.split('\n').map((line) => this.parseStatusLine(line))
  }

  async porcelainStatus(cwd: string): Promise<readonly GitPorcelainEntry[]> {
    const result = await this.git(cwd, ['status', '--porcelain=v1', '-z', '--untracked-files=all'])
    const fields = result.stdout.split('\0')
    const entries: GitPorcelainEntry[] = []
    for (let i = 0; i < fields.length; i += 1) {
      const field = fields[i] ?? ''
      if (field.length === 0) continue
      const code = field.slice(0, 2)
      entries.push({ code, path: field.slice(3) })
      // A rename or copy is followed by the path it came from.
      if (code.includes('R') || code.includes('C')) i += 1
    }
    return entries
  }

  async resolveCommit(cwd: string, revision: string): Promise<string> {
    const result = await this.git(cwd, ['rev-parse', '--verify', '--quiet', '--end-of-options', `${revision}^{commit}`])
    return result.stdout.trim()
  }

  async mergeBase(cwd: string, a: string, b: string): Promise<string> {
    const result = await this.git(cwd, ['merge-base', '--end-of-options', a, b])
    return result.stdout.trim()
  }

  async treeFiles(cwd: string, commit: string): Promise<readonly string[]> {
    const result = await this.git(cwd, ['ls-tree', '-r', '-z', '--full-tree', commit])
    // Each entry is `<mode> <type> <object>\t<path>`. A symbolic link is a
    // blob of mode 120000, and a submodule is a commit.
    return result.stdout.split('\0').flatMap((entry) => {
      const match = /^(\d+) (\w+) \w+\t(.+)$/s.exec(entry)
      return match !== null && match[2] === 'blob' && match[1] !== '120000' ? [match[3] ?? ''] : []
    })
  }

  async fileAt(cwd: string, commit: string, path: string): Promise<string> {
    const result = await this.git(cwd, ['cat-file', 'blob', `${commit}:${path}`])
    return result.stdout
  }

  async diffCommits(cwd: string, base: string, head: string): Promise<string> {
    const args = ['-c', 'core.quotePath=false', 'diff', '--no-ext-diff', '--no-textconv', '--no-color', base, head, '--']
    const result = await this.git(cwd, args)
    return result.stdout
  }

  async changedPaths(cwd: string, base: string, head: string): Promise<readonly string[]> {
    const result = await this.git(cwd, ['diff', '--name-only', '-z', '--no-renames', base, head, '--'])
    return result.stdout.split('\0').filter((path) => path.length > 0)
  }

  async diff(cwd: string, options: GitDiffOptions = {}): Promise<string> {
    const args = ['diff']
    if (options.staged === true) {
      args.push('--staged')
    }

    const range = this.diffRange(options)
    if (range !== undefined) {
      args.push(range)
    }

    if (options.paths !== undefined && options.paths.length > 0) {
      args.push('--', ...options.paths)
    }

    const result = await this.git(cwd, args)
    return result.stdout
  }

  async commitAll(cwd: string, request: GitCommitAllRequest): Promise<GitCommitAllResult> {
    const filesChanged = await this.changedFiles(cwd)
    await this.git(cwd, ['add', '-A'])

    const commitArgs = ['commit', '-m', request.message]
    if (request.author !== undefined) {
      commitArgs.push('--author', request.author)
    }
    if (request.allow_empty === true) {
      commitArgs.push('--allow-empty')
    }
    await this.git(cwd, commitArgs)

    const commit = await this.git(cwd, ['rev-parse', 'HEAD'])
    const branch = await this.currentBranch(cwd)
    return {
      branch,
      commit: commit.stdout.trim(),
      files_changed: filesChanged,
      message: request.message,
    }
  }

  async reconcileIntegrationBranch(
    cwd: string,
    request: GitReconcileRequest,
  ): Promise<GitReconcileResult> {
    const job = this.integrationQueue.then(() => this.reconcileIntegrationBranchNow(cwd, request))
    this.integrationQueue = job.then(
      () => undefined,
      () => undefined,
    )
    return job
  }

  async pushBranch(cwd: string, request: GitPushRequest): Promise<void> {
    const remote = request.remote ?? 'origin'
    const setUpstream = request.setUpstream ?? true
    const args = ['push']
    if (setUpstream) {
      args.push('--set-upstream')
    }
    if (request.forceWithLease === true) {
      args.push('--force-with-lease')
    }
    args.push(remote, request.branch)
    await this.git(cwd, args)
  }

  private async reconcileIntegrationBranchNow(
    cwd: string,
    request: GitReconcileRequest,
  ): Promise<GitReconcileResult> {
    await this.checkoutIntegrationBranch(cwd, request)
    const merge = await this.git(cwd, ['merge', '--no-ff', '--no-edit', request.sourceBranch], {
      acceptedExitCodes: [0, 1],
    })
    if (merge.exitCode !== 0) {
      await this.git(cwd, ['merge', '--abort'], { acceptedExitCodes: [0, 128] })
      throw new GitCommandError(['merge', '--no-ff', '--no-edit', request.sourceBranch], cwd, merge)
    }

    const head = await this.git(cwd, ['rev-parse', 'HEAD'])
    return {
      head: head.stdout.trim(),
      integrationBranch: request.integrationBranch,
      sourceBranch: request.sourceBranch,
    }
  }

  private async checkoutIntegrationBranch(cwd: string, request: GitReconcileRequest): Promise<void> {
    const branchRef = `refs/heads/${request.integrationBranch}`
    const existing = await this.git(cwd, ['show-ref', '--verify', '--quiet', branchRef], {
      acceptedExitCodes: [0, 1],
    })
    if (existing.exitCode === 0) {
      await this.git(cwd, ['checkout', request.integrationBranch])
      return
    }

    const baseBranch = request.baseBranch ?? 'HEAD'
    await this.git(cwd, ['checkout', '-B', request.integrationBranch, baseBranch])
  }

  private async git(
    cwd: string,
    args: readonly string[],
    options: GitCommandOptions = {},
  ): Promise<ProcessResult> {
    const command: ProcessCommand = {
      args,
      command: 'git',
      cwd,
    }
    const result = await this.process.exec(command)
    const acceptedExitCodes = options.acceptedExitCodes ?? [0]
    if (!acceptedExitCodes.includes(result.exitCode)) {
      throw new GitCommandError(args, cwd, result)
    }
    return result
  }

  private parseStatusLine(line: string): GitStatusEntry {
    const index = line.slice(0, 1) as GitStatusCode
    const worktree = line.slice(1, 2) as GitStatusCode
    const path = line.slice(3)
    const renameSeparator = ' -> '
    const renameAt = path.indexOf(renameSeparator)
    if (renameAt === -1) {
      return { index, path, worktree }
    }

    return {
      index,
      originalPath: path.slice(0, renameAt),
      path: path.slice(renameAt + renameSeparator.length),
      worktree,
    }
  }

  private diffRange(options: GitDiffOptions): string | undefined {
    if (options.base !== undefined && options.head !== undefined) {
      return `${options.base}..${options.head}`
    }
    return options.base ?? options.head
  }
}
