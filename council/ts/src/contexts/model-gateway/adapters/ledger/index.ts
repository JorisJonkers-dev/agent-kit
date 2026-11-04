import { appendFile, mkdir, readFile } from 'node:fs/promises'
import { dirname } from 'node:path'

import type { EnvPort } from '../../../../ports/index.js'
import { ledgerPath } from '../../application/locations.js'
import type { ModelGatewayLedgerPort } from '../../application/ports.js'
import { ledgerLine, type LedgerRecord } from '../../domain/ledger.js'
import { FileLock, type LockTimings } from '../lock/index.js'

export type LedgerLockTimings = LockTimings

/**
 * The gateway's ledger: one JSON line per run, appended under an exclusive
 * lock file beside it. Two gateway processes appending at once take turns, so
 * a line is never interleaved with another; a lock left behind by a process
 * that died is broken once it is old enough.
 */
export class NodeModelGatewayLedger implements ModelGatewayLedgerPort {
  private readonly lock: FileLock

  constructor(
    private readonly env: EnvPort,
    timings: LedgerLockTimings = {},
  ) {
    this.lock = new FileLock('ledger', timings)
  }

  async append(record: LedgerRecord): Promise<void> {
    const path = ledgerPath(this.env)
    await mkdir(dirname(path), { recursive: true })
    await this.lock.run(`${path}.lock`, () => appendFile(path, ledgerLine(record)))
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
}
