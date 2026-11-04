import { ledgerPath as gatewayLedgerPath } from '../../model-gateway/index.js'
import { invocationLines, type LoggedInvocation } from '../domain/budget.js'
import { gatewayLedgerLines } from '../domain/gateway-ledger.js'
import { EMPTY_LEDGER, parseLedger, type Ledger, type LedgerCriticRun } from '../domain/ledger.js'
import { decoded } from './decode.js'
import { ledgerPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'

/** What critique's own ledger and the gateway's hold together: the runs, findings and launches. */
export interface LedgerRead {
  readonly ledger: Ledger
  readonly invocations: readonly LoggedInvocation[]
  /**
   * The runs critique's own ledger holds a run line for, which is what
   * `record` writes: the gateway's record says a critic ran, not that the run
   * was recorded, so it never makes a run look recorded.
   */
  readonly recorded_runs: readonly LedgerCriticRun[]
}

/**
 * The one ledger critique reads: the gateway's, where every critic run is
 * recorded from now on, together with critique's own ledger, which holds the
 * runs recorded before the move and the flow events that belong to no run.
 * Nothing is migrated or deleted, so a run that both ledgers hold — one
 * recorded while the move was made — counts once, by its run and critic id.
 */
export class LedgerSource {
  constructor(private readonly deps: Pick<CritiqueDeps, 'env' | 'files'>) {}

  async read(): Promise<LedgerRead> {
    const own = await this.text(ledgerPath(this.deps.env))
    const gateway = gatewayLedgerLines(await this.text(gatewayLedgerPath(this.deps.env)))
    const ledger = decoded('invalid-ledger', ledgerPath(this.deps.env), () => (own === '' ? EMPTY_LEDGER : parseLedger(own)))
    return {
      invocations: merge(invocationLines(own), gateway.invocations, (line) => `${line.run_id ?? ''} ${line.critic_id} ${line.kind}`),
      ledger: {
        findings: ledger.findings,
        flow_events: ledger.flow_events,
        runs: merge(ledger.runs, gateway.runs, (run) => `${run.run_id} ${run.critic_id}`),
        skipped_reviews: ledger.skipped_reviews,
      },
      recorded_runs: ledger.runs,
    }
  }

  private async text(path: string): Promise<string> {
    return (await this.deps.files.exists(path)) ? this.deps.files.readText(path) : ''
  }
}

/**
 * Critique's own lines, in order, then the gateway's that critique's do not
 * already hold. A launch both ledgers recorded counts once; two launches of
 * the same critic in the same run, which critique's ledger holds as two
 * lines, still count twice.
 */
function merge<T>(own: readonly T[], gateway: readonly T[], key: (line: T) => string): readonly T[] {
  const seen = new Set(own.map(key))
  return [...own, ...gateway.filter((line) => !seen.has(key(line)))]
}
