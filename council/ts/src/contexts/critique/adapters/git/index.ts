import type { ProcessPort, ProcessResult } from '../../../../ports/index.js'
import type { CritiqueGitPort } from '../../application/ports.js'
import type { GitRemote } from '../../domain/family-allowlist.js'

/** `git config --get-regexp` exits 1 when nothing matches. */
const NO_MATCH = 1

export class ProcessCritiqueGit implements CritiqueGitPort {
  constructor(private readonly process: ProcessPort) {}

  async root(cwd: string): Promise<string> {
    return (await this.git(cwd, ['rev-parse', '--show-toplevel'])).stdout.trim()
  }

  async commonDir(cwd: string): Promise<string> {
    return (await this.git(cwd, ['rev-parse', '--path-format=absolute', '--git-common-dir'])).stdout.trim()
  }

  async remotes(cwd: string): Promise<readonly GitRemote[]> {
    const result = await this.git(cwd, ['config', '--get-regexp', '^remote\\..*\\.url$'], [NO_MATCH])
    return result.stdout
      .split('\n')
      .map((line) => /^remote\.(.+)\.url (.+)$/.exec(line.trim()))
      .flatMap((match) => (match === null ? [] : [{ name: match[1] ?? '', url: match[2] ?? '' }]))
  }

  private async git(cwd: string, args: readonly string[], allowed: readonly number[] = []): Promise<ProcessResult> {
    const result = await this.process.exec({ args: ['-C', cwd, ...args], command: 'git' })
    if (result.exitCode !== 0 && !allowed.includes(result.exitCode)) {
      throw new Error(`git ${args.join(' ')} failed: ${result.stderr.trim()}`)
    }
    return result
  }
}
