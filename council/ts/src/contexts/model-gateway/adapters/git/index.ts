import { execFile } from 'node:child_process'

import type { GitRemote } from '../../../../shared-kernel/index.js'
import type { ModelGatewayGitPort } from '../../application/ports.js'

const OUTPUT_LIMIT = 1024 * 1024

/** How long git may take before the gateway gives up on it. */
const TIMEOUT_MS = 10_000

/** git's answer for a path that is in no repository; every other failure is an error. */
const NOT_A_REPOSITORY = /not a git repository/i

interface GitResult {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
}

/**
 * Finds a path's repository with git: its root and its remotes. It starts no
 * client, only git. A path in no repository has no root; a path git cannot
 * read at all, or does not answer for in time, is an error rather than a
 * repository with nothing to say.
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
      return root === '' ? undefined : root
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

  /** The command's exit code and output; a git that cannot start or does not answer in time rejects. */
  private git(path: string, args: readonly string[]): Promise<GitResult> {
    return new Promise((resolve, reject) => {
      const options = {
        encoding: 'utf8' as const,
        env: { ...process.env, PATH: this.path },
        maxBuffer: OUTPUT_LIMIT,
        timeout: this.timeoutMs,
      }
      execFile('git', ['-C', path, ...args], options, (error, stdout, stderr) => {
        if (error === null) resolve({ exitCode: 0, stderr, stdout })
        else if (typeof error.code === 'number') resolve({ exitCode: error.code, stderr, stdout })
        else if (error.killed === true) reject(new Error(`git did not answer within ${String(this.timeoutMs)} ms`))
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
