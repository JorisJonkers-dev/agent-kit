import { createHash } from 'node:crypto'

import type { Task, TaskId } from '../../../domain/contracts/task.js'

import type {
  CreateTaskGraphOptions,
  DiscoveredWorkBatch,
  DiscoveredWorkDuplicate,
  DiscoveredWorkResult,
  GraphEdge,
  GraphIdStrategy,
  GraphNode,
  GraphTaskDraft,
  TaskGraph,
} from './types.js'
import { projectWaveView } from './waves.js'

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

export function graphFromNodes(
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

function firstContentOwner(nodes: ReadonlyMap<TaskId, GraphNode>, contentHash: string): TaskId | undefined {
  for (const node of nodes.values()) {
    if (node.task.content_hash === contentHash) {
      return node.task.id
    }
  }
  return undefined
}
