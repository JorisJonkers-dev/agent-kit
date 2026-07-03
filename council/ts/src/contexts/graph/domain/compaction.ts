import type { Task, TaskId } from '../../../domain/contracts/task.js'

import { graphFromNodes } from './graph-factory.js'
import type { GraphNode, TaskGraph } from './types.js'

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
