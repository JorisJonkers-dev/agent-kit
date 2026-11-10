import type { GateResult } from '../domain/gate.js'
import type { Refusal } from '../domain/outcomes.js'
import { AllowCommand, type AllowResult } from './allow.js'
import { BudgetCommand, type BudgetResult } from './budget-view.js'
import { CheckAdrsCommand, type CheckAdrsResult } from './check-adrs.js'
import { CheckTicketsCommand, type CheckTicketsResult } from './check-tickets.js'
import { FactsCommand, type FactsResult } from './facts.js'
import { FanOutCommand, type FanOutResult } from './fan-out.js'
import { GateCommand } from './gate.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { PrepareCommand, type PrepareResult } from './prepare.js'
import { RecordCommand, type RecordResult } from './record.js'
import { RefuteCommand, type RefuteResult } from './refute.js'
import { ReviewsCommand, type ReviewsResult } from './reviews.js'
import { SessionStartCommand, type SessionStartResult } from './session-start.js'
import { SetupCommand, type SetupResult } from './setup.js'
import { StateCommand, type StateResult } from './flow-state.js'
import { StatsCommand, type StatsResult } from './stats.js'
import { TriageCommand, type TriageResult } from './triage.js'

/** The runner's commands. */
export interface CritiqueCommands {
  readonly prepare: CritiqueCommand<PrepareResult | Refusal>
  readonly facts: CritiqueCommand<FactsResult | Refusal>
  readonly fanOut: CritiqueCommand<FanOutResult>
  readonly record: CritiqueCommand<RecordResult>
  readonly refute: CritiqueCommand<RefuteResult>
  readonly allow: CritiqueCommand<AllowResult | Refusal>
  readonly budget: CritiqueCommand<BudgetResult | Refusal>
  readonly checkAdrs: CritiqueCommand<CheckAdrsResult | Refusal>
  readonly checkTickets: CritiqueCommand<CheckTicketsResult | Refusal>
  readonly stats: CritiqueCommand<StatsResult | Refusal>
  readonly state: CritiqueCommand<StateResult | Refusal>
  readonly reviews: CritiqueCommand<ReviewsResult | Refusal>
  readonly triage: CritiqueCommand<TriageResult | Refusal>
  readonly gate: CritiqueCommand<GateResult>
  readonly sessionStart: CritiqueCommand<SessionStartResult>
  readonly setup: CritiqueCommand<SetupResult>
}

export function createCritiqueCommands(deps: CritiqueDeps): CritiqueCommands {
  return {
    allow: new AllowCommand(deps),
    budget: new BudgetCommand(deps),
    checkAdrs: new CheckAdrsCommand(deps),
    checkTickets: new CheckTicketsCommand(deps),
    facts: new FactsCommand(deps),
    fanOut: new FanOutCommand(deps),
    gate: new GateCommand(deps),
    prepare: new PrepareCommand(deps),
    record: new RecordCommand(deps),
    refute: new RefuteCommand(deps),
    reviews: new ReviewsCommand(deps),
    sessionStart: new SessionStartCommand(deps),
    setup: new SetupCommand(deps),
    state: new StateCommand(deps),
    stats: new StatsCommand(deps),
    triage: new TriageCommand(deps),
  }
}
