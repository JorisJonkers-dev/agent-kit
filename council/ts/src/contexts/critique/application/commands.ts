import { allow, type AllowResult } from './allow.js'
import { fanOut, type FanOutResult } from './fan-out.js'
import type { CritiqueDeps } from './ports.js'
import { prepare, type PrepareResult } from './prepare.js'
import { record, type RecordResult } from './record.js'
import type { Refusal } from '../domain/outcomes.js'

/** The runner's commands. Each takes the JSON the skill sends and returns JSON. */
export interface CritiqueCommands {
  prepare(input: unknown): Promise<PrepareResult | Refusal>
  fanOut(input: unknown): Promise<FanOutResult>
  record(input: unknown): Promise<RecordResult>
  allow(input: unknown): Promise<AllowResult | Refusal>
}

export function createCritiqueCommands(deps: CritiqueDeps): CritiqueCommands {
  return {
    allow: (input) => allow(deps, input),
    fanOut: (input) => fanOut(deps, input),
    prepare: (input) => prepare(deps, input),
    record: (input) => record(deps, input),
  }
}
