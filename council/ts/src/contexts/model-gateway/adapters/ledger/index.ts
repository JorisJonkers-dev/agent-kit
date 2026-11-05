import { appendFile, mkdir, readFile } from 'node:fs/promises'
import { dirname } from 'node:path'

import type { EnvPort } from '../../../../ports/index.js'
import { ledgerPath } from '../../application/locations.js'
import type { LedgerChange, ModelGatewayLedgerPort } from '../../application/ports.js'
import { ledgerLine, type LedgerEntry } from '../../domain/ledger.js'
import { FileLock, type LockTimings } from '../lock/index.js'

export type LedgerLockTimings = LockTimings

/**
 * The gateway's ledger: one JSON line per run, reservation and settlement,
 * appended under an exclusive lock file beside it. Two gateway processes
 * appending at once take turns, so a line is never interleaved with another;
 * a lock left behind by a process that died is broken once it is old enough.
 *
 * A change reads the ledger, decides and appends under that same lock, so two
 * gateway processes never both fit a reservation the other would have refused.
 */
export class NodeModelGatewayLedger implements ModelGatewayLedgerPort {
  private readonly lock: FileLock

  constructor(
    private readonly env: EnvPort,
    timings: LedgerLockTimings = {},
  ) {
    this.lock = new FileLock('ledger', timings)
  }

  async append(record: LedgerEntry): Promise<void> {
    const path = ledgerPath(this.env)
    await mkdir(dirname(path), { recursive: true })
    await this.lock.run(`${path}.lock`, () => appendFile(path, ledgerLine(record)))
  }

  async read(): Promise<readonly LedgerEntry[]> {
    return this.load()
  }

  async update<T>(change: (entries: readonly LedgerEntry[]) => LedgerChange<T>): Promise<T> {
    const path = ledgerPath(this.env)
    await mkdir(dirname(path), { recursive: true })
    return this.lock.run(`${path}.lock`, async () => {
      const changed = change(await this.load())
      if (changed.append.length > 0) await appendFile(path, changed.append.map(ledgerLine).join(''))
      return changed.result
    })
  }

  /**
   * Every record the ledger holds, oldest first. A ledger that is not there
   * yet holds none; a line a writer left half-written is skipped, so one
   * crashed run never hides the rest of the history.
   */
  private async load(): Promise<readonly LedgerEntry[]> {
    const path = ledgerPath(this.env)
    let text: string
    try {
      text = await readFile(path, 'utf8')
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return []
      throw error
    }
    const records: LedgerEntry[] = []
    for (const line of text.split('\n')) {
      if (line.trim() === '') continue
      try {
        records.push(JSON.parse(line) as LedgerEntry)
      } catch {
        // A torn line from a writer that died mid-append: the rest of the ledger still counts.
      }
    }
    return records
  }
}
