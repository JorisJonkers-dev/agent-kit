import type { ContentAddressed, ContextLinked, JsonRecord } from './common.js'
import type { EngineTagged } from './engine.js'

export type TaskId = `T${number}` | `ck-${string}`

export type TaskDifficulty = 'trivial' | 'moderate' | 'hard'

export type TaskModel = 'haiku' | 'sonnet' | 'opus'

export type TaskRetryPolicy = JsonRecord

export type TaskResourceProfile = JsonRecord

export interface TaskResolvedAttachment {
  readonly activeSkills: readonly string[]
  readonly mcpProfile: string
}

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
  readonly success_criteria?: readonly string[]
  readonly verify_proves?: readonly string[]
  readonly failure_modes?: readonly string[]
  readonly retry_policy?: TaskRetryPolicy
  readonly resource_profile?: TaskResourceProfile
  readonly attachment?: TaskResolvedAttachment
  readonly human_review_required?: boolean
  readonly dev_notes?: string
  readonly spec_ref?: string
  readonly archetype?: string
  readonly context_profile?: string
  readonly discovered_from?: string
  readonly supersedes?: readonly TaskId[]
}
