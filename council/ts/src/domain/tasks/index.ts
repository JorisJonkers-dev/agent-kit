import type { JsonRecord, JsonValue } from '../contracts/common.js'

const TASK_BLOCK_RE =
  /^## (?<headerId>[^\n:]+)(?::[^\n]*)?\n<!-- council-task-id: (?<markerId>[^>]+) -->\n```json\n(?<body>.*?)\n```/gms

const REQUIRED_VALIDATE_FIELDS = ['id', 'objective', 'depends_on', 'paths', 'model', 'verify'] as const

const REQUIRED_SCHEMA_FIELDS = [
  'id',
  'title',
  'objective',
  'output_format',
  'paths',
  'depends_on',
  'difficulty',
  'model',
  'verify',
  'boundaries',
] as const

const TASK_ID_RE = /^(?:T[0-9]+|ck-[0-9a-f]{4,})$/

const SCHEMA_STRING_FIELDS = new Set([
  'id',
  'title',
  'objective',
  'output_format',
  'difficulty',
  'model',
  'dev_notes',
  'spec_ref',
  'archetype',
  'context_profile',
  'discovered_from',
  'content_hash',
  'model_tier',
  'verify',
  'boundaries',
])

const SCHEMA_STRING_ARRAY_FIELDS = new Set([
  'paths',
  'depends_on',
  'acceptance_criteria',
  'context_refs',
  'supersedes',
])

const SCHEMA_ALLOWED_FIELDS = new Set([
  ...SCHEMA_STRING_FIELDS,
  ...SCHEMA_STRING_ARRAY_FIELDS,
  'engine',
])

export interface SpecRefLike {
  readonly name: string
}

export interface ValidateTasksOptions {
  readonly onWarning?: (message: string) => void
}

export interface TasksJsonSchemaValidation {
  readonly valid: boolean
  readonly errors: readonly string[]
}

export const TASKS_JSON_SCHEMA = {
  $schema: 'http://json-schema.org/draft-07/schema#',
  $comment: 'JSON Schema is a SECONDARY tooling layer; validateTasks remains the authoritative gate.',
  title: 'council-tasks',
  type: 'array',
  items: {
    type: 'object',
    additionalProperties: false,
    required: REQUIRED_SCHEMA_FIELDS,
    properties: {
      id: {
        oneOf: [
          { type: 'string', pattern: '^T[0-9]+$' },
          { type: 'string', pattern: '^ck-[0-9a-f]{4,}$' },
        ],
      },
      title: { type: 'string' },
      objective: { type: 'string' },
      output_format: { type: 'string' },
      paths: { type: 'array', items: { type: 'string' } },
      depends_on: { type: 'array', items: { $ref: '#/items/properties/id' } },
      difficulty: { type: 'string', enum: ['trivial', 'moderate', 'hard'] },
      model: { type: 'string', enum: ['haiku', 'sonnet', 'opus'] },
      acceptance_criteria: { type: 'array', items: { type: 'string' } },
      dev_notes: { type: 'string' },
      spec_ref: { type: 'string' },
      context_refs: { type: 'array', items: { type: 'string' } },
      archetype: { type: 'string' },
      context_profile: { type: 'string' },
      discovered_from: { type: 'string' },
      supersedes: { type: 'array', items: { $ref: '#/items/properties/id' } },
      content_hash: { type: 'string' },
      engine: {},
      model_tier: { type: 'string' },
      verify: { type: 'string' },
      boundaries: { type: 'string' },
    },
  },
} as const

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
  }

  assertTaskDag(tasks)
}

export function validateTasksJsonSchema(tasks: unknown): TasksJsonSchemaValidation {
  const errors: string[] = []

  if (!Array.isArray(tasks)) {
    return { valid: false, errors: ['tasks must be an array'] }
  }

  for (const [index, task] of tasks.entries()) {
    const path = `$[${String(index)}]`
    if (!isJsonRecord(task)) {
      errors.push(`${path} must be an object`)
      continue
    }

    for (const field of REQUIRED_SCHEMA_FIELDS) {
      if (!(field in task)) {
        errors.push(`${path}.${field} is required`)
      }
    }

    for (const field of Object.keys(task)) {
      if (!SCHEMA_ALLOWED_FIELDS.has(field)) {
        errors.push(`${path}.${field} is not allowed by schema`)
      }
    }

    validateSchemaFieldTypes(task, path, errors)
  }

  return { valid: errors.length === 0, errors }
}

export function assertTasksJsonSchema(tasks: unknown): void {
  const result = validateTasksJsonSchema(tasks)
  if (!result.valid) {
    throw new Error(`tasks JSON Schema validation failed: ${result.errors.join('; ')}`)
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

function validateSchemaFieldTypes(task: JsonRecord, path: string, errors: string[]): void {
  for (const field of SCHEMA_STRING_FIELDS) {
    if (field in task && typeof task[field] !== 'string') {
      errors.push(`${path}.${field} must be a string`)
    }
  }

  for (const field of SCHEMA_STRING_ARRAY_FIELDS) {
    if (field in task && !isStringArray(task[field])) {
      errors.push(`${path}.${field} must be an array of strings`)
    }
  }

  if (typeof task.id === 'string' && !TASK_ID_RE.test(task.id)) {
    errors.push(`${path}.id must match a council task id`)
  }

  if (isJsonArray(task.depends_on)) {
    for (const [index, dep] of task.depends_on.entries()) {
      if (typeof dep === 'string' && !TASK_ID_RE.test(dep)) {
        errors.push(`${path}.depends_on[${String(index)}] must match a council task id`)
      }
    }
  }

  if (isJsonArray(task.supersedes)) {
    for (const [index, dep] of task.supersedes.entries()) {
      if (typeof dep === 'string' && !TASK_ID_RE.test(dep)) {
        errors.push(`${path}.supersedes[${String(index)}] must match a council task id`)
      }
    }
  }

  if (
    typeof task.difficulty === 'string' &&
    !['trivial', 'moderate', 'hard'].includes(task.difficulty)
  ) {
    errors.push(`${path}.difficulty must be trivial, moderate, or hard`)
  }

  if (typeof task.model === 'string' && !['haiku', 'sonnet', 'opus'].includes(task.model)) {
    errors.push(`${path}.model must be haiku, sonnet, or opus`)
  }
}

function normaliseTasks(tasks: readonly JsonRecord[]): string {
  return stableJsonStringify(tasks)
}

function stableJsonStringify(value: unknown, level = 0): string {
  if (value === null) {
    return 'null'
  }

  if (typeof value === 'string') {
    return quoteJsonString(value)
  }

  if (typeof value === 'number' || typeof value === 'boolean') {
    return primitiveJsonString(value)
  }

  if (Array.isArray(value)) {
    if (value.length === 0) {
      return '[]'
    }

    const currentIndent = ' '.repeat(level * 2)
    const nextIndent = ' '.repeat((level + 1) * 2)
    return `[\n${value.map((item) => `${nextIndent}${stableJsonStringify(item, level + 1)}`).join(',\n')}\n${currentIndent}]`
  }

  if (isJsonRecord(value)) {
    const keys = Object.keys(value).sort()
    if (keys.length === 0) {
      return '{}'
    }

    const currentIndent = ' '.repeat(level * 2)
    const nextIndent = ' '.repeat((level + 1) * 2)
    return `{\n${keys
      .map((key) => `${nextIndent}${quoteJsonString(key)}: ${stableJsonStringify(value[key], level + 1)}`)
      .join(',\n')}\n${currentIndent}}`
  }

  throw new Error(`cannot serialize non-JSON value: ${describeNonJsonValue(value)}`)
}

function quoteJsonString(value: string): string {
  const quoted = JSON.stringify(value)
  let escaped = ''
  for (let index = 0; index < quoted.length; index += 1) {
    const code = quoted.charCodeAt(index)
    escaped += code > 127 ? `\\u${code.toString(16).padStart(4, '0')}` : quoted.charAt(index)
  }
  return escaped
}

function isJsonRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isJsonArray(value: JsonValue | undefined): value is readonly JsonValue[] {
  return Array.isArray(value)
}

function isStringArray(value: JsonValue | undefined): boolean {
  return isJsonArray(value) && value.every((item) => typeof item === 'string')
}

function comparableKey(value: JsonValue | undefined): string {
  return stableJsonStringify(value)
}

function stringifyForDisplay(value: JsonValue | string | undefined): string {
  if (typeof value === 'string') {
    return value
  }
  if (typeof value === 'number' || typeof value === 'boolean' || value === null) {
    return primitiveJsonString(value)
  }
  if (value === undefined) {
    return ''
  }
  return stableJsonStringify(value)
}

function primitiveJsonString(value: number | boolean | null): string {
  if (typeof value === 'boolean') {
    return value ? 'true' : 'false'
  }
  if (value === null) {
    return 'null'
  }
  return Number.isFinite(value) ? String(Object.is(value, -0) ? 0 : value) : 'null'
}

function describeNonJsonValue(value: unknown): string {
  return value === undefined ? 'undefined' : typeof value
}

function formatPythonList(values: readonly string[]): string {
  return `[${values.map((value) => pythonRepr(value)).join(', ')}]`
}

function pythonRepr(value: string): string {
  return `'${value.replaceAll('\\', '\\\\').replaceAll("'", "\\'")}'`
}

function jsonErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
