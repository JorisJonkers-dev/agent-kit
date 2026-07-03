import type { TaskId } from '../../../shared-kernel/task.js'

import { createTaskGraph } from './graph-factory.js'
import type { CreateTaskGraphOptions, GraphTaskDraft, TaskGraph } from './types.js'

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
