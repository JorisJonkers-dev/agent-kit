import type { ProcessPort } from '../../../ports/index.js'

const VERSION_TIMEOUT_MS = 10_000

/** The first line `<binary> --version` prints, or null when the CLI is missing or fails. Free: no model is called. */
export async function cliVersion(process: ProcessPort, binary: string): Promise<string | null> {
  const result = await process
    .exec({ args: ['--version'], command: binary, timeoutMs: VERSION_TIMEOUT_MS })
    .catch(() => undefined)
  const line = result?.exitCode === 0 ? (result.stdout.split('\n')[0]?.trim() ?? '') : ''
  return line.length > 0 ? line : null
}
