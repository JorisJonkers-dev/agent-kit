import type { ContentAddressed, ContextLinked } from './common.js'
import type { EngineTagged } from './engine.js'

export interface ReviewVerdict extends ContentAddressed, EngineTagged {
  readonly satisfied: boolean
  readonly reasons: string
  readonly issues: readonly string[]
  readonly task_id?: string
  readonly reviewer?: string
}

export interface RoutingVerdict extends ContentAddressed, ContextLinked, EngineTagged {
  readonly route: string
  readonly reasons: string
  readonly task_id?: string
  readonly candidate_routes?: readonly string[]
}
