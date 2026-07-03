import type { JsonRecord, JsonValue } from '../../../shared-kernel/common.js'
import { TASK_ATTACHMENT_ACTIVE_SKILLS_MAX } from '../../../shared-kernel/index.js'
import {
  comparableKey,
  formatPythonList,
  isJsonArray,
  isJsonRecord,
  pythonRepr,
  stringifyForDisplay,
} from './task-json.js'

const REQUIRED_VALIDATE_FIELDS = ['id', 'objective', 'depends_on', 'paths', 'model', 'verify'] as const

export interface ValidateTasksOptions {
  readonly onWarning?: (message: string) => void
}

export function validateTasks(tasks: unknown, options: ValidateTasksOptions = {}): asserts tasks is JsonRecord[] {
  if (!Array.isArray(tasks) || tasks.length === 0) {
    throw new Error('consolidator returned no tasks')
  }

  const seen = new Set<string>()
  for (const task of tasks) {
    if (!isJsonRecord(task)) {
      throw new Error('task ? must be an object')
    }

    const missing = REQUIRED_VALIDATE_FIELDS.filter((field) => !(field in task))
    if (missing.length > 0) {
      throw new Error(`task ${stringifyForDisplay(task.id ?? '?')} missing fields: ${formatPythonList(missing)}`)
    }

    const taskIdKey = comparableKey(task.id)
    if (seen.has(taskIdKey)) {
      throw new Error(`duplicate task id: ${stringifyForDisplay(task.id)}`)
    }
    seen.add(taskIdKey)

    if (!stringifyForDisplay(task.verify).trim()) {
      options.onWarning?.(
        `warning: task ${stringifyForDisplay(task.id)} has no verify command - its result is unchecked except by the adversarial verifier`,
      )
    }

    assertTaskAttachment(task)
  }

  assertTaskDag(tasks)
}

function assertTaskAttachment(task: JsonRecord): void {
  if (task.attachment === undefined) {
    return
  }

  const taskLabel = pythonRepr(stringifyForDisplay(task.id))
  if (!isJsonRecord(task.attachment)) {
    throw new Error(`task ${taskLabel} attachment must be an object`)
  }

  if (!('mcpProfile' in task.attachment)) {
    throw new Error(`task ${taskLabel} attachment.mcpProfile is required`)
  }
  if (typeof task.attachment.mcpProfile !== 'string' || task.attachment.mcpProfile.trim().length === 0) {
    throw new Error(`task ${taskLabel} attachment.mcpProfile must be a non-empty string`)
  }

  if (!('activeSkills' in task.attachment)) {
    throw new Error(`task ${taskLabel} attachment.activeSkills is required`)
  }
  const activeSkills = task.attachment.activeSkills
  if (!isJsonArray(activeSkills) || activeSkills.some((skill) => typeof skill !== 'string')) {
    throw new Error(`task ${taskLabel} attachment.activeSkills must be an array of strings`)
  }
  if (activeSkills.length > TASK_ATTACHMENT_ACTIVE_SKILLS_MAX) {
    throw new Error(
      `task ${taskLabel} attachment.activeSkills must contain at most ${String(TASK_ATTACHMENT_ACTIVE_SKILLS_MAX)} skills`,
    )
  }
}

function assertTaskDag(tasks: readonly JsonRecord[]): void {
  const ids = new Set(tasks.map((task) => comparableKey(task.id)))
  const deps = new Map<string, readonly JsonValue[]>()
  const idLabels = new Map<string, string>()

  for (const task of tasks) {
    const taskIdKey = comparableKey(task.id)
    if (!isJsonArray(task.depends_on)) {
      throw new Error(`task ${pythonRepr(stringifyForDisplay(task.id))} depends_on must be an array`)
    }

    deps.set(taskIdKey, task.depends_on)
    idLabels.set(taskIdKey, stringifyForDisplay(task.id))

    for (const dep of task.depends_on) {
      const depKey = comparableKey(dep)
      if (!ids.has(depKey)) {
        throw new Error(
          `task ${pythonRepr(stringifyForDisplay(task.id))} depends on unknown task ${pythonRepr(stringifyForDisplay(dep))}`,
        )
      }
    }
  }

  const remaining = new Map(deps)
  const done = new Set<string>()

  while (remaining.size > 0) {
    const ready = [...remaining.entries()]
      .filter(([, depList]) => depList.every((dep) => done.has(comparableKey(dep))))
      .map(([taskId]) => taskId)
      .sort((left, right) => (idLabels.get(left) ?? left).localeCompare(idLabels.get(right) ?? right))

    if (ready.length === 0) {
      const remainingLabels = [...remaining.keys()]
        .map((taskId) => idLabels.get(taskId) ?? taskId)
        .sort()
      throw new Error(`dependency cycle among tasks: ${formatPythonList(remainingLabels)}`)
    }

    for (const taskId of ready) {
      done.add(taskId)
      remaining.delete(taskId)
    }
  }
}
