import type { ContentAddressed, ContextLinked } from './common.js'
import type { EngineTagged } from './engine.js'

export interface Amendment extends ContentAddressed, ContextLinked, EngineTagged {
  readonly id: string
  readonly summary: string
  readonly reason?: string
  readonly status?: string
  readonly task_refs?: readonly string[]
  readonly supersedes?: readonly string[]
  readonly discovered_from?: string
}
