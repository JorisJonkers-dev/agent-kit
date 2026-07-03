import {
  expandEngineCommand,
  getEngineDef,
  parseEngineRegistryConfig,
  type EngineSpawnCommand,
} from '../contexts/engines/index.js'
import type { DagAgent, DagAgentAssignment } from '../ports/index.js'
import type { EngineDef as AssignedEngineDef, Task } from '../shared-kernel/index.js'

const DEFAULT_EFFORT = 'medium'

export interface DagWorkerCommandInput {
  readonly agent: AssignedEngineDef | DagAgent
  readonly assignment?: DagAgentAssignment
  readonly cwd?: string
  readonly effort?: string
  readonly outputFile?: string
  readonly promptFile?: string
  readonly repoRoot: string
  readonly task: Task
}

export interface DagWorkerCommandResolution {
  readonly command: EngineSpawnCommand
  readonly engine: {
    readonly name: string
    readonly model: string
  }
  readonly outputFile: string
  readonly prompt: string
  readonly promptFile: string
}

export function resolveDagWorkerCommand(input: DagWorkerCommandInput): DagWorkerCommandResolution {
  const engineName = assignedEngineName(input.agent)
  const model = assignedEngineModel(input.agent, input.assignment, input.task)
  const promptFile = input.promptFile ?? workerCommandPromptFile(input.task.id)
  const outputFile = input.outputFile ?? workerCommandOutputFile(input.task.id)
  const engine = getEngineDef(parseEngineRegistryConfig(), engineName)
  const prompt = renderDagWorkerPrompt(input.task, input.repoRoot)
  const command = expandEngineCommand({
    cwd: input.cwd ?? input.repoRoot,
    effort: input.effort ?? DEFAULT_EFFORT,
    engine,
    model,
    outputFile,
    promptFile,
  })

  return {
    command,
    engine: {
      model,
      name: engine.name,
    },
    outputFile,
    prompt,
    promptFile,
  }
}

export function renderDagWorkerPrompt(task: Task, repoRoot: string): string {
  return [
    `Task ${task.id}: ${task.title}`,
    '',
    section('Objective', task.objective),
    section('Allowed paths', bulletList(task.paths)),
    section('Boundaries', task.boundaries),
    section('Verify command', task.verify),
    section('Expected output', task.output_format),
    section(
      'Path constraint',
      `All file paths you modify or report must be repo-root-relative paths from ${repoRoot}.`,
    ),
    ...optionalListSection('Acceptance criteria', task.acceptance_criteria),
    ...optionalListSection('Success criteria', task.success_criteria),
    ...optionalListSection('Verify proves', task.verify_proves),
    ...optionalListSection('Failure modes', task.failure_modes),
    ...optionalTextSection('Developer notes', task.dev_notes),
  ].join('\n')
}

export function workerCommandPromptFile(taskId: Task['id']): string {
  return `.council/workers/${taskId}/prompt.md`
}

export function workerCommandOutputFile(taskId: Task['id']): string {
  return `.council/workers/${taskId}/output.json`
}

function section(title: string, body: string): string {
  return `${title}:\n${body}`
}

function bulletList(items: readonly string[]): string {
  return items.map((item) => `- ${item}`).join('\n')
}

function optionalListSection(title: string, items: readonly string[] | undefined): readonly string[] {
  return items === undefined || items.length === 0 ? [] : [section(title, bulletList(items))]
}

function optionalTextSection(title: string, text: string | undefined): readonly string[] {
  return text === undefined ? [] : [section(title, text)]
}

function assignedEngineName(agent: AssignedEngineDef | DagAgent): string {
  return 'cli' in agent ? agent.cli : agent.kind
}

function assignedEngineModel(
  agent: AssignedEngineDef | DagAgent,
  assignment: DagAgentAssignment | undefined,
  task: Task,
): string {
  if ('cli' in agent) return agent.model
  if (task.engine?.cli === agent.kind) return task.engine.model
  return assignment?.model ?? agent.model ?? task.model
}
