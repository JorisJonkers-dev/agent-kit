import { appendFile, mkdir, open, readFile, rm, stat } from 'node:fs/promises'
import { dirname } from 'node:path'

import type { EnvPort } from '../../../../ports/index.js'
import { ledgerPath } from '../../application/locations.js'
import type { ModelGatewayLedgerPort } from '../../application/ports.js'
import { ledgerLine, type LedgerRecord } from '../../domain/ledger.js'

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

export interface LedgerLockTimings {
  readonly retryMs?: number
  readonly staleMs?: number
  readonly timeoutMs?: number
}

/**
 * The gateway's ledger: one JSON line per run, appended under an exclusive
 * lock file beside it. Two gateway processes appending at once take turns, so
 * a line is never interleaved with another; a lock left behind by a process
 * that died is broken once it is old enough.
 */
export class NodeModelGatewayLedger implements ModelGatewayLedgerPort {
  private readonly retryMs: number
  private readonly staleMs: number
  private readonly timeoutMs: number

  constructor(
    private readonly env: EnvPort,
    timings: LedgerLockTimings = {},
  ) {
    this.retryMs = timings.retryMs ?? RETRY_MS
    this.staleMs = timings.staleMs ?? STALE_MS
    this.timeoutMs = timings.timeoutMs ?? TIMEOUT_MS
  }

  async append(record: LedgerRecord): Promise<void> {
    const path = ledgerPath(this.env)
    await mkdir(dirname(path), { recursive: true })
    const lock = `${path}.lock`
    await this.acquire(lock)
    try {
      await appendFile(path, ledgerLine(record))
    } finally {
      await rm(lock, { force: true })
    }
  }

  /**
   * Every record the ledger holds, oldest first. A ledger that is not there
   * yet holds none; a line a writer left half-written is skipped, so one
   * crashed run never hides the rest of the history.
   */
  async read(): Promise<readonly LedgerRecord[]> {
    const path = ledgerPath(this.env)
    let text: string
    try {
      text = await readFile(path, 'utf8')
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return []
      throw error
    }
    const records: LedgerRecord[] = []
    for (const line of text.split('\n')) {
      if (line.trim() === '') continue
      try {
        records.push(JSON.parse(line) as LedgerRecord)
      } catch {
        // A torn line from a writer that died mid-append: the rest of the ledger still counts.
      }
    }
    return records
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
        throw new Error(`${lock}: another gateway process held the ledger lock for ${String(this.timeoutMs)} ms`)
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
