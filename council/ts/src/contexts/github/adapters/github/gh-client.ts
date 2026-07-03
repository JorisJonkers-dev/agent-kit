import type { ProcessCommand, ProcessPort, ProcessResult } from '../../../../ports/index.js'

export class GithubCommandError extends Error {
  readonly args: readonly string[]
  readonly cwd: string
  readonly result: ProcessResult

  constructor(args: readonly string[], cwd: string, result: ProcessResult) {
    const detail =
      result.stderr.trim() || result.stdout.trim() || `exit code ${String(result.exitCode)}`
    super(`gh ${args.join(' ')} failed in ${cwd}: ${detail}`)
    this.name = 'GithubCommandError'
    this.args = args
    this.cwd = cwd
    this.result = result
  }
}

export class GhCommandClient {
  private readonly process: ProcessPort

  constructor(process: ProcessPort) {
    this.process = process
  }

  async ghJson(cwd: string, args: readonly string[]): Promise<string> {
    const result = await this.gh(cwd, args)
    return result.stdout
  }

  async gh(cwd: string, args: readonly string[]): Promise<ProcessResult> {
    const command: ProcessCommand = {
      args,
      command: 'gh',
      cwd,
    }
    const result = await this.process.exec(command)
    if (result.exitCode !== 0) {
      throw new GithubCommandError(args, cwd, result)
    }
    return result
  }
}
