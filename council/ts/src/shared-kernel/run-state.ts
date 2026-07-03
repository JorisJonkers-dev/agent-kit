import type { ContentAddressed } from './common.js'
import type { EngineTagged } from './engine.js'

export interface RunState extends ContentAddressed, EngineTagged {
  readonly stage?: string
  readonly intensity?: string
  readonly rounds?: number
  readonly task_count?: number
  readonly spec_id?: string
  readonly spec_slug?: string
  readonly spec_relpath?: string
  readonly agents?: readonly string[]
  readonly integration_branch?: string
}
