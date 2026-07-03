import { isJsonArray, isJsonRecord, isStringArray } from './task-json.js';
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
];
const TASK_ID_RE = /^(?:T[0-9]+|ck-[0-9a-f]{4,})$/;
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
]);
const SCHEMA_STRING_ARRAY_FIELDS = new Set([
    'paths',
    'depends_on',
    'acceptance_criteria',
    'context_refs',
    'supersedes',
]);
const SCHEMA_ALLOWED_FIELDS = new Set([
    ...SCHEMA_STRING_FIELDS,
    ...SCHEMA_STRING_ARRAY_FIELDS,
    'engine',
]);
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
};
export function validateTasksJsonSchema(tasks) {
    const errors = [];
    if (!Array.isArray(tasks)) {
        return { valid: false, errors: ['tasks must be an array'] };
    }
    for (const [index, task] of tasks.entries()) {
        const path = `$[${String(index)}]`;
        if (!isJsonRecord(task)) {
            errors.push(`${path} must be an object`);
            continue;
        }
        for (const field of REQUIRED_SCHEMA_FIELDS) {
            if (!(field in task)) {
                errors.push(`${path}.${field} is required`);
            }
        }
        for (const field of Object.keys(task)) {
            if (!SCHEMA_ALLOWED_FIELDS.has(field)) {
                errors.push(`${path}.${field} is not allowed by schema`);
            }
        }
        validateSchemaFieldTypes(task, path, errors);
    }
    return { valid: errors.length === 0, errors };
}
export function assertTasksJsonSchema(tasks) {
    const result = validateTasksJsonSchema(tasks);
    if (!result.valid) {
        throw new Error(`tasks JSON Schema validation failed: ${result.errors.join('; ')}`);
    }
}
function validateSchemaFieldTypes(task, path, errors) {
    for (const field of SCHEMA_STRING_FIELDS) {
        if (field in task && typeof task[field] !== 'string') {
            errors.push(`${path}.${field} must be a string`);
        }
    }
    for (const field of SCHEMA_STRING_ARRAY_FIELDS) {
        if (field in task && !isStringArray(task[field])) {
            errors.push(`${path}.${field} must be an array of strings`);
        }
    }
    if (typeof task.id === 'string' && !TASK_ID_RE.test(task.id)) {
        errors.push(`${path}.id must match a council task id`);
    }
    if (isJsonArray(task.depends_on)) {
        for (const [index, dep] of task.depends_on.entries()) {
            if (typeof dep === 'string' && !TASK_ID_RE.test(dep)) {
                errors.push(`${path}.depends_on[${String(index)}] must match a council task id`);
            }
        }
    }
    if (isJsonArray(task.supersedes)) {
        for (const [index, dep] of task.supersedes.entries()) {
            if (typeof dep === 'string' && !TASK_ID_RE.test(dep)) {
                errors.push(`${path}.supersedes[${String(index)}] must match a council task id`);
            }
        }
    }
    if (typeof task.difficulty === 'string' &&
        !['trivial', 'moderate', 'hard'].includes(task.difficulty)) {
        errors.push(`${path}.difficulty must be trivial, moderate, or hard`);
    }
    if (typeof task.model === 'string' && !['haiku', 'sonnet', 'opus'].includes(task.model)) {
        errors.push(`${path}.model must be haiku, sonnet, or opus`);
    }
}
