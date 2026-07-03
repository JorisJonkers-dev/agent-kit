import { basename } from 'node:path'

import { applyPreFanoutGate, createTaskGraph } from '../contexts/graph/index.js'
import { assertTasksBijection, parseTasksMd, renderTasksMd, validateTasks } from '../contexts/tasks/index.js'
import type { EngineDef, JsonRecord, Task } from '../shared-kernel/index.js'
import { assertPreFanoutGatePassed, repoFilesForGate, type ExecutionPlan } from './fanout.js'

export interface FleetInput {
  readonly agents: string
  readonly dryRun: boolean
  readonly github: boolean
  readonly repoFiles?: ReadonlySet<string>
  readonly tasksPath: string
}

export interface FleetWorkflowDeps {
  readonly createPullRequest: (run: string) => Promise<string>
  readonly readText: (path: string) => Promise<string>
}

export async function fleetWorkflow(input: FleetInput, deps: FleetWorkflowDeps): Promise<ExecutionPlan> {
  const tasks = await readTasksJson(input.tasksPath, deps.readText)
  const gate = applyPreFanoutGate({
    graph: createTaskGraph(tasks),
    repoFiles: repoFilesForGate(tasks, input.repoFiles),
  })
  assertPreFanoutGatePassed(gate.violations)
  const ids = tasks.map((task) => task.id)
  const agents = assignAgents(ids, parseAgentsPool(input.agents))
  const run = basename(input.tasksPath, '.json')
  const github = await resolveGithub(input.github, input.dryRun, run, deps.createPullRequest)
  return {
    agents: stringifyAssignments(agents),
    github: github.kind,
    ...(github.url ? { prUrl: github.url } : {}),
    run,
    tasks,
    waves: gate.waves,
  }
}

export async function roundTripTasksMarkdownWorkflow(
  tasksPath: string,
  deps: Pick<FleetWorkflowDeps, 'readText'>,
): Promise<readonly JsonRecord[]> {
  const tasks = await readTasksJson(tasksPath, deps.readText)
  const records = tasks as unknown as readonly JsonRecord[]
  const markdown = renderTasksMd(records)
  assertTasksBijection(records, markdown)
  return parseTasksMd(markdown)
}

export function parseEngineSpec(spec: string): EngineDef {
  const [cli, ...rest] = spec.split(':')
  const model = rest.join(':')
  if ((cli !== 'claude' && cli !== 'codex') || model.trim().length === 0) {
    throw new Error(`engine must be claude:<model> or codex:<model>, got ${JSON.stringify(spec)}`)
  }
  return { cli, label: `${cli}:${model}`, model }
}

export function parseAgentsPool(spec: string): readonly EngineDef[] {
  if (spec.trim().length === 0) throw new Error('agents pool must not be empty')
  return spec.split(',').flatMap((part) => {
    const pieces = part.trim().split('*')
    if (pieces.length > 2) throw new Error(`malformed agent spec ${JSON.stringify(part)}`)
    const [engineRaw, countRaw] = pieces
    if (!engineRaw) throw new Error(`malformed agent spec ${JSON.stringify(part)}`)
    const engine = parseEngineSpec(engineRaw)
    const count = countRaw === undefined ? 1 : Number.parseInt(countRaw, 10)
    if (!Number.isInteger(count) || count < 1 || String(count) !== String(countRaw ?? 1)) {
      throw new Error(`agent count must be a positive integer in ${JSON.stringify(part)}`)
    }
    return Array.from({ length: count }, () => engine)
  })
}

export function assignAgents(
  taskIds: readonly string[],
  agents: readonly EngineDef[],
): ReadonlyMap<string, EngineDef> {
  const [head, ...tail] = agents
  if (head === undefined) throw new Error('agents pool must not be empty')
  const pool = [head, ...tail]
  return new Map(taskIds.map((taskId, index) => [taskId, pool[index % pool.length] ?? head]))
}

export function stringifyAssignments(assignments: ReadonlyMap<string, EngineDef>): Readonly<Record<string, string>> {
  return Object.fromEntries(
    [...assignments.entries()].map(([taskId, engine]) => [taskId, `${engine.cli}:${engine.model}`]),
  )
}

async function readTasksJson(path: string, readText: (path: string) => Promise<string>): Promise<readonly Task[]> {
  const parsed = JSON.parse(await readText(path)) as unknown
  validateTasks(parsed)
  return parsed as unknown as readonly Task[]
}

async function resolveGithub(
  github: boolean,
  dryRun: boolean,
  run: string,
  createPullRequest: (run: string) => Promise<string>,
): Promise<{ readonly kind: ExecutionPlan['github']; readonly url?: string }> {
  if (!github) return { kind: 'disabled' }
  if (dryRun) return { kind: 'dry-run' }
  return { kind: 'created', url: await createPullRequest(run) }
}
