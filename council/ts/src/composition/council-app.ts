import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'

import { normalizeLegacyRunDir } from '../adapters/runstore/index.js'
import { resolveCouncilConfig } from '../contexts/config/index.js'
import { planWaves } from '../contexts/graph/index.js'
import { recommendLenses, type LensProblemProfile, type LensRecommendation } from '../contexts/triage/index.js'
import type { GhPort } from '../ports/index.js'
import type { Task } from '../shared-kernel/index.js'
import {
  assignAgents,
  configWorkflow,
  fanoutWorkflow,
  fleetWorkflow,
  parseAgentsPool,
  planWorkflow,
  reviewPackWorkflow,
  roundTripTasksMarkdownWorkflow,
  statusWorkflow,
  stringifyAssignments,
} from '../workflows/index.js'
import type {
  ConfigCommandInput,
  ConfigCommandResult,
  ExecutionPlan,
  FanoutInput,
  FleetInput,
  PlanInput,
  PlanResult,
  ReviewPackInput,
  RunSummary,
} from '../workflows/index.js'

export interface CouncilAppDeps {
  readonly gh?: GhPort
  readonly readText?: (path: string) => Promise<string>
  readonly writeText?: (path: string, text: string) => Promise<void>
}

export interface RecommendInput {
  readonly profile?: LensProblemProfile
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
    return Promise.resolve(planWorkflow(input))
  }

  fanout(input: FanoutInput): Promise<ExecutionPlan> {
    return fanoutWorkflow(input, {
      createPullRequest: (run) => this.createPullRequest(run),
      status: (statusInput) => this.status(statusInput),
    })
  }

  fleet(input: FleetInput): Promise<ExecutionPlan> {
    return fleetWorkflow(input, {
      createPullRequest: (run) => this.createPullRequest(run),
      readText: this.readText,
    })
  }

  status(input: { readonly runDir: string }): Promise<RunSummary> {
    return statusWorkflow(input, {
      normalizeRunDir: normalizeLegacyRunDir,
    })
  }

  readReviewPack(input: ReviewPackInput): Promise<import('../shared-kernel/index.js').JsonRecord> {
    return reviewPackWorkflow(input, {
      status: (statusInput) => this.status(statusInput),
    })
  }

  config(input: ConfigCommandInput): Promise<ConfigCommandResult> {
    return configWorkflow(input, {
      readText: this.readText,
      writeText: this.writeText,
    })
  }

  recommend(input: RecommendInput = {}): Promise<LensRecommendation> {
    return Promise.resolve(recommendLenses(input.profile))
  }

  roundTripTasksMarkdown(tasksPath: string): Promise<readonly import('../shared-kernel/index.js').JsonRecord[]> {
    return roundTripTasksMarkdownWorkflow(tasksPath, { readText: this.readText })
  }

  private async createPullRequest(run: string): Promise<string> {
    if (!this.gh) throw new Error('--github requires a gh adapter')
    const pr = await this.gh.createPullRequest({
      body: `Council run ${run}`,
      cwd: '.',
      draft: true,
      title: `Council ${run}`,
    })
    return pr.url
  }
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

async function writeTextFile(path: string, text: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, text, 'utf8')
}
