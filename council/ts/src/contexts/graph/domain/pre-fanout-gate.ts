import type { TaskId } from '../../../shared-kernel/task.js'

import type { TaskGraph } from './types.js'
import { projectWaveView } from './waves.js'

type VerifyViolationKind =
  | 'empty-verify'
  | 'placeholder-verify'
  | 'non-proving-verify'
  | 'destructive-command'

export type PreFanoutGateViolationKind =
  | VerifyViolationKind
  | 'absolute-task-path'
  | 'missing-task-path'
  | 'same-wave-path-overlap'

export interface PreFanoutGateInput {
  readonly graph: TaskGraph
  readonly repoFiles: ReadonlySet<string>
}

export interface PreFanoutGateViolation {
  readonly kind: PreFanoutGateViolationKind
  readonly message: string
  readonly otherPath?: string
  readonly otherTaskId?: TaskId
  readonly path?: string
  readonly taskId: TaskId
  readonly verify?: string
  readonly wave?: number
}

export interface PreFanoutGateResult {
  readonly ok: boolean
  readonly violations: readonly PreFanoutGateViolation[]
  readonly waves: readonly (readonly TaskId[])[]
}

interface TaskPathEntry {
  readonly normalized: string
  readonly original: string
}

const PLACEHOLDER_VERIFY_COMMANDS = new Set([
  'manual',
  'n a',
  'na',
  'no verify',
  'none',
  'not applicable',
  'not needed',
  'placeholder',
  'skip',
  'skipped',
  'tbd',
  'todo',
  'verify manually',
])

const DESTRUCTIVE_COMMAND_PATTERNS: readonly RegExp[] = [
  /\brm\s+-[^\n;&|]*[rf][^\n;&|]*[rf](?:\s|$)/u,
  /\bgit\s+reset\s+--hard(?:\s|$)/u,
  /\bgit\s+clean\s+-[^\n;&|]*[fdx][^\n;&|]*[fdx](?:\s|$)/u,
  /\bgit\s+checkout\s+--\s+\S+/u,
  /\bfind\s+.+\s+-delete(?:\s|$)/u,
  /\bxargs\s+rm(?:\s|$)/u,
  /\bmkfs(?:\.[a-z0-9_-]+)?(?:\s|$)/u,
]

const PROVING_COMMAND_PATTERNS: readonly RegExp[] = [
  /^(?:npm|pnpm|yarn|bun)\s+(?:test|run\s+(?:test|typecheck|lint|eslint|build|check|depcruise)(?::[a-z0-9_-]+)?)(?:\s|$)/u,
  /^(?:npx\s+)?(?:vitest|jest|mocha|ava|tap|playwright|eslint|tsc|depcruise|dependency-cruiser)(?:\s|$)/u,
  /^uv\s+run\s+(?:python\s+(?:-m\s+)?(?:pytest|unittest|scripts?\/\S+)|pytest|ruff|mypy|pyright)(?:\s|$)/u,
  /^(?:pytest|ruff|mypy|pyright)(?:\s|$)/u,
  /^go\s+test(?:\s|$)/u,
  /^cargo\s+(?:test|clippy|check)(?:\s|$)/u,
  /^(?:dotnet|mvn)\s+(?:test|verify|check|build)(?:\s|$)/u,
  /^(?:gradle|gradlew|\.\/gradlew)\s+(?:test|check|build)(?:\s|$)/u,
  /^test\s+-(?:e|f|s|d)\s+\S+/u,
  /^(?:rg|grep)\s+.+\s+\S+/u,
]

export function applyPreFanoutGate(input: PreFanoutGateInput): PreFanoutGateResult {
  const waves = projectWaveView(input.graph)
  const repoFiles = normalizedRepoFiles(input.repoFiles)
  const violations = [
    ...findTaskPolicyViolations(input.graph, repoFiles),
    ...findSameWavePathOverlaps(input.graph, waves),
  ]
  return { ok: violations.length === 0, violations, waves }
}

function findTaskPolicyViolations(
  graph: TaskGraph,
  repoFiles: ReadonlySet<string>,
): readonly PreFanoutGateViolation[] {
  const violations: PreFanoutGateViolation[] = []
  for (const node of graph.nodes.values()) {
    const verifyKind = verifyViolationKind(node.task.verify)
    if (verifyKind !== undefined) {
      violations.push({
        kind: verifyKind,
        message: verifyViolationMessage(verifyKind, node.task.id),
        taskId: node.task.id,
        verify: node.task.verify,
      })
    }

    for (const path of node.task.paths) {
      if (isAbsoluteTaskPath(path)) {
        violations.push({
          kind: 'absolute-task-path',
          message: `task ${node.task.id} declares absolute path ${path}`,
          path,
          taskId: node.task.id,
        })
        continue
      }

      const normalized = normalizeTaskPath(path)
      if (normalized.length > 0 && !repoFiles.has(normalized)) {
        violations.push({
          kind: 'missing-task-path',
          message: `task ${node.task.id} declares path ${path} that is absent from the repo file set`,
          path,
          taskId: node.task.id,
        })
      }
    }
  }
  return violations
}

function findSameWavePathOverlaps(
  graph: TaskGraph,
  waves: readonly (readonly TaskId[])[],
): readonly PreFanoutGateViolation[] {
  const pathEntriesByTask = pathEntriesForGraph(graph)
  const violations: PreFanoutGateViolation[] = []

  waves.forEach((wave, waveIndex) => {
    for (const [leftIndex, leftTaskId] of wave.entries()) {
      const leftPaths = pathEntriesByTask.get(leftTaskId) ?? []
      for (const rightTaskId of wave.slice(leftIndex + 1)) {
        const rightPaths = pathEntriesByTask.get(rightTaskId) ?? []
        collectPathOverlaps(leftTaskId, leftPaths, rightTaskId, rightPaths, waveIndex, violations)
      }
    }
  })

  return violations
}

function collectPathOverlaps(
  leftTaskId: TaskId,
  leftPaths: readonly TaskPathEntry[],
  rightTaskId: TaskId,
  rightPaths: readonly TaskPathEntry[],
  wave: number,
  violations: PreFanoutGateViolation[],
): void {
  for (const leftPath of leftPaths) {
    for (const rightPath of rightPaths) {
      if (pathsIntersect(leftPath.normalized, rightPath.normalized)) {
        violations.push({
          kind: 'same-wave-path-overlap',
          message: `tasks ${leftTaskId} and ${rightTaskId} both declare ${leftPath.original} in ready wave ${String(wave)}`,
          otherPath: rightPath.original,
          otherTaskId: rightTaskId,
          path: leftPath.original,
          taskId: leftTaskId,
          wave,
        })
      }
    }
  }
}

function pathEntriesForGraph(graph: TaskGraph): ReadonlyMap<TaskId, readonly TaskPathEntry[]> {
  const entries = new Map<TaskId, readonly TaskPathEntry[]>()
  for (const node of graph.nodes.values()) {
    entries.set(node.task.id, pathEntriesForTask(node.task.paths))
  }
  return entries
}

function pathEntriesForTask(paths: readonly string[]): readonly TaskPathEntry[] {
  const entries: TaskPathEntry[] = []
  const seen = new Set<string>()
  for (const path of paths) {
    const normalized = normalizeTaskPath(path)
    if (normalized.length === 0 || seen.has(normalized)) {
      continue
    }
    seen.add(normalized)
    entries.push({ normalized, original: path })
  }
  return entries
}

function normalizedRepoFiles(repoFiles: ReadonlySet<string>): ReadonlySet<string> {
  return new Set([...repoFiles].map(normalizeTaskPath).filter(Boolean))
}

function normalizeTaskPath(path: string): string {
  return path
    .trim()
    .replaceAll('\\', '/')
    .split('/')
    .filter((part) => part.length > 0 && part !== '.')
    .join('/')
}

function pathsIntersect(left: string, right: string): boolean {
  return left === right || left.startsWith(`${right}/`) || right.startsWith(`${left}/`)
}

function isAbsoluteTaskPath(path: string): boolean {
  const trimmed = path.trim()
  return trimmed.startsWith('/') || /^[a-z]:[\\/]/iu.test(trimmed) || trimmed.startsWith('\\\\')
}

function verifyViolationKind(verify: string): VerifyViolationKind | undefined {
  const command = verify.trim()
  if (command.length === 0) {
    return 'empty-verify'
  }
  if (isPlaceholderVerifyCommand(command)) {
    return 'placeholder-verify'
  }
  if (isDestructiveCommand(command)) {
    return 'destructive-command'
  }
  if (!isProvingVerifyCommand(command)) {
    return 'non-proving-verify'
  }
  return undefined
}

function verifyViolationMessage(kind: VerifyViolationKind, taskId: TaskId): string {
  switch (kind) {
    case 'empty-verify':
      return `task ${taskId} has an empty verify command`
    case 'placeholder-verify':
      return `task ${taskId} has a placeholder verify command`
    case 'non-proving-verify':
      return `task ${taskId} verify command does not prove the task result`
    case 'destructive-command':
      return `task ${taskId} verify command contains a destructive shell command`
  }
}

function isPlaceholderVerifyCommand(command: string): boolean {
  const normalized = command
    .toLowerCase()
    .replace(/[._-]+/gu, ' ')
    .replace(/\s+/gu, ' ')
    .trim()
  return (
    PLACEHOLDER_VERIFY_COMMANDS.has(normalized) ||
    /^<[^>]+>$/u.test(command) ||
    /^\{[^}]+\}$/u.test(command)
  )
}

function isDestructiveCommand(command: string): boolean {
  const normalized = command.toLowerCase()
  return DESTRUCTIVE_COMMAND_PATTERNS.some((pattern) => pattern.test(normalized))
}

function isProvingVerifyCommand(command: string): boolean {
  return commandSegments(command).some((segment) =>
    PROVING_COMMAND_PATTERNS.some((pattern) => pattern.test(segment)),
  )
}

function commandSegments(command: string): readonly string[] {
  return command
    .toLowerCase()
    .split(/&&|\|\||;/u)
    .map((segment) => stripSegmentPrefix(segment))
    .filter(Boolean)
}

function stripSegmentPrefix(segment: string): string {
  return segment.trim().replace(/^(?:env\s+)?(?:[a-z_][a-z0-9_]*=[^\s]+\s+)*/u, '')
}
