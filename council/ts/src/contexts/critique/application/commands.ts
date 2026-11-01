import type { GateResult } from '../domain/gate.js'
import type { Refusal } from '../domain/outcomes.js'
import { AllowCommand, type AllowResult } from './allow.js'
import { CheckTicketsCommand, type CheckTicketsResult } from './check-tickets.js'
import { FanOutCommand, type FanOutResult } from './fan-out.js'
import { GateCommand } from './gate.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { PrepareCommand, type PrepareResult } from './prepare.js'
import { RecordCommand, type RecordResult } from './record.js'
import { StateCommand, type StateResult } from './flow-state.js'
import { StatsCommand, type StatsResult } from './stats.js'
import { TriageCommand, type TriageResult } from './triage.js'

/** The runner's commands. */
export interface CritiqueCommands {
  readonly prepare: CritiqueCommand<PrepareResult | Refusal>
  readonly fanOut: CritiqueCommand<FanOutResult>
  readonly record: CritiqueCommand<RecordResult>
  readonly allow: CritiqueCommand<AllowResult | Refusal>
  readonly checkTickets: CritiqueCommand<CheckTicketsResult | Refusal>
  readonly stats: CritiqueCommand<StatsResult | Refusal>
  readonly state: CritiqueCommand<StateResult | Refusal>
  readonly triage: CritiqueCommand<TriageResult | Refusal>
  readonly gate: CritiqueCommand<GateResult>
}

export function createCritiqueCommands(deps: CritiqueDeps): CritiqueCommands {
  return {
    allow: new AllowCommand(deps),
    checkTickets: new CheckTicketsCommand(deps),
    fanOut: new FanOutCommand(deps),
    gate: new GateCommand(deps),
    prepare: new PrepareCommand(deps),
    record: new RecordCommand(deps),
    state: new StateCommand(deps),
    stats: new StatsCommand(deps),
    triage: new TriageCommand(deps),
  }
}
