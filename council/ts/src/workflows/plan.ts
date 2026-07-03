import { resolveCouncilConfig, type CouncilConfig, type ResolvedCouncilConfig } from '../contexts/config/index.js'
import { classifyTriage, type TriageInput, type TriageVerdict } from '../contexts/triage/index.js'

export interface PlanInput {
  readonly brief?: string
  readonly config?: CouncilConfig
  readonly design?: boolean
  readonly runDir?: string
  readonly triage?: TriageInput
}

export interface PlanResult {
  readonly command: 'plan'
  readonly config: ResolvedCouncilConfig
  readonly designRequested: boolean
  readonly directTierPolicy: 'shrink-dag-only'
  readonly executesWorkers: false
  readonly estimatedModelCalls: number
  readonly runDir?: string
  readonly taskLimit?: number
  readonly triage?: TriageVerdict
}

export function planWorkflow(input: PlanInput = {}): PlanResult {
  const config = resolveCouncilConfig(input.config === undefined ? {} : { flags: input.config })
  const triage = input.triage ? classifyTriage(input.triage) : undefined
  const taskLimit = triage?.route === 'direct' ? 1 : undefined
  return {
    command: 'plan',
    config,
    designRequested: input.design ?? false,
    directTierPolicy: 'shrink-dag-only',
    executesWorkers: false,
    estimatedModelCalls: 2 + config.rounds * 4 + 1,
    ...(input.runDir ? { runDir: input.runDir } : {}),
    ...(taskLimit === undefined ? {} : { taskLimit }),
    ...(triage ? { triage } : {}),
  }
}
