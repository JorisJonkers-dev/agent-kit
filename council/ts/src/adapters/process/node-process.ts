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
    child.stdout.on('data', (chunk: Buffer) => {
      stdout += chunk.toString('utf8')
    })
    child.stderr.on('data', (chunk: Buffer) => {
      stderr += chunk.toString('utf8')
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
