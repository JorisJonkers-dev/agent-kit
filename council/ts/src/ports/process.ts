export interface ProcessCommand {
  readonly command: string
  readonly args: readonly string[]
  readonly cwd?: string
  readonly env?: Readonly<Record<string, string>>
  readonly timeoutMs?: number
}

export interface ProcessResult {
  readonly stdout: string
  readonly stderr: string
  readonly exitCode: number
  /** True when the command outlived `timeoutMs` and was killed. */
  readonly timedOut?: boolean
}

export interface ProcessPort {
  exec(command: ProcessCommand): Promise<ProcessResult>
}
