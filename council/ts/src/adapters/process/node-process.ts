import { spawn } from 'node:child_process'

import type { ProcessCommand, ProcessPort, ProcessResult } from '../../ports/index.js'

export class NodeProcessAdapter implements ProcessPort {
  async exec(command: ProcessCommand): Promise<ProcessResult> {
    const child = spawn(command.command, [...command.args], {
      ...(command.cwd === undefined ? {} : { cwd: command.cwd }),
      env: command.env === undefined ? process.env : { ...process.env, ...command.env },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    // The streams decode, so a character split across two chunks stays whole.
    child.stdout.setEncoding('utf8')
    child.stderr.setEncoding('utf8')
    child.stdout.on('data', (chunk: string) => {
      stdout += chunk
    })
    child.stderr.on('data', (chunk: string) => {
      stderr += chunk
    })
    const timeout = setProcessTimeout(child, command.timeoutMs)
    const exitCode = await new Promise<number>((resolve, reject) => {
      child.once('error', reject)
      child.once('close', (code) => {
        resolve(processExitCode(code))
      })
    }).finally(() => {
      if (timeout !== undefined) clearTimeout(timeout)
    })

    return {
      exitCode,
      stderr,
      stdout,
    }
  }
}

function setProcessTimeout(
  child: ReturnType<typeof spawn>,
  timeoutMs: number | undefined,
): NodeJS.Timeout | undefined {
  if (timeoutMs === undefined) return undefined
  const timeout = setTimeout(() => {
    child.kill('SIGTERM')
  }, timeoutMs)
  timeout.unref()
  return timeout
}

function processExitCode(code: number | null): number {
  if (code !== null) return code
  return 124
}
