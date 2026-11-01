import { agyInvocationsOn, flowSpend, invocationLines, type InvocationLine, type LoggedInvocation } from '../domain/budget.js'
import type { CriticRun } from '../domain/critic.js'
import type { RunManifest } from '../domain/manifest.js'
import { ledgerPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'

/**
 * The runner's own record of what critics cost: every launch is appended to
 * the user's ledger as an invocation line as soon as it ends, and the flow's
 * Vibe spend and today's agy count are read back from those lines. The
 * ledger lives in the user's home, so the count is kept per machine.
 */
export class FlowBudget {
  constructor(private readonly deps: Pick<CritiqueDeps, 'clock' | 'env' | 'files'>) {}

  /** What the Vibe critics of this run's flow have been charged so far, this run's included. */
  async vibeSpend(manifest: Pick<RunManifest, 'repository' | 'flow'>): Promise<number> {
    return flowSpend(await this.lines(), manifest.repository, manifest.flow)
  }

  /** agy launches on this machine today. */
  async agyInvocationsToday(): Promise<number> {
    return agyInvocationsOn(await this.lines(), this.deps.clock.now())
  }

  async logInvocation(
    manifest: Pick<RunManifest, 'flow' | 'repository' | 'round' | 'run_id' | 'stage'>,
    run: Pick<CriticRun, 'charged_usd' | 'client' | 'critic_id' | 'family' | 'model' | 'status'>,
    kind: InvocationLine['kind'],
  ): Promise<void> {
    const line: InvocationLine = {
      charged_usd: run.charged_usd,
      client: run.client,
      critic_id: run.critic_id,
      family: run.family,
      flow: manifest.flow,
      kind,
      model: run.model,
      recorded_at: this.deps.clock.now().toISOString(),
      repository: manifest.repository,
      round: manifest.round,
      run_id: manifest.run_id,
      stage: manifest.stage,
      status: run.status,
      type: 'invocation',
    }
    await this.deps.files.appendText(ledgerPath(this.deps.env), `${JSON.stringify(line)}\n`)
  }

  private async lines(): Promise<readonly LoggedInvocation[]> {
    const path = ledgerPath(this.deps.env)
    return (await this.deps.files.exists(path)) ? invocationLines(await this.deps.files.readText(path)) : []
  }
}
