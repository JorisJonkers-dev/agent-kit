import type { ContentAddressed, ContextLinked } from './common.js'

export interface DesignLedger extends ContentAddressed {
  readonly entries?: readonly DesignLedgerEntry[]
}

export interface DesignLedgerEntry extends ContentAddressed, ContextLinked {
  readonly id: string
  readonly decision: string
  readonly rationale?: string
  readonly status?: string
  readonly task_refs?: readonly string[]
  readonly supersedes?: readonly string[]
}
