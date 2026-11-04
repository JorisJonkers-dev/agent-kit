import { appendFile } from 'node:fs/promises'

import type { EnvPort } from '../../../../ports/index.js'
import { ledgerPath } from '../../application/locations.js'
import type { ModelGatewayLedgerPort } from '../../application/ports.js'
import { ledgerLine, type LedgerRecord } from '../../domain/ledger.js'
import { withFileLock, type FileLockTimings } from '../fs/lock.js'

export type LedgerLockTimings = FileLockTimings

/**
 * The gateway's ledger: one JSON line per run, appended under an exclusive
 * lock file beside it. Two gateway processes appending at once take turns, so
 * a line is never interleaved with another; a lock left behind by a process
 * that died is broken once it is old enough.
 */
export class NodeModelGatewayLedger implements ModelGatewayLedgerPort {
  constructor(
    private readonly env: EnvPort,
    private readonly timings: LedgerLockTimings = {},
  ) {}

  async append(record: LedgerRecord): Promise<void> {
    const path = ledgerPath(this.env)
    await withFileLock(path, 'ledger', this.timings, () => appendFile(path, ledgerLine(record)))
  }
}
