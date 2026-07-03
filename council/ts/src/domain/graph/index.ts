export type {
  BoundsGateInput,
  BoundsGateResult,
  CreateTaskGraphOptions,
  DiscoveredWorkBatch,
  DiscoveredWorkDuplicate,
  DiscoveredWorkResult,
  GraphEdge,
  GraphEdgeKind,
  GraphIdStrategy,
  GraphNode,
  GraphTaskDraft,
  GraphTaskState,
  TaskGraph,
} from './types.js'
export { compactClosedTasks } from './compaction.js'
export { createTaskGraph, ingestDiscoveredWork } from './graph-factory.js'
export { dispatchReadySet, markTaskState, propagateStalled } from './ready-set-policy.js'
export { planWaves, projectWaveView } from './waves.js'
export { applyBoundsGate, findOutOfBoundsFiles } from './bounds-gate.js'
