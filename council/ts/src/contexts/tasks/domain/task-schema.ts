import type { JsonRecord } from '../../../shared-kernel/common.js'
import { TASK_ATTACHMENT_ACTIVE_SKILLS_MAX } from '../../../shared-kernel/index.js'
import { isJsonArray, isJsonRecord, isStringArray } from './task-json.js'

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
  'success_criteria',
  'verify_proves',
  'failure_modes',
  'supersedes',
])

const SCHEMA_RECORD_FIELDS = new Set([
  'retry_policy',
  'resource_profile',
])

const SCHEMA_BOOLEAN_FIELDS = new Set([
  'human_review_required',
])

const ATTACHMENT_FIELDS = new Set([
  'activeSkills',
  'mcpProfile',
])

const SCHEMA_ALLOWED_FIELDS = new Set([
  ...SCHEMA_STRING_FIELDS,
  ...SCHEMA_STRING_ARRAY_FIELDS,
  ...SCHEMA_RECORD_FIELDS,
  ...SCHEMA_BOOLEAN_FIELDS,
  'attachment',
  'engine',
])

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
      attachment: {
        type: 'object',
        additionalProperties: false,
        required: ['activeSkills', 'mcpProfile'],
        properties: {
          activeSkills: {
            type: 'array',
            items: { type: 'string' },
            maxItems: TASK_ATTACHMENT_ACTIVE_SKILLS_MAX,
          },
          mcpProfile: { type: 'string', minLength: 1, pattern: '\\S' },
        },
      },
      success_criteria: { type: 'array', items: { type: 'string' } },
      verify_proves: { type: 'array', items: { type: 'string' } },
      failure_modes: { type: 'array', items: { type: 'string' } },
      retry_policy: { type: 'object' },
      resource_profile: { type: 'object' },
      human_review_required: { type: 'boolean' },
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

  for (const field of SCHEMA_RECORD_FIELDS) {
    if (field in task && !isJsonRecord(task[field])) {
      errors.push(`${path}.${field} must be an object`)
    }
  }

  for (const field of SCHEMA_BOOLEAN_FIELDS) {
    if (field in task && typeof task[field] !== 'boolean') {
      errors.push(`${path}.${field} must be a boolean`)
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

  validateAttachment(task.attachment, path, errors)
}

function validateAttachment(
  attachment: JsonRecord[string] | undefined,
  path: string,
  errors: string[],
): void {
  if (attachment === undefined) {
    return
  }

  if (!isJsonRecord(attachment)) {
    errors.push(`${path}.attachment must be an object`)
    return
  }

  for (const field of Object.keys(attachment)) {
    if (!ATTACHMENT_FIELDS.has(field)) {
      errors.push(`${path}.attachment.${field} is not allowed by schema`)
    }
  }

  if (!('activeSkills' in attachment)) {
    errors.push(`${path}.attachment.activeSkills is required`)
  }
  if (!('mcpProfile' in attachment)) {
    errors.push(`${path}.attachment.mcpProfile is required`)
  }
  if ('mcpProfile' in attachment && typeof attachment.mcpProfile !== 'string') {
    errors.push(`${path}.attachment.mcpProfile must be a string`)
  }
  if (
    typeof attachment.mcpProfile === 'string' &&
    attachment.mcpProfile.trim().length === 0
  ) {
    errors.push(`${path}.attachment.mcpProfile must be a non-empty string`)
  }
  if ('activeSkills' in attachment && !isStringArray(attachment.activeSkills)) {
    errors.push(`${path}.attachment.activeSkills must be an array of strings`)
  }
  if (
    isJsonArray(attachment.activeSkills) &&
    attachment.activeSkills.length > TASK_ATTACHMENT_ACTIVE_SKILLS_MAX
  ) {
    errors.push(`${path}.attachment.activeSkills must contain at most ${String(TASK_ATTACHMENT_ACTIVE_SKILLS_MAX)} skills`)
  }
}
