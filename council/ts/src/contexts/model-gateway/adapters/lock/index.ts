import { open, rm, stat } from 'node:fs/promises'

/** How long a writer waits before it looks at the lock again. */
const RETRY_MS = 10

/** How long a writer waits for another process's lock before it gives up. */
const TIMEOUT_MS = 10_000

/**
 * A lock older than this belongs to a process that died holding it, and is
 * broken. A live writer holds it for one append, a few milliseconds, and the
 * value must stay below TIMEOUT_MS: a writer that gave up waiting before the
 * lock could be broken would lose its record.
 */
const STALE_MS = 5_000

export interface LockTimings {
  readonly retryMs?: number
  readonly staleMs?: number
  readonly timeoutMs?: number
}

/**
 * An exclusive lock file. Two gateway processes taking the same lock take
 * turns, so what each does under it is never interleaved with the other's; a
 * lock left behind by a process that died is broken once it is old enough.
 */
export class FileLock {
  private readonly retryMs: number
  private readonly staleMs: number
  private readonly timeoutMs: number

  /** `name` is what the lock guards, for the message of a writer that gave up waiting. */
  constructor(
    private readonly name: string,
    timings: LockTimings = {},
  ) {
    this.retryMs = timings.retryMs ?? RETRY_MS
    this.staleMs = timings.staleMs ?? STALE_MS
    this.timeoutMs = timings.timeoutMs ?? TIMEOUT_MS
  }

  /** Runs `action` holding the lock at `lock`, and releases it whatever the action does. */
  async run<T>(lock: string, action: () => Promise<T>): Promise<T> {
    await this.acquire(lock)
    try {
      return await action()
    } finally {
      await rm(lock, { force: true })
    }
  }

  /** Takes the lock, waiting for whoever holds it; a lock that is too old to be alive is broken. */
  private async acquire(lock: string): Promise<void> {
    const deadline = Date.now() + this.timeoutMs
    for (;;) {
      if (await this.take(lock)) return
      if (await this.stale(lock)) {
        await rm(lock, { force: true })
        continue
      }
      if (Date.now() >= deadline) {
        throw new Error(`${lock}: another gateway process held the ${this.name} lock for ${String(this.timeoutMs)} ms`)
      }
      await new Promise<void>((resolve) => {
        setTimeout(resolve, this.retryMs)
      })
    }
  }

  /** Creates the lock file, or says that someone else has it. */
  private async take(lock: string): Promise<boolean> {
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
  private async stale(lock: string): Promise<boolean> {
    try {
      return Date.now() - (await stat(lock)).mtimeMs > this.staleMs
    } catch {
      return false
    }
  }
}
