import { spawn } from 'node:child_process'

import type { ProcessCommand, ProcessPort, ProcessResult } from '../../ports/index.js'

export interface NodeProcessAdapterOptions {
  /** How long a timed-out command has to exit after SIGTERM before it is sent SIGKILL. */
  readonly killGraceMs?: number
}

const DEFAULT_KILL_GRACE_MS = 5000

export class NodeProcessAdapter implements ProcessPort {
  private readonly killGraceMs: number

  constructor(options: NodeProcessAdapterOptions = {}) {
    this.killGraceMs = options.killGraceMs ?? DEFAULT_KILL_GRACE_MS
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    const child = spawn(command.command, [...command.args], {
      ...(command.cwd === undefined ? {} : { cwd: command.cwd }),
      env: command.env === undefined ? process.env : { ...process.env, ...command.env },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk: Buffer) => {
      stdout += chunk.toString('utf8')
    })
    child.stderr.on('data', (chunk: Buffer) => {
      stderr += chunk.toString('utf8')
    })
    const timer = new ProcessTimer(child, command.timeoutMs, this.killGraceMs)
    const exitCode = await new Promise<number>((resolve, reject) => {
      child.once('error', reject)
      child.once('close', (code) => {
        resolve(processExitCode(code))
      })
    }).finally(() => {
      timer.clear()
    })

    return {
      exitCode,
      stderr,
      stdout,
      ...(timer.fired ? { timedOut: true } : {}),
    }
  }
}

/**
 * Stops a command that outlives its timeout: SIGTERM first, then SIGKILL
 * once the grace period ends, so a command that ignores SIGTERM still stops.
 */
class ProcessTimer {
  fired = false
  private readonly timers: NodeJS.Timeout[] = []

  constructor(
    private readonly child: ReturnType<typeof spawn>,
    timeoutMs: number | undefined,
    private readonly killGraceMs: number,
  ) {
    if (timeoutMs !== undefined) {
      this.after(timeoutMs, () => {
        this.terminate()
      })
    }
  }

  clear(): void {
    for (const timer of this.timers) clearTimeout(timer)
  }

  private terminate(): void {
    this.fired = true
    this.child.kill('SIGTERM')
    this.after(this.killGraceMs, () => {
      this.child.kill('SIGKILL')
    })
  }

  private after(ms: number, action: () => void): void {
    const timer = setTimeout(action, ms)
    timer.unref()
    this.timers.push(timer)
  }
}

function processExitCode(code: number | null): number {
  if (code !== null) return code
  return 124
}
