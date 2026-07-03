import { createHash } from 'node:crypto'

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

export function createTaskGraph(
  drafts: readonly GraphTaskDraft[],
  options: CreateTaskGraphOptions = {},
): TaskGraph {
  const idStrategy = options.idStrategy ?? 'legacy-ordinal'
  const nodes = new Map<TaskId, GraphNode>()
  let nextOrdinal = nextOrdinalAfter(drafts)

  drafts.forEach((draft, order) => {
    const contentHash = taskContentHash(draft)
    const minted = draft.id ?? mintTaskId(contentHash, idStrategy, nodes, nextOrdinal)
    nextOrdinal = nextOrdinalFor(minted, nextOrdinal)
    if (nodes.has(minted)) {
      throw new Error(`duplicate task id: ${minted}`)
    }
    nodes.set(minted, {
      blocked_by: [],
      order,
      state: 'pending',
      task: normalizeTask(draft, minted, contentHash),
    })
  })

  const graph = graphFromNodes(nodes, idStrategy, nextOrdinal)
  assertKnownDependencies(graph)
  assertAcyclic(graph)
  return graph
}

export function dispatchReadySet(graph: TaskGraph, limit = Number.POSITIVE_INFINITY): readonly TaskId[] {
  const criticalPaths = criticalPathLengths(graph)
  const ready = [...graph.nodes.values()].filter((node) => isReadyNode(graph, node))
  ready.sort((left, right) => {
    const leftPath = criticalPaths.get(left.task.id) ?? 0
    const rightPath = criticalPaths.get(right.task.id) ?? 0
    if (leftPath !== rightPath) {
      return rightPath - leftPath
    }
    if (left.order !== right.order) {
      return left.order - right.order
    }
    return left.task.id.localeCompare(right.task.id)
  })
  return ready.slice(0, limit).map((node) => node.task.id)
}

export function projectWaveView(graph: TaskGraph): readonly (readonly TaskId[])[] {
  const remaining = new Map<TaskId, readonly TaskId[]>()
  for (const node of graph.nodes.values()) {
    remaining.set(node.task.id, node.task.depends_on.filter((dependency) => graph.nodes.has(dependency)))
  }

  const waves: TaskId[][] = []
  const done = new Set<TaskId>()
  while (remaining.size > 0) {
    const ready = [...remaining.entries()]
      .filter(([, dependencies]) => dependencies.every((dependency) => done.has(dependency)))
      .map(([id]) => id)
      .sort()
    if (ready.length === 0) {
      throw new Error(`dependency cycle among tasks: ${[...remaining.keys()].sort().join(', ')}`)
    }
    waves.push(ready)
    ready.forEach((id) => {
      done.add(id)
      remaining.delete(id)
    })
  }
  return waves
}

export function planWaves(
  drafts: readonly GraphTaskDraft[],
  options: CreateTaskGraphOptions = {},
): readonly (readonly TaskId[])[] {
  return projectWaveView(createTaskGraph(drafts, options))
}

export function markTaskState(graph: TaskGraph, id: TaskId, state: GraphTaskState): TaskGraph {
  const node = graph.nodes.get(id)
  if (node === undefined) {
    throw new Error(`unknown task id: ${id}`)
  }
  const nodes = new Map(graph.nodes)
  nodes.set(id, { ...node, blocked_by: state === 'blocked' ? node.blocked_by : [], state })
  return graphFromNodes(nodes, graph.idStrategy, graph.nextOrdinal)
}

export function propagateStalled(graph: TaskGraph, stalledId: TaskId): TaskGraph {
  const stalled = graph.nodes.get(stalledId)
  if (stalled === undefined) {
    throw new Error(`unknown task id: ${stalledId}`)
  }

  const nodes = new Map(graph.nodes)
  nodes.set(stalledId, { ...stalled, blocked_by: [], state: 'stalled' })
  const queue: TaskId[] = [stalledId]
  let blockedBy = queue.shift()
  while (blockedBy !== undefined) {
    const blocker = blockedBy
    const dependents = dependsOnDependents(graph, blocker)
    dependents.forEach((dependentId) => {
      const dependent = nodes.get(dependentId)
      if (dependent === undefined || dependent.state === 'closed' || dependent.state === 'stalled') {
        return
      }
      queue.push(dependentId)
      nodes.set(dependentId, { ...dependent, blocked_by: [blocker], state: 'blocked' })
    })
    blockedBy = queue.shift()
  }

  return graphFromNodes(nodes, graph.idStrategy, graph.nextOrdinal)
}

export function ingestDiscoveredWork(graph: TaskGraph, batch: DiscoveredWorkBatch): DiscoveredWorkResult {
  if (!graph.nodes.has(batch.sourceId)) {
    throw new Error(`unknown discovery source: ${batch.sourceId}`)
  }

  const nodes = new Map(graph.nodes)
  const deduped: DiscoveredWorkDuplicate[] = []
  const ingested: TaskId[] = []
  let nextOrdinal = graph.nextOrdinal

  batch.tasks.forEach((draft) => {
    const contentHash = taskContentHash(draft)
    const existingId = firstContentOwner(nodes, contentHash)
    if (existingId !== undefined) {
      deduped.push({ content_hash: contentHash, existing_id: existingId })
      return
    }

    const minted = draft.id ?? mintTaskId(contentHash, graph.idStrategy, nodes, nextOrdinal)
    const duplicate = nodes.get(minted)
    if (duplicate !== undefined) {
      throw new Error(`duplicate task id: ${minted}`)
    }

    nextOrdinal = nextOrdinalFor(minted, nextOrdinal)
    const discoveredDraft: GraphTaskDraft = { ...draft, discovered_from: batch.sourceId }
    nodes.set(minted, {
      blocked_by: [],
      order: nodes.size,
      state: 'pending',
      task: normalizeTask(discoveredDraft, minted, contentHash),
    })
    ingested.push(minted)
  })

  const nextGraph = graphFromNodes(nodes, graph.idStrategy, nextOrdinal)
  assertKnownDependencies(nextGraph)
  assertAcyclic(nextGraph)
  return { deduped, graph: nextGraph, ingested }
}

export function compactClosedTasks(graph: TaskGraph): TaskGraph {
  const nodes = new Map<TaskId, GraphNode>()
  for (const node of graph.nodes.values()) {
    if (node.state !== 'closed') {
      const task: Task = {
        ...node.task,
        depends_on: node.task.depends_on.filter((dependency) => {
          const dependencyNode = graph.nodes.get(dependency)
          return dependencyNode?.state !== 'closed'
        }),
      }
      nodes.set(node.task.id, { ...node, task })
    }
  }
  return graphFromNodes(nodes, graph.idStrategy, graph.nextOrdinal)
}

export function applyBoundsGate(input: BoundsGateInput): BoundsGateResult {
  const outOfBounds = findOutOfBoundsFiles(input.filesChanged, input.allowedPaths, input.taskId)
  return {
    files_changed: input.filesChanged,
    out_of_bounds: outOfBounds,
    status: outOfBounds.length > 0 ? 'out-of-bounds' : (input.status ?? 'ok'),
  }
}

export function findOutOfBoundsFiles(
  filesChanged: readonly string[],
  allowedPaths: readonly string[],
  taskId: TaskId,
): readonly string[] {
  const allowed = new Set(allowedPaths)
  const storyPath = `workers/${taskId}/story.md`
  return filesChanged.filter((file) => !allowed.has(file) && file !== storyPath)
}

function graphFromNodes(
  nodes: ReadonlyMap<TaskId, GraphNode>,
  idStrategy: GraphIdStrategy,
  nextOrdinal: number,
): TaskGraph {
  return {
    contentIndex: contentIndexFor(nodes),
    edges: edgesFor(nodes),
    idStrategy,
    nextOrdinal,
    nodes,
  }
}

function normalizeTask(draft: GraphTaskDraft, id: TaskId, contentHash: string): Task {
  return {
    ...draft,
    content_hash: contentHash,
    depends_on: draft.depends_on ?? [],
    id,
  }
}

function taskContentHash(draft: GraphTaskDraft): string {
  return draft.content_hash ?? digest(stableJson(stripIdentity(draft)))
}

function stripIdentity(draft: GraphTaskDraft): Record<string, unknown> {
  const content = { ...draft } as Record<string, unknown>
  delete content.content_hash
  delete content.id
  return content
}

function stableJson(value: unknown): string {
  if (value === null || typeof value !== 'object') {
    const serialized: unknown = JSON.stringify(value)
    return typeof serialized === 'string' ? serialized : 'undefined'
  }
  if (Array.isArray(value)) {
    return `[${value.map((entry) => stableJson(entry)).join(',')}]`
  }
  const record = value as Record<string, unknown>
  return `{${Object.keys(record)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${stableJson(record[key])}`)
    .join(',')}}`
}

function digest(value: string): string {
  return createHash('sha256').update(value).digest('hex')
}

function mintTaskId(
  contentHash: string,
  idStrategy: GraphIdStrategy,
  nodes: ReadonlyMap<TaskId, GraphNode>,
  nextOrdinal: number,
): TaskId {
  if (idStrategy === 'content-hash') {
    let suffix = 12
    let candidate: TaskId = `ck-${contentHash.slice(0, suffix)}`
    let collision = 2
    while (nodes.has(candidate)) {
      if (suffix < contentHash.length) {
        suffix += 1
        candidate = `ck-${contentHash.slice(0, suffix)}`
      } else {
        candidate = `ck-${contentHash}-${String(collision)}`
        collision += 1
      }
    }
    return candidate
  }

  return ordinalTaskId(nextOrdinal)
}

function ordinalTaskId(nextOrdinal: number): `T${number}` {
  return `T${String(nextOrdinal)}` as `T${number}`
}

function nextOrdinalAfter(drafts: readonly GraphTaskDraft[]): number {
  return drafts.reduce((next, draft) => {
    if (draft.id?.startsWith('T') !== true) {
      return next
    }
    const ordinal = Number.parseInt(draft.id.slice(1), 10)
    return Number.isFinite(ordinal) ? Math.max(next, ordinal + 1) : next
  }, 1)
}

function nextOrdinalFor(id: TaskId, nextOrdinal: number): number {
  if (!id.startsWith('T')) {
    return nextOrdinal
  }
  const ordinal = Number.parseInt(id.slice(1), 10)
  return Number.isFinite(ordinal) ? Math.max(nextOrdinal, ordinal + 1) : nextOrdinal
}

function edgesFor(nodes: ReadonlyMap<TaskId, GraphNode>): readonly GraphEdge[] {
  const edges: GraphEdge[] = []
  for (const node of nodes.values()) {
    node.task.depends_on.forEach((dependency) => {
      if (nodes.has(dependency)) {
        edges.push({ from: dependency, kind: 'depends_on', to: node.task.id })
      }
    })
    if (isKnownTaskId(nodes, node.task.discovered_from)) {
      edges.push({ from: node.task.discovered_from, kind: 'discovered_from', to: node.task.id })
    }
    node.task.supersedes?.forEach((superseded) => {
      if (nodes.has(superseded)) {
        edges.push({ from: superseded, kind: 'supersedes', to: node.task.id })
      }
    })
  }
  return edges
}

function contentIndexFor(nodes: ReadonlyMap<TaskId, GraphNode>): ReadonlyMap<string, TaskId> {
  const index = new Map<string, TaskId>()
  for (const node of nodes.values()) {
    if (!index.has(node.task.content_hash ?? '')) {
      index.set(node.task.content_hash ?? '', node.task.id)
    }
  }
  return index
}

function isKnownTaskId(nodes: ReadonlyMap<TaskId, GraphNode>, value: string | undefined): value is TaskId {
  return value !== undefined && nodes.has(value as TaskId)
}

function assertKnownDependencies(graph: TaskGraph): void {
  for (const node of graph.nodes.values()) {
    node.task.depends_on.forEach((dependency) => {
      if (!graph.nodes.has(dependency)) {
        throw new Error(`task ${node.task.id} depends on unknown task ${dependency}`)
      }
    })
  }
}

function assertAcyclic(graph: TaskGraph): void {
  projectWaveView(graph)
}

function isReadyNode(graph: TaskGraph, node: GraphNode): boolean {
  return (
    node.state === 'pending' &&
    node.task.depends_on.every((dependency) => graph.nodes.get(dependency)?.state === 'closed')
  )
}

function criticalPathLengths(graph: TaskGraph): ReadonlyMap<TaskId, number> {
  const memo = new Map<TaskId, number>()
  const dependents = new Map<TaskId, TaskId[]>()
  graph.edges
    .filter((edge) => edge.kind === 'depends_on')
    .forEach((edge) => {
      dependents.set(edge.from, [...(dependents.get(edge.from) ?? []), edge.to])
    })

  const pathLength = (id: TaskId): number => {
    const cached = memo.get(id)
    if (cached !== undefined) {
      return cached
    }
    const openDependents = (dependents.get(id) ?? []).filter((dependent) => {
      const dependentNode = graph.nodes.get(dependent)
      return dependentNode !== undefined && dependentNode.state !== 'closed'
    })
    const length =
      1 +
      openDependents.reduce((longest, dependent) => Math.max(longest, pathLength(dependent)), 0)
    memo.set(id, length)
    return length
  }

  for (const id of graph.nodes.keys()) {
    pathLength(id)
  }
  return memo
}

function dependsOnDependents(graph: TaskGraph, id: TaskId): readonly TaskId[] {
  return graph.edges
    .filter((edge) => edge.kind === 'depends_on' && edge.from === id)
    .map((edge) => edge.to)
}

function firstContentOwner(nodes: ReadonlyMap<TaskId, GraphNode>, contentHash: string): TaskId | undefined {
  for (const node of nodes.values()) {
    if (node.task.content_hash === contentHash) {
      return node.task.id
    }
  }
  return undefined
}
