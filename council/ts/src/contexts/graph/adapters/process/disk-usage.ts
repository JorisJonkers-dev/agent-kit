import { execFile } from 'node:child_process'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)

export async function readDuBytes(path: string): Promise<number> {
  const { stdout } = await execFileAsync('du', ['-sk', path])
  return parseDuBytes(path, stdout)
}

export function parseDuBytes(path: string, stdout: string): number {
  const [kilobytes] = stdout.trim().split(/\s+/u)
  const parsed = Number(kilobytes)

  if (!Number.isFinite(parsed)) {
    throw new Error(`could not parse du output for ${path}: ${stdout}`)
  }

  return parsed * 1024
}
