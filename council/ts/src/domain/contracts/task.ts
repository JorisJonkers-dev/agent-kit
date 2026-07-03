import type { ContentAddressed, ContextLinked } from './common.js'
import type { EngineTagged } from './engine.js'

export type TaskId = `T${number}` | `ck-${string}`

export type TaskDifficulty = 'trivial' | 'moderate' | 'hard'

export type TaskModel = 'haiku' | 'sonnet' | 'opus'

export interface Task extends ContentAddressed, ContextLinked, EngineTagged {
  readonly id: TaskId
  readonly title: string
  readonly objective: string
  readonly output_format: string
  readonly paths: readonly string[]
  readonly depends_on: readonly TaskId[]
  readonly difficulty: TaskDifficulty
  readonly model: TaskModel
  readonly verify: string
  readonly boundaries: string
  readonly acceptance_criteria?: readonly string[]
  readonly dev_notes?: string
  readonly spec_ref?: string
  readonly archetype?: string
  readonly context_profile?: string
  readonly discovered_from?: string
  readonly supersedes?: readonly TaskId[]
}
