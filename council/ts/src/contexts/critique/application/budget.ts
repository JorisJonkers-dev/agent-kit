import {
  agyInvocationsOn,
  spendByFlow,
  type FlowSpend,
  type InvocationLine,
  type LoggedInvocation,
} from '../domain/budget.js'
import type { CriticRun } from '../domain/critic.js'
import { critiqueLedgerRecord, type LedgerClient } from '../domain/gateway-ledger.js'
import type { RunManifest } from '../domain/manifest.js'
import { LedgerSource } from './ledger-source.js'
import { ledgerPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'

/**
 * The runner's own record of what critics cost: every launch is appended to
 * the user's ledger as an invocation line as soon as it ends, and to the
 * model gateway's ledger as one record of the run, so the gateway counts a
 * critic run the way it counts a delegation. The flow's Vibe spend and
 * today's agy count are read back from both ledgers, so a run recorded before
 * the move still counts.
 */
export class FlowBudget {
  private readonly source: LedgerSource

  constructor(private readonly deps: Pick<CritiqueDeps, 'clock' | 'env' | 'files' | 'ledger'>) {
    this.source = new LedgerSource(deps)
  }

  /** What the Vibe critics were charged under each of a feature's flow keys. */
  async vibeSpendByFlow(repository: string, flows: readonly string[]): Promise<readonly FlowSpend[]> {
    return spendByFlow(await this.lines(), repository, flows)
  }

  /** agy launches on this machine today. */
  async agyInvocationsToday(): Promise<number> {
    return agyInvocationsOn(await this.lines(), this.deps.clock.now())
  }

  /**
   * Records one launch: critique's own invocation line, and the gateway's
   * record of the run, which carries the gateway's fields beside critique's.
   * `spec` is the spec the run belongs to, where it has one. A gateway ledger
   * that will not take the record is not the stage's failure — the critics
   * have already run and spent money — so the message comes back for the
   * stage's summary instead of throwing.
   */
  async logInvocation(
    manifest: Pick<RunManifest, 'flow' | 'repository' | 'round' | 'run_id' | 'stage'>,
    run: Pick<CriticRun, 'charged_usd' | 'client' | 'cost_usd' | 'critic_id' | 'exit_code' | 'family' | 'model' | 'status'> &
      Partial<Pick<CriticRun, 'duration_ms' | 'failure' | 'finding_count' | 'lens_hashes' | 'lenses' | 'prompt_hashes' | 'retry_of' | 'role' | 'tokens' | 'vibe_cost_usd'>>,
    client: LedgerClient,
    spec?: string,
  ): Promise<string | undefined> {
    const at = this.deps.clock.now().toISOString()
    const line: InvocationLine = {
      charged_usd: run.charged_usd,
      client: run.client,
      critic_id: run.critic_id,
      family: run.family,
      flow: manifest.flow,
      kind: client.kind,
      model: run.model,
      recorded_at: at,
      repository: manifest.repository,
      round: manifest.round,
      run_id: manifest.run_id,
      stage: manifest.stage,
      status: run.status,
      type: 'invocation',
      ...(run.vibe_cost_usd === undefined ? {} : { vibe_cost_usd: run.vibe_cost_usd }),
    }
    await this.deps.files.appendText(ledgerPath(this.deps.env), `${JSON.stringify(line)}\n`)
    try {
      await this.deps.ledger.append(
        critiqueLedgerRecord(
          {
            flow: manifest.flow,
            repository: manifest.repository,
            round: manifest.round,
            run_id: manifest.run_id,
            stage: manifest.stage,
            ...(spec === undefined ? {} : { spec }),
          },
          run,
          client,
          at,
        ),
      )
    } catch (error) {
      return error instanceof Error ? error.message : String(error)
    }
    return undefined
  }

  private async lines(): Promise<readonly LoggedInvocation[]> {
    return (await this.source.read()).invocations
  }
}
