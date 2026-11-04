import type { PlanWindowUsage } from '../../model-gateway/index.js'
import { totalSpend, utcDay, type AntigravityQuota, type FlowSpend } from '../domain/budget.js'
import { stageOutlooks, type StageOutlook } from '../domain/budget-view.js'
import type { Refusal } from '../domain/outcomes.js'
import { FlowBudget } from './budget.js'
import { readInput } from './decode.js'
import { FlowLocator, flowKeys, readFlowName } from './flow-state.js'
import { LedgerSource } from './ledger-source.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RoutingSource } from './routing.js'

export interface BudgetResult {
  readonly status: 'read'
  readonly flow: string
  readonly vibe: {
    /** The flow's spend, its draft's included: the state's `vibe_spend_usd`. */
    readonly spent_usd: number
    /** The spend under each flow key, the spec's then its draft's. */
    readonly flows: readonly FlowSpend[]
    /**
     * Vibe's plan windows as the model gateway reads them, with what the
     * machine-wide ledger says is spent in each; empty when this machine
     * gives Vibe no plan limit.
     */
    readonly plan_limits: readonly PlanWindowUsage[]
    /** The gateway's warning for each window at or above its warning ratio. */
    readonly warnings: readonly string[]
  }
  /** This machine's agy launches on the UTC day of the clock, which is the day prepare counts. */
  readonly antigravity: AntigravityQuota & { readonly day: string; readonly invocations: number }
  /** Each stage routing defines, in flow order, with what the limits would do to it now. */
  readonly stages: readonly StageOutlook[]
}

/**
 * What the next costly stage of one feature would draw on: the Vibe spend of
 * its flow and Vibe's plan limit as the model gateway reads it, and today's
 * Antigravity launches against the daily cap and the reserve. It reads the
 * ledgers and routing (with this machine's override) through the code the
 * stages enforce the limits with, so the figures cannot drift from what
 * prepare and fan-out do. It launches nothing and writes nothing.
 */
export class BudgetCommand implements CritiqueCommand<BudgetResult | Refusal> {
  private readonly flows: FlowLocator
  private readonly budget: FlowBudget
  private readonly routing: RoutingSource
  private readonly source: LedgerSource

  constructor(private readonly deps: CritiqueDeps) {
    this.flows = new FlowLocator(deps)
    this.budget = new FlowBudget(deps)
    this.routing = new RoutingSource(deps)
    this.source = new LedgerSource(deps)
  }

  async run(raw: unknown): Promise<BudgetResult | Refusal> {
    const input = readInput(raw, 'budget', (fields) => readFlowName(fields, 'budget'))
    const located = await this.flows.locate(input)
    if (located.status === 'refused') return located
    const { keys } = flowKeys((await this.source.read()).ledger, located, input.draft)
    const routing = await this.routing.load()
    const spends = await this.budget.vibeSpendByFlow(located.repository.key, keys)
    const invocations = await this.budget.agyInvocationsToday()
    const plan = await this.deps.plan.read('vibe')
    return {
      antigravity: { ...routing.limits.antigravity_quota, day: utcDay(this.deps.clock.now()), invocations },
      flow: located.flow,
      // Before /to-spec the flow is the draft's own; after it the spec's comes first, then its draft's.
      stages: stageOutlooks(routing, {
        agyToday: invocations,
        draft: (located.spec === undefined ? spends[0] : spends[1]) ?? null,
        plan: plan.unable_to_start ?? null,
        spec: located.spec === undefined ? null : (spends[0] ?? null),
      }),
      status: 'read',
      vibe: { flows: spends, plan_limits: plan.windows, spent_usd: totalSpend(spends), warnings: plan.warnings },
    }
  }
}
