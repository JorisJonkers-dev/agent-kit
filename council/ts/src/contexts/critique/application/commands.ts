import type { Refusal } from '../domain/outcomes.js'
import { AllowCommand, type AllowResult } from './allow.js'
import { FanOutCommand, type FanOutResult } from './fan-out.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { PrepareCommand, type PrepareResult } from './prepare.js'
import { RecordCommand, type RecordResult } from './record.js'

/** The runner's four commands. */
export interface CritiqueCommands {
  readonly prepare: CritiqueCommand<PrepareResult | Refusal>
  readonly fanOut: CritiqueCommand<FanOutResult>
  readonly record: CritiqueCommand<RecordResult>
  readonly allow: CritiqueCommand<AllowResult | Refusal>
}

export function createCritiqueCommands(deps: CritiqueDeps): CritiqueCommands {
  return {
    allow: new AllowCommand(deps),
    fanOut: new FanOutCommand(deps),
    prepare: new PrepareCommand(deps),
    record: new RecordCommand(deps),
  }
}
