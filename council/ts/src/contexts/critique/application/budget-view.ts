import { totalSpend, utcDay, type AntigravityQuota, type FlowSpend } from '../domain/budget.js'
import { stageOutlooks, type StageOutlook } from '../domain/budget-view.js'
import { EMPTY_LEDGER, parseLedger } from '../domain/ledger.js'
import type { Refusal } from '../domain/outcomes.js'
import { FlowBudget } from './budget.js'
import { decoded, readInput } from './decode.js'
import { FlowLocator, flowKeys, readFlowName } from './flow-state.js'
import { ledgerPath } from './locations.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RoutingSource } from './routing.js'

export interface BudgetResult {
  readonly status: 'read'
  readonly flow: string
  readonly vibe: {
    /** The flow's spend, its draft's included: the state's `vibe_spend_usd`. */
    readonly spent_usd: number
    readonly cap_usd: number
    /** The spend under each flow key, the spec's then its draft's; each key is held to the cap on its own. */
    readonly flows: readonly FlowSpend[]
  }
  /** This machine's agy launches on the UTC day of the clock, which is the day prepare counts. */
  readonly antigravity: AntigravityQuota & { readonly day: string; readonly invocations: number }
  /** Each stage routing defines, in flow order, with what the limits would do to it now. */
  readonly stages: readonly StageOutlook[]
}

/**
 * What the next costly stage of one feature would draw on: the Vibe spend
 * of its flow against the per-flow cap, and today's Antigravity launches
 * against the daily cap and the reserve. It reads the ledger and routing
 * (with this machine's override) through the code the stages enforce the
 * limits with, so the figures cannot drift from what prepare and fan-out
 * do. It launches nothing and writes nothing.
 */
export class BudgetCommand implements CritiqueCommand<BudgetResult | Refusal> {
  private readonly flows: FlowLocator
  private readonly budget: FlowBudget
  private readonly routing: RoutingSource

  constructor(private readonly deps: CritiqueDeps) {
    this.flows = new FlowLocator(deps)
    this.budget = new FlowBudget(deps)
    this.routing = new RoutingSource(deps)
  }

  async run(raw: unknown): Promise<BudgetResult | Refusal> {
    const input = readInput(raw, 'budget', (fields) => readFlowName(fields, 'budget'))
    const located = await this.flows.locate(input)
    if (located.status === 'refused') return located
    const { keys } = flowKeys(await this.ledger(), located, input.draft)
    const routing = await this.routing.load()
    const spends = await this.budget.vibeSpendByFlow(located.repository.key, keys)
    const invocations = await this.budget.agyInvocationsToday()
    return {
      antigravity: { ...routing.limits.antigravity_quota, day: utcDay(this.deps.clock.now()), invocations },
      flow: located.flow,
      // Before /to-spec the flow is the draft's own; after it the spec's comes first, then its draft's.
      stages: stageOutlooks(routing, {
        agyToday: invocations,
        draft: (located.spec === undefined ? spends[0] : spends[1]) ?? null,
        spec: located.spec === undefined ? null : (spends[0] ?? null),
      }),
      status: 'read',
      vibe: { cap_usd: routing.limits.budget.vibe_flow_cap_usd, flows: spends, spent_usd: totalSpend(spends) },
    }
  }

  private async ledger() {
    const path = ledgerPath(this.deps.env)
    const text = (await this.deps.files.exists(path)) ? await this.deps.files.readText(path) : ''
    return decoded('invalid-ledger', path, () => (text === '' ? EMPTY_LEDGER : parseLedger(text)))
  }
}
