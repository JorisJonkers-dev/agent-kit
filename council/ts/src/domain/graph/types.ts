import type { Task, TaskId } from '../contracts/task.js'

export type GraphEdgeKind = 'depends_on' | 'discovered_from' | 'supersedes'

export type GraphIdStrategy = 'legacy-ordinal' | 'content-hash'

export type GraphTaskState = 'pending' | 'running' | 'closed' | 'stalled' | 'blocked'

export type GraphTaskDraft = Omit<Task, 'content_hash' | 'depends_on' | 'id'> & {
  readonly content_hash?: string
  readonly depends_on?: readonly TaskId[]
  readonly id?: TaskId
}

export interface GraphEdge {
  readonly from: TaskId
  readonly kind: GraphEdgeKind
  readonly to: TaskId
}

export interface GraphNode {
  readonly blocked_by: readonly TaskId[]
  readonly order: number
  readonly state: GraphTaskState
  readonly task: Task
}

export interface TaskGraph {
  readonly contentIndex: ReadonlyMap<string, TaskId>
  readonly edges: readonly GraphEdge[]
  readonly idStrategy: GraphIdStrategy
  readonly nextOrdinal: number
  readonly nodes: ReadonlyMap<TaskId, GraphNode>
}

export interface CreateTaskGraphOptions {
  readonly idStrategy?: GraphIdStrategy
}

export interface DiscoveredWorkBatch {
  readonly sourceId: TaskId
  readonly tasks: readonly GraphTaskDraft[]
}

export interface DiscoveredWorkDuplicate {
  readonly content_hash: string
  readonly existing_id: TaskId
}

export interface DiscoveredWorkResult {
  readonly deduped: readonly DiscoveredWorkDuplicate[]
  readonly graph: TaskGraph
  readonly ingested: readonly TaskId[]
}

export interface BoundsGateInput {
  readonly allowedPaths: readonly string[]
  readonly filesChanged: readonly string[]
  readonly status?: string
  readonly taskId: TaskId
}

export interface BoundsGateResult {
  readonly files_changed: readonly string[]
  readonly out_of_bounds: readonly string[]
  readonly status: string
}
