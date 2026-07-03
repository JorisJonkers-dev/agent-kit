import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { basename, dirname } from 'node:path'

import { normalizeLegacyRunDir, type LegacyRunReport, type WorkerResult } from '../adapters/fs/index.js'
import {
  coerceConfigValue,
  CONFIG_KEYS,
  parseCouncilConfig,
  resolveCouncilConfig,
  writeCouncilConfig,
} from '../domain/config/index.js'
import type { CouncilConfig, ResolvedCouncilConfig } from '../domain/config/index.js'
import type { EngineDef, JsonRecord, RunState, Task } from '../domain/contracts/index.js'
import { planWaves } from '../domain/graph/index.js'
import {
  assertTasksBijection,
  parseTasksMd,
  renderTasksMd,
  validateTasks,
} from '../domain/tasks/index.js'
import { classifyTriage, type TriageInput, type TriageVerdict } from '../domain/triage/index.js'
import type { GhPort } from '../ports/index.js'

export interface CouncilAppDeps {
  readonly gh?: GhPort
  readonly readText?: (path: string) => Promise<string>
  readonly writeText?: (path: string, text: string) => Promise<void>
}

export interface PlanInput {
  readonly brief?: string
  readonly config?: CouncilConfig
  readonly design?: boolean
  readonly runDir?: string
  readonly triage?: TriageInput
}

export interface PlanResult {
  readonly command: 'plan'
  readonly config: ResolvedCouncilConfig
  readonly designRequested: boolean
  readonly directTierPolicy: 'shrink-dag-only'
  readonly executesWorkers: false
  readonly estimatedModelCalls: number
  readonly runDir?: string
  readonly taskLimit?: number
  readonly triage?: TriageVerdict
}

export interface FanoutInput {
  readonly dryRun: boolean
  readonly github: boolean
  readonly runDir: string
}

export interface FleetInput {
  readonly agents: string
  readonly dryRun: boolean
  readonly github: boolean
  readonly tasksPath: string
}

export interface ExecutionPlan {
  readonly agents?: Readonly<Record<string, string>>
  readonly github: 'disabled' | 'dry-run' | 'created'
  readonly prUrl?: string
  readonly run: string
  readonly tasks: readonly Task[]
  readonly waves: readonly (readonly string[])[]
}

export interface RunSummary {
  readonly report?: LegacyRunReport
  readonly run: string
  readonly state: RunState
  readonly tasks: readonly Task[]
  readonly waves: readonly (readonly string[])[]
  readonly workerResults: readonly WorkerResult[]
}

export interface ConfigPaths {
  readonly project: string
  readonly user: string
}

export interface ConfigCommandInput {
  readonly action: 'show' | 'get' | 'set' | 'unset' | 'path'
  readonly key?: string
  readonly value?: string
  readonly project?: boolean
  readonly paths: ConfigPaths
}

export interface ConfigCommandResult {
  readonly config?: CouncilConfig
  readonly key?: string
  readonly paths: ConfigPaths
  readonly resolved?: ResolvedCouncilConfig
  readonly target?: string
  readonly value?: unknown
}

export interface SelfTestGolden {
  readonly agents: Readonly<Record<string, string>>
  readonly config: {
    readonly defaultIntensity: string
    readonly quickRoundOverride: number
    readonly thoroughWorker: string
  }
  readonly splitDestUrl: string
  readonly verify: {
    readonly localized: string
    readonly relative: string
  }
  readonly waves: readonly (readonly string[])[]
}

export class CouncilApp {
  private readonly gh: GhPort | undefined
  private readonly readText: (path: string) => Promise<string>
  private readonly writeText: (path: string, text: string) => Promise<void>

  constructor(deps: CouncilAppDeps = {}) {
    this.gh = deps.gh
    this.readText = deps.readText ?? ((path) => readFile(path, 'utf8'))
    this.writeText = deps.writeText ?? writeTextFile
  }

  plan(input: PlanInput = {}): Promise<PlanResult> {
    const config = resolveCouncilConfig(input.config === undefined ? {} : { flags: input.config })
    const triage = input.triage ? classifyTriage(input.triage) : undefined
    const taskLimit = triage?.route === 'direct' ? 1 : undefined
    return Promise.resolve({
      command: 'plan',
      config,
      designRequested: input.design ?? false,
      directTierPolicy: 'shrink-dag-only',
      executesWorkers: false,
      estimatedModelCalls: 2 + config.rounds * 4 + 1,
      ...(input.runDir ? { runDir: input.runDir } : {}),
      ...(taskLimit === undefined ? {} : { taskLimit }),
      ...(triage ? { triage } : {}),
    })
  }

  async fanout(input: FanoutInput): Promise<ExecutionPlan> {
    const summary = await this.status({ runDir: input.runDir })
    const github = await this.maybeCreatePullRequest(input.github, input.dryRun, summary.run)
    return {
      github: github.kind,
      ...(github.url ? { prUrl: github.url } : {}),
      run: summary.run,
      tasks: summary.tasks,
      waves: summary.waves,
    }
  }

  async fleet(input: FleetInput): Promise<ExecutionPlan> {
    const tasks = await this.readTasksJson(input.tasksPath)
    const waves = planWaves(tasks)
    const ids = tasks.map((task) => task.id)
    const agents = assignAgents(ids, parseAgentsPool(input.agents))
    const github = await this.maybeCreatePullRequest(input.github, input.dryRun, basename(input.tasksPath, '.json'))
    return {
      agents: stringifyAssignments(agents),
      github: github.kind,
      ...(github.url ? { prUrl: github.url } : {}),
      run: basename(input.tasksPath, '.json'),
      tasks,
      waves,
    }
  }

  async status(input: { readonly runDir: string }): Promise<RunSummary> {
    const normalized = await normalizeLegacyRunDir(input.runDir)
    return {
      ...(normalized.report ? { report: normalized.report } : {}),
      run: normalized.runId,
      state: normalized.state,
      tasks: normalized.tasks,
      waves: normalized.report?.waves ?? planWaves(normalized.tasks),
      workerResults: [...normalized.workerResults.values()],
    }
  }

  async readReviewPack(input: { readonly gate: '1' | 'design' | '2'; readonly runDir: string }): Promise<JsonRecord> {
    const summary = await this.status({ runDir: input.runDir })
    return {
      gate: input.gate,
      run: summary.run,
      task_count: summary.tasks.length,
      waves: summary.waves,
      worker_results: summary.workerResults.length,
    }
  }

  async config(input: ConfigCommandInput): Promise<ConfigCommandResult> {
    if (input.action === 'path') {
      return { paths: input.paths }
    }

    const user = await this.readOptionalConfig(input.paths.user)
    const project = await this.readOptionalConfig(input.paths.project)
    const target = input.project ? input.paths.project : input.paths.user
    const current = input.project ? project : user

    if (input.action === 'show') {
      return {
        config: current,
        paths: input.paths,
        resolved: resolveCouncilConfig({ project, user }),
        target,
      }
    }

    if (input.action === 'get') {
      const key = requireConfigKey(input.key)
      const resolved = resolveCouncilConfig({ project, user })
      return { key, paths: input.paths, resolved, value: resolved[key], target }
    }

    if (input.action === 'set') {
      const key = requireConfigKey(input.key)
      if (input.value === undefined) throw new Error('config set requires <key> <value>')
      const next = { ...current, [key]: coerceConfigValue(key, input.value) }
      await this.writeConfig(target, next)
      return { config: next, key, paths: input.paths, target, value: next[key] }
    }

    const key = requireConfigKey(input.key)
    const next = omitKey(current, key)
    await this.writeConfig(target, next, key)
    return { config: next, key, paths: input.paths, target }
  }

  async roundTripTasksMarkdown(tasksPath: string): Promise<readonly JsonRecord[]> {
    const tasks = await this.readTasksJson(tasksPath)
    const records = tasks as unknown as readonly JsonRecord[]
    const markdown = renderTasksMd(records)
    assertTasksBijection(records, markdown)
    return parseTasksMd(markdown)
  }

  private async readTasksJson(path: string): Promise<readonly Task[]> {
    const parsed = JSON.parse(await this.readText(path)) as unknown
    validateTasks(parsed)
    return parsed as unknown as readonly Task[]
  }

  private async maybeCreatePullRequest(
    github: boolean,
    dryRun: boolean,
    run: string,
  ): Promise<{ readonly kind: ExecutionPlan['github']; readonly url?: string }> {
    if (!github) return { kind: 'disabled' }
    if (dryRun) return { kind: 'dry-run' }
    if (!this.gh) throw new Error('--github requires a gh adapter')
    const pr = await this.gh.createPullRequest({
      body: `Council run ${run}`,
      cwd: '.',
      draft: true,
      title: `Council ${run}`,
    })
    return { kind: 'created', url: pr.url }
  }

  private async readOptionalConfig(path: string): Promise<CouncilConfig> {
    try {
      return parseCouncilConfig(await this.readText(path))
    } catch (error) {
      if (isErrno(error, 'ENOENT')) return {}
      throw error
    }
  }

  private async writeConfig(path: string, next: CouncilConfig, unsetKey?: string): Promise<void> {
    let source = ''
    try {
      source = await this.readText(path)
    } catch (error) {
      if (!isErrno(error, 'ENOENT')) throw error
    }
    const writableSource = unsetKey === undefined ? source : removeRootAssignment(source, unsetKey)
    await this.writeText(path, writeCouncilConfig(writableSource, next))
  }
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

export function extractJson(text: string): unknown {
  const fenced = /```json\s*([\s\S]*?)\s*```/u.exec(text)
  if (fenced?.[1]) return JSON.parse(fenced[1])

  const start = text.indexOf('{')
  if (start < 0) throw new Error('no JSON object found')
  for (let end = text.length; end > start; end -= 1) {
    const candidate = text.slice(start, end).trim()
    if (!candidate.endsWith('}')) continue
    try {
      return JSON.parse(candidate)
    } catch {
      continue
    }
  }
  throw new Error('no JSON object found')
}

export function renderTemplate(template: string, values: Readonly<Record<string, string>>): string {
  return Object.entries(values).reduce(
    (rendered, [key, value]) => rendered.replaceAll(`{{${key}}}`, value),
    template,
  )
}

export function splitDestUrl(owner: string, name: string): string {
  return `git@github.com:${owner}/${name}.git`
}

export function localizeVerify(command: string, repoRoot: string, worktree: string): string {
  return command.replaceAll(repoRoot, worktree)
}

export function pythonSelfTestGolden(): SelfTestGolden {
  const tasks = [
    taskForSelfTest('T1', []),
    taskForSelfTest('T2', ['T1']),
    taskForSelfTest('T3', ['T1']),
    taskForSelfTest('T4', ['T2', 'T3']),
  ]
  const config = resolveCouncilConfig({ flags: { intensity: 'quick', rounds: 5 } })
  const agents = assignAgents(['t1', 't2', 't3'], parseAgentsPool('claude:haiku,codex:gpt-5.5'))
  return {
    agents: stringifyAssignments(agents),
    config: {
      defaultIntensity: resolveCouncilConfig().intensity,
      quickRoundOverride: config.rounds,
      thoroughWorker: resolveCouncilConfig({ flags: { intensity: 'thorough' } }).worker,
    },
    splitDestUrl: splitDestUrl('o', 'n'),
    verify: {
      localized: localizeVerify('cd /workspace/services/foo && npm test', '/workspace', '/tmp/wt/T1'),
      relative: localizeVerify('npm test', '/workspace', '/tmp/wt/T1'),
    },
    waves: planWaves(tasks),
  }
}

function taskForSelfTest(id: 'T1' | 'T2' | 'T3' | 'T4', dependsOn: readonly ('T1' | 'T2' | 'T3')[]): Task {
  return {
    boundaries: 'Stay in scope',
    depends_on: dependsOn,
    difficulty: 'moderate',
    id,
    model: 'haiku',
    objective: `Task ${id}`,
    output_format: 'Code edits',
    paths: [`${id}.txt`],
    title: id,
    verify: 'npm test',
  }
}

function stringifyAssignments(assignments: ReadonlyMap<string, EngineDef>): Readonly<Record<string, string>> {
  return Object.fromEntries(
    [...assignments.entries()].map(([taskId, engine]) => [taskId, `${engine.cli}:${engine.model}`]),
  )
}

function requireConfigKey(key: string | undefined): (typeof CONFIG_KEYS)[number] {
  if (key === undefined) throw new Error('config action requires a key')
  if (!(CONFIG_KEYS as readonly string[]).includes(key)) {
    throw new Error(`unknown key ${key}; choose from ${CONFIG_KEYS.join(', ')}`)
  }
  return key as (typeof CONFIG_KEYS)[number]
}

function omitKey<T extends object>(object: T, key: string): T {
  return Object.fromEntries(Object.entries(object).filter(([k]) => k !== key)) as T
}

function removeRootAssignment(source: string, key: string): string {
  const lines = source.replace(/\r\n/gu, '\n').split('\n')
  let inTable = false
  const kept = lines.filter((line) => {
    if (/^\s*\[/.test(line)) inTable = true
    return inTable || !new RegExp(`^\\s*${escapeRegExp(key)}\\s*=`).test(line)
  })
  return kept.join('\n')
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&')
}

async function writeTextFile(path: string, text: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, text, 'utf8')
}

function isErrno(error: unknown, code: string): boolean {
  return error instanceof Error && 'code' in error && error.code === code
}
