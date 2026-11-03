import { execFile } from 'node:child_process'

export interface ProcessRequest {
  readonly command: string
  readonly args: readonly string[]
}

export interface ProcessResult {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
}

/** Runs one program and reports how it exited; the gh and git adapters reach the system only through it. */
export interface BuildTrackerProcess {
  /** Resolves for any exit code; rejects only when the program cannot be started. */
  exec(request: ProcessRequest): Promise<ProcessResult>
}

const OUTPUT_LIMIT = 64 * 1024 * 1024

export class NodeBuildTrackerProcess implements BuildTrackerProcess {
  exec(request: ProcessRequest): Promise<ProcessResult> {
    return new Promise((resolve, reject) => {
      execFile(request.command, [...request.args], { encoding: 'utf8', maxBuffer: OUTPUT_LIMIT }, (error, stdout, stderr) => {
        if (error === null) resolve({ exitCode: 0, stderr, stdout })
        else if (typeof error.code === 'number') resolve({ exitCode: error.code, stderr, stdout })
        else reject(new Error(`cannot run ${request.command}: ${error.message}`))
      })
    })
  }
}
