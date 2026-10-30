import { allow, type AllowResult } from './allow.js'
import { fanOut, type FanOutResult } from './fan-out.js'
import type { CritiqueDeps } from './ports.js'
import { prepare, type PrepareResult } from './prepare.js'
import type { Refusal } from './repository.js'

/** The runner's commands. Each takes the JSON the skill sends and returns JSON. */
export interface CritiqueCommands {
  prepare(input: unknown): Promise<PrepareResult | Refusal>
  fanOut(input: unknown): Promise<FanOutResult>
  allow(input: unknown): Promise<AllowResult | Refusal>
}

export function createCritiqueCommands(deps: CritiqueDeps): CritiqueCommands {
  return {
    allow: (input) => allow(deps, input),
    fanOut: (input) => fanOut(deps, input),
    prepare: (input) => prepare(deps, input),
  }
}
