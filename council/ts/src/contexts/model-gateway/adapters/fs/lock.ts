import { mkdir, open, rm, stat } from 'node:fs/promises'
import { dirname } from 'node:path'

/** How long a writer waits before it looks at the lock again. */
const RETRY_MS = 10

/** How long a writer waits for another process's lock before it gives up. */
const TIMEOUT_MS = 10_000

/**
 * A lock older than this belongs to a process that died holding it, and is
 * broken. A live writer holds it for one write, a few milliseconds, and the
 * value must stay below TIMEOUT_MS: a writer that gave up waiting before the
 * lock could be broken would lose its write.
 */
const STALE_MS = 5_000

export interface FileLockTimings {
  readonly retryMs?: number
  readonly staleMs?: number
  readonly timeoutMs?: number
}

/**
 * Runs `action` while holding an exclusive lock file beside `path`, so two
 * gateway processes writing the same file take turns. A lock left behind by a
 * process that died is broken once it is old enough; `what` names the file in
 * the error a writer that waited too long reports.
 */
export async function withFileLock<T>(
  path: string,
  what: string,
  timings: FileLockTimings,
  action: () => Promise<T>,
): Promise<T> {
  const lock = `${path}.lock`
  await mkdir(dirname(path), { recursive: true })
  await acquire(lock, what, timings)
  try {
    return await action()
  } finally {
    await rm(lock, { force: true })
  }
}

/** Takes the lock, waiting for whoever holds it; a lock that is too old to be alive is broken. */
async function acquire(lock: string, what: string, timings: FileLockTimings): Promise<void> {
  const retryMs = timings.retryMs ?? RETRY_MS
  const staleMs = timings.staleMs ?? STALE_MS
  const timeoutMs = timings.timeoutMs ?? TIMEOUT_MS
  const deadline = Date.now() + timeoutMs
  for (;;) {
    if (await take(lock)) return
    if (await stale(lock, staleMs)) {
      await rm(lock, { force: true })
      continue
    }
    if (Date.now() >= deadline) {
      throw new Error(`${lock}: another gateway process held the ${what} lock for ${String(timeoutMs)} ms`)
    }
    await new Promise<void>((resolve) => {
      setTimeout(resolve, retryMs)
    })
  }
}

/** Creates the lock file, or says that someone else has it. */
async function take(lock: string): Promise<boolean> {
  try {
    const handle = await open(lock, 'wx')
    await handle.close()
    return true
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'EEXIST') return false
    throw error
  }
}

/** Whether the lock is older than a live writer could be; a lock that vanished meanwhile is not stale. */
async function stale(lock: string, staleMs: number): Promise<boolean> {
  try {
    return Date.now() - (await stat(lock)).mtimeMs > staleMs
  } catch {
    return false
  }
}
