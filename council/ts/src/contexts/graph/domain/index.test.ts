import { describe, expect, it } from 'vitest'

import type { TaskId } from '../../../shared-kernel/task.js'

import {
  applyBoundsGate,
  compactClosedTasks,
  createTaskGraph,
  dispatchReadySet,
  ingestDiscoveredWork,
  markTaskState,
  planWaves,
  projectWaveView,
  propagateStalled,
  type GraphNode,
  type GraphTaskDraft,
  type TaskGraph,
} from './index.js'

function task(overrides: Partial<GraphTaskDraft> & Pick<GraphTaskDraft, 'objective'>): GraphTaskDraft {
  return {
    boundaries: 'same package',
    difficulty: 'moderate',
    model: 'haiku',
    output_format: 'patch',
    paths: ['council/ts/src/domain/graph/index.ts'],
    title: overrides.objective,
    verify: 'npm test',
    ...overrides,
  }
}

describe('task graph', () => {
  it('mints stable legacy ids, computes content hashes, and rejects invalid static graphs', () => {
    const graph = createTaskGraph([
      task({ id: 'T2', objective: 'first' }),
      task({ objective: 'second' }),
      task({ content_hash: 'same', id: 'ck-known', objective: 'third' }),
      task({ content_hash: 'same', objective: 'fourth' }),
    ])

    expect([...graph.nodes.keys()]).toEqual(['T2', 'T3', 'ck-known', 'T4'])
    expect(graph.nextOrdinal).toBe(5)
    expect(graph.contentIndex.get('same')).toBe('ck-known')
    expect(graph.nodes.get('T3')?.task.content_hash).toHaveLength(64)

    const closed = markTaskState(graph, 'T3', 'closed')
    expect(closed.nodes.get('T3')?.task.id).toBe('T3')
    expect(closed.nodes.get('T3')?.state).toBe('closed')

    expect(() => markTaskState(graph, 'T9', 'closed')).toThrow('unknown task id: T9')
    expect(() =>
      createTaskGraph([task({ id: 'T1', objective: 'one' }), task({ id: 'T1', objective: 'two' })]),
    ).toThrow('duplicate task id: T1')
    expect(() => createTaskGraph([task({ depends_on: ['T9'], objective: 'bad dep' })])).toThrow(
      'depends on unknown task T9',
    )
    expect(() =>
      createTaskGraph([
        task({ depends_on: ['T2'], id: 'T1', objective: 'one' }),
        task({ depends_on: ['T1'], id: 'T2', objective: 'two' }),
      ]),
    ).toThrow('dependency cycle among tasks: T1, T2')
  })

  it('can mint hash-shaped ids without coupling them to content hashes', () => {
    const graph = createTaskGraph(
      [
        task({ content_hash: 'abcdef123456', objective: 'one' }),
        task({ content_hash: 'abcdef123456', objective: 'duplicate content is still a task' }),
        task({ content_hash: 'abcdef1234567890', objective: 'long collision' }),
      ],
      { idStrategy: 'content-hash' },
    )

    expect([...graph.nodes.keys()]).toEqual([
      'ck-abcdef123456',
      'ck-abcdef123456-2',
      'ck-abcdef1234567',
    ])
    expect(graph.nodes.get('ck-abcdef123456')?.task.content_hash).toBe('abcdef123456')
  })

  it('materializes typed edges and legacy wave projections', () => {
    const graph = createTaskGraph([
      task({ id: 'T1', objective: 'base' }),
      task({
        depends_on: ['T1'],
        discovered_from: 'T1',
        id: 'T2',
        objective: 'child',
        supersedes: ['T1'],
      }),
      task({ discovered_from: 'external-note', id: 'T3', objective: 'unlinked discovery' }),
    ])

    expect(graph.edges).toEqual([
      { from: 'T1', kind: 'depends_on', to: 'T2' },
      { from: 'T1', kind: 'discovered_from', to: 'T2' },
      { from: 'T1', kind: 'supersedes', to: 'T2' },
    ])
    expect(projectWaveView(graph)).toEqual([['T1', 'T3'], ['T2']])
    expect(
      planWaves([
        task({ id: 'T1', objective: 'base' }),
        task({ depends_on: ['T1'], id: 'T2', objective: 'child' }),
      ]),
    ).toEqual([['T1'], ['T2']])
  })

  it('dispatches a critical-path-first ready set', () => {
    const graph = createTaskGraph([
      task({ id: 'T1', objective: 'long root' }),
      task({ id: 'T2', objective: 'short root' }),
      task({ depends_on: ['T1'], id: 'T3', objective: 'middle' }),
      task({ depends_on: ['T3'], id: 'T4', objective: 'leaf' }),
    ])

    expect(dispatchReadySet(graph)).toEqual(['T1', 'T2'])
    expect(dispatchReadySet(graph, 1)).toEqual(['T1'])

    const afterRoot = markTaskState(graph, 'T1', 'closed')
    expect(dispatchReadySet(afterRoot)).toEqual(['T3', 'T2'])

    const running = markTaskState(afterRoot, 'T2', 'running')
    expect(dispatchReadySet(running)).toEqual(['T3'])

    const equalPathGraph = createTaskGraph([
      task({ id: 'T1', objective: 'left' }),
      task({ id: 'T2', objective: 'right' }),
    ])
    expect(dispatchReadySet(equalPathGraph)).toEqual(['T1', 'T2'])
    const left = equalPathGraph.nodes.get('T1')
    const right = equalPathGraph.nodes.get('T2')
    expect(left).toBeDefined()
    expect(right).toBeDefined()
    if (left === undefined || right === undefined) {
      throw new Error('expected graph nodes')
    }
    const sameOrderGraph: TaskGraph = {
      ...equalPathGraph,
      nodes: new Map<TaskId, GraphNode>([
        ['T2', { ...right, order: 0 }],
        ['T1', left],
      ]),
    }
    expect(dispatchReadySet(sameOrderGraph)).toEqual(['T1', 'T2'])
  })

  it('propagates stalled work to dependent pending and running tasks', () => {
    const graph = createTaskGraph([
      task({ id: 'T1', objective: 'root' }),
      task({ depends_on: ['T1'], id: 'T2', objective: 'dependent' }),
      task({ depends_on: ['T2'], id: 'T3', objective: 'transitive' }),
      task({ depends_on: ['T1'], id: 'T4', objective: 'already closed' }),
      task({ depends_on: ['T1'], id: 'T5', objective: 'already stalled' }),
    ])
    const withClosed = markTaskState(graph, 'T4', 'closed')
    const withRunning = markTaskState(withClosed, 'T3', 'running')
    const withStalled = markTaskState(withRunning, 'T5', 'stalled')
    const propagated = propagateStalled(withStalled, 'T1')

    expect(propagated.nodes.get('T1')?.state).toBe('stalled')
    expect(propagated.nodes.get('T2')?.state).toBe('blocked')
    expect(propagated.nodes.get('T2')?.blocked_by).toEqual(['T1'])
    expect(propagated.nodes.get('T3')?.state).toBe('blocked')
    expect(propagated.nodes.get('T3')?.blocked_by).toEqual(['T2'])
    expect(propagated.nodes.get('T4')?.state).toBe('closed')
    expect(propagated.nodes.get('T5')?.state).toBe('stalled')
    expect(() => propagateStalled(graph, 'T9')).toThrow('unknown task id: T9')
  })

  it('ingests discovered work, dedups by content hash, and validates the added graph', () => {
    const source = markTaskState(createTaskGraph([task({ id: 'T1', objective: 'source' })]), 'T1', 'closed')
    const result = ingestDiscoveredWork(source, {
      sourceId: 'T1',
      tasks: [
        task({ content_hash: 'fresh', depends_on: ['T1'], objective: 'fresh work', supersedes: ['T1'] }),
        task({ content_hash: 'fresh', objective: 'duplicate work' }),
      ],
    })

    expect(result.ingested).toEqual(['T2'])
    expect(result.deduped).toEqual([{ content_hash: 'fresh', existing_id: 'T2' }])
    expect(result.graph.edges).toEqual([
      { from: 'T1', kind: 'depends_on', to: 'T2' },
      { from: 'T1', kind: 'discovered_from', to: 'T2' },
      { from: 'T1', kind: 'supersedes', to: 'T2' },
    ])

    const sourceContentHash = source.nodes.get('T1')?.task.content_hash
    expect(sourceContentHash).toBeDefined()
    if (sourceContentHash === undefined) {
      throw new Error('expected source content hash')
    }
    const duplicateIdSameContent = ingestDiscoveredWork(source, {
      sourceId: 'T1',
      tasks: [task({ content_hash: sourceContentHash, id: 'T1', objective: 'source' })],
    })
    expect(duplicateIdSameContent.deduped[0]?.existing_id).toBe('T1')

    expect(() => ingestDiscoveredWork(source, { sourceId: 'T9', tasks: [] })).toThrow(
      'unknown discovery source: T9',
    )
    expect(() =>
      ingestDiscoveredWork(source, {
        sourceId: 'T1',
        tasks: [task({ content_hash: 'different', id: 'T1', objective: 'bad duplicate id' })],
      }),
    ).toThrow('duplicate task id: T1')
    expect(() =>
      ingestDiscoveredWork(source, {
        sourceId: 'T1',
        tasks: [task({ depends_on: ['T9'], objective: 'bad dependency' })],
      }),
    ).toThrow('depends on unknown task T9')
  })

  it('compacts closed tasks and treats their dependencies as already satisfied', () => {
    const graph = createTaskGraph([
      task({ id: 'T1', objective: 'done' }),
      task({ depends_on: ['T1'], id: 'T2', objective: 'next' }),
      task({ depends_on: ['T2'], id: 'T3', objective: 'later' }),
    ])
    const closed = markTaskState(graph, 'T1', 'closed')
    const compacted = compactClosedTasks(closed)

    expect([...compacted.nodes.keys()]).toEqual(['T2', 'T3'])
    expect(compacted.nodes.get('T2')?.task.depends_on).toEqual([])
    expect(compacted.edges).toEqual([{ from: 'T2', kind: 'depends_on', to: 'T3' }])
    expect(dispatchReadySet(compacted)).toEqual(['T2'])
  })
})

describe('bounds gate', () => {
  it('ports the out-of-bounds status gate with a story allowlist', () => {
    expect(
      applyBoundsGate({
        allowedPaths: ['src/a.ts'],
        filesChanged: ['src/a.ts', 'workers/T7/story.md'],
        status: 'ok',
        taskId: 'T7',
      }),
    ).toEqual({
      files_changed: ['src/a.ts', 'workers/T7/story.md'],
      out_of_bounds: [],
      status: 'ok',
    })

    expect(applyBoundsGate({ allowedPaths: [], filesChanged: [], taskId: 'T1' }).status).toBe('ok')

    expect(
      applyBoundsGate({
        allowedPaths: ['src/a.ts'],
        filesChanged: ['src/a.ts', 'src/b.ts', 'workers/T8/story.md'],
        taskId: 'T7',
      }),
    ).toEqual({
      files_changed: ['src/a.ts', 'src/b.ts', 'workers/T8/story.md'],
      out_of_bounds: ['src/b.ts', 'workers/T8/story.md'],
      status: 'out-of-bounds',
    })
  })
})

describe('pure projections', () => {
  it('throws if a supplied graph view is cyclic', () => {
    const cyclicNodeGraph = createTaskGraph([task({ id: 'T1', objective: 'placeholder' })])
    const node = cyclicNodeGraph.nodes.get('T1')
    expect(node).toBeDefined()
    if (node === undefined) {
      throw new Error('expected graph node')
    }
    const nodes = new Map<TaskId, GraphNode>([
      ['T1', { ...node, task: { ...node.task, depends_on: ['T1'] } }],
    ])
    const cyclicGraph: TaskGraph = { ...cyclicNodeGraph, nodes }

    expect(() => projectWaveView(cyclicGraph)).toThrow('dependency cycle among tasks: T1')
  })
})
