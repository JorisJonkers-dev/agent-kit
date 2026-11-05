import { execFile } from 'node:child_process'
import { rm } from 'node:fs/promises'

import type { GitRemote } from '../../../../shared-kernel/index.js'
import type { ModelGatewayGitPort } from '../../application/ports.js'

const OUTPUT_LIMIT = 1024 * 1024

/** How long git may take before the gateway gives up on it. */
const TIMEOUT_MS = 10_000

/** How long a worktree may take to check out or remove: far longer than a lookup, as a large repository's files are written. */
const WORKTREE_TIMEOUT_MS = 120_000

/** git's answer for a path that is in no repository; every other failure is an error. */
const NOT_A_REPOSITORY = /not a git repository/i

interface GitResult {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
}

/**
 * Finds a path's repository with git: its root and its remotes, and adds and
 * removes the throwaway worktrees of it. It starts no client, only git. A path
 * in no repository has no root; a path git cannot read at all, or does not
 * answer for in time, is an error rather than a repository with nothing to say.
 */
export class NodeModelGatewayGit implements ModelGatewayGitPort {
  constructor(
    private readonly path: string = process.env.PATH ?? '',
    private readonly timeoutMs: number = TIMEOUT_MS,
  ) {}

  async root(path: string): Promise<string | undefined> {
    const result = await this.git(path, ['rev-parse', '--show-toplevel'])
    if (result.exitCode === 0) {
      const root = result.stdout.trim()
      if (root === '') throw new Error('git answered with no repository root')
      return root
    }
    if (NOT_A_REPOSITORY.test(result.stderr)) return undefined
    throw new Error(messageOf(result))
  }

  async commonDir(path: string): Promise<string> {
    const result = await this.git(path, ['rev-parse', '--path-format=absolute', '--git-common-dir'])
    const commonDir = result.stdout.trim()
    if (result.exitCode !== 0 || commonDir === '') throw new Error(messageOf(result))
    return commonDir
  }

  async remotes(path: string): Promise<readonly GitRemote[]> {
    const result = await this.git(path, ['config', '--get-regexp', String.raw`^remote\..*\.url$`])
    if (result.exitCode === 1) return []
    if (result.exitCode !== 0) throw new Error(messageOf(result))
    return result.stdout.split('\n').flatMap((line) => {
      const match = /^remote\.(.+)\.url\s+(.+)$/.exec(line.trim())
      return match === null ? [] : [{ name: String(match[1]), url: String(match[2]) }]
    })
  }

  async addWorktree(repo: string, path: string): Promise<void> {
    const result = await this.git(repo, ['worktree', 'add', '--detach', path, 'HEAD'], WORKTREE_TIMEOUT_MS)
    if (result.exitCode !== 0) throw new Error(messageOf(result))
  }

  async createWorktree(repo: string, path: string, branch: string, base: string): Promise<void> {
    const result = await this.git(repo, ['worktree', 'add', '-b', branch, path, base], WORKTREE_TIMEOUT_MS)
    if (result.exitCode !== 0) throw new Error(messageOf(result))
  }

  async stageAll(path: string): Promise<void> {
    const result = await this.git(path, ['add', '-A'], WORKTREE_TIMEOUT_MS)
    if (result.exitCode !== 0) throw new Error(messageOf(result))
  }

  async stagedFiles(path: string): Promise<readonly string[]> {
    const result = await this.git(path, ['diff', '--cached', '--name-only', '-z'])
    if (result.exitCode !== 0) throw new Error(messageOf(result))
    return result.stdout.split('\0').filter((name) => name !== '')
  }

  async deleteBranch(repo: string, branch: string): Promise<void> {
    const result = await this.git(repo, ['branch', '-D', branch])
    if (result.exitCode !== 0) throw new Error(messageOf(result))
  }

  async listWorktrees(repo: string): Promise<readonly string[]> {
    const result = await this.git(repo, ['worktree', 'list', '--porcelain'])
    if (result.exitCode !== 0) throw new Error(messageOf(result))
    /**
     * The porcelain answer is one block per worktree, opened by the worktree's
     * `worktree <path>` line. A block git marks `prunable` names an entry whose
     * directory is gone, and a worktree whose directory git cannot read any
     * more is gone with it: both are skipped rather than handed on to be
     * watched, where their status could not be read and every writing run
     * would be refused for a checkout that is not there.
     */
    const listed: { path: string; prunable: boolean }[] = []
    let current: { path: string; prunable: boolean } | undefined
    for (const line of result.stdout.split('\n')) {
      if (line.startsWith('worktree ')) {
        if (current !== undefined) listed.push(current)
        current = { path: line.slice('worktree '.length).trim(), prunable: false }
      } else if (line === 'prunable' || line.startsWith('prunable ')) {
        if (current !== undefined) current = { ...current, prunable: true }
      }
    }
    if (current !== undefined) listed.push(current)
    return listed.filter((entry) => !entry.prunable).map((entry) => entry.path)
  }

  async status(path: string): Promise<string> {
    const result = await this.git(path, ['status', '--porcelain'])
    if (result.exitCode !== 0) throw new Error(messageOf(result))
    return result.stdout
  }

  /**
   * `git worktree remove --force`, which takes the directory with it. A
   * worktree git no longer knows (its directory moved, or a lock was left) is
   * deleted by hand and pruned, so a run never leaves its worktree behind.
   */
  async removeWorktree(repo: string, path: string): Promise<void> {
    const result = await this.git(repo, ['worktree', 'remove', '--force', path], WORKTREE_TIMEOUT_MS)
    if (result.exitCode === 0) return
    await rm(path, { force: true, recursive: true })
    const pruned = await this.git(repo, ['worktree', 'prune'])
    if (pruned.exitCode !== 0) throw new Error(messageOf(pruned))
  }

  /** The command's exit code and output; a git that cannot start or does not answer in time rejects. */
  private git(path: string, args: readonly string[], timeoutMs: number = this.timeoutMs): Promise<GitResult> {
    return new Promise((resolve, reject) => {
      const options = {
        encoding: 'utf8' as const,
        env: { ...process.env, PATH: this.path },
        maxBuffer: OUTPUT_LIMIT,
        timeout: timeoutMs,
      }
      execFile('git', ['-C', path, ...args], options, (error, stdout, stderr) => {
        if (error === null) resolve({ exitCode: 0, stderr, stdout })
        else if (typeof error.code === 'number') resolve({ exitCode: error.code, stderr, stdout })
        else if (error.killed === true) reject(new Error(`git did not answer within ${String(timeoutMs)} ms`))
        else reject(new Error(`cannot run git: ${error.message}`))
      })
    })
  }
}

/** git's own words for a failure, or its exit code when it said nothing. */
function messageOf(result: GitResult): string {
  const message = result.stderr.trim()
  return message === '' ? `git exited ${String(result.exitCode)}` : message
}
