import type { JsonRecord } from '../contracts/common.js'
import {
  isJsonRecord,
  jsonErrorMessage,
  pythonRepr,
  stableJsonStringify,
  stringifyForDisplay,
} from './task-json.js'
import { validateTasks } from './task-validator.js'

const TASK_BLOCK_RE =
  /^## (?<headerId>[^\n:]+)(?::[^\n]*)?\n<!-- council-task-id: (?<markerId>[^>]+) -->\n```json\n(?<body>.*?)\n```/gms

export interface SpecRefLike {
  readonly name: string
}

export function renderTasksMd(tasks: readonly JsonRecord[], specRef?: SpecRefLike): string {
  const featureId = specRef?.name ?? 'council'
  const header = `# Tasks: ${featureId}\n\n<!-- council-tasks-format: v1 -->`

  const lines = [header.trim(), '']
  for (const task of tasks) {
    const id = stringifyForDisplay(task.id)
    const taskTitle = stringifyForDisplay(task.title ?? id).replaceAll('\n', ' ').trim() || id
    lines.push(
      `## ${id}: ${taskTitle}`,
      `<!-- council-task-id: ${id} -->`,
      '```json',
      stableJsonStringify(task),
      '```',
      '',
    )
  }

  return `${lines.join('\n').trimEnd()}\n`
}

export function parseTasksMd(text: string): JsonRecord[] {
  const tasks: JsonRecord[] = []
  TASK_BLOCK_RE.lastIndex = 0

  for (const match of text.matchAll(TASK_BLOCK_RE)) {
    const groups = match.groups as Record<string, string | undefined>

    const headerId = groups.headerId?.trim() ?? ''
    const markerId = groups.markerId?.trim() ?? ''
    if (headerId !== markerId) {
      throw new Error(`task marker mismatch: header ${pythonRepr(headerId)}, marker ${pythonRepr(markerId)}`)
    }

    let task: unknown
    try {
      task = JSON.parse(groups.body ?? '')
    } catch (error) {
      throw new Error(`task ${pythonRepr(markerId)} JSON block is invalid: ${jsonErrorMessage(error)}`)
    }

    if (!isJsonRecord(task)) {
      throw new Error(`task ${pythonRepr(markerId)} JSON block must be an object`)
    }

    if (stringifyForDisplay(task.id).trim() !== markerId) {
      throw new Error(`task ${pythonRepr(markerId)} JSON id does not match marker`)
    }

    tasks.push(task)
  }

  if (tasks.length === 0) {
    throw new Error('no council task JSON blocks found in tasks.md')
  }

  const seen = new Set<string>()
  for (const task of tasks) {
    const taskId = stringifyForDisplay(task.id)
    if (seen.has(taskId)) {
      throw new Error(`duplicate task id in tasks.md: ${taskId}`)
    }
    seen.add(taskId)
  }

  return tasks
}

export function assertTasksBijection(tasks: readonly JsonRecord[], tasksMdText: string): void {
  const parsed = parseTasksMd(tasksMdText)
  validateTasks(parsed)
  if (normaliseTasks(parsed) !== normaliseTasks(tasks)) {
    throw new Error('tasks.md does not match tasks.json')
  }
}

function normaliseTasks(tasks: readonly JsonRecord[]): string {
  return stableJsonStringify(tasks)
}
