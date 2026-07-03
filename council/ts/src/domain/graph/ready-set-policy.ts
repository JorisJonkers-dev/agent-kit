import type { TaskId } from '../contracts/task.js'

import { graphFromNodes } from './graph-factory.js'
import type { GraphNode, GraphTaskState, TaskGraph } from './types.js'

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
