import type {
  DagVerifyInput,
  DagVerifyResult,
  ProcessCommand,
  ProcessPort,
} from '../../ports/index.js'

export interface ProcessVerificationAdapterOptions {
  readonly shell?: string
  readonly shellArgs?: readonly string[]
  readonly env?: Readonly<Record<string, string>>
  readonly timeoutMs?: number
}

export class ProcessVerificationAdapter {
  private readonly env: Readonly<Record<string, string>> | undefined
  private readonly process: ProcessPort
  private readonly shell: string
  private readonly shellArgs: readonly string[]
  private readonly timeoutMs: number | undefined

  constructor(process: ProcessPort, options: ProcessVerificationAdapterOptions = {}) {
    this.env = options.env
    this.process = process
    this.shell = options.shell ?? 'sh'
    this.shellArgs = options.shellArgs ?? ['-lc']
    this.timeoutMs = options.timeoutMs
  }

  async verify(request: DagVerifyInput): Promise<DagVerifyResult> {
    const command = request.command.trim()
    if (command.length === 0) {
      return {
        command,
        exit_code: null,
        output: '',
        status: 'skipped',
      }
    }

    const processCommand: ProcessCommand = {
      args: [...this.shellArgs, command],
      command: this.shell,
      cwd: request.worktree_path,
      ...(this.env !== undefined ? { env: this.env } : {}),
      ...(this.timeoutMs !== undefined ? { timeoutMs: this.timeoutMs } : {}),
    }
    const result = await this.process.exec(processCommand)
    return {
      command,
      exit_code: result.exitCode,
      output: `${result.stdout}${result.stderr}`,
      status: result.exitCode === 0 ? 'passed' : 'failed',
    }
  }
}
