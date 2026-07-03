import { assertTasksJsonSchema, validateTasks } from '../../../tasks/index.js';
export const STORY_FILE = 'story.json';
export const DESIGN_LEDGER_FILE = 'design-ledger.json';
export const WORKERS_DIR = 'workers';
export const RESULT_FILE = 'result.json';
export function parseJson(text) {
    return JSON.parse(text);
}
export function assertRunState(value) {
    const record = assertRecord(value, 'state');
    assertAllowed(record, 'state', [
        'stage',
        'intensity',
        'rounds',
        'task_count',
        'spec_id',
        'spec_slug',
        'spec_relpath',
        'agents',
        'integration_branch',
        'engine',
        'model_tier',
        'content_hash',
    ]);
    optionalString(record, 'state', 'stage');
    optionalString(record, 'state', 'intensity');
    optionalInteger(record, 'state', 'rounds');
    optionalInteger(record, 'state', 'task_count');
    optionalString(record, 'state', 'spec_id');
    optionalString(record, 'state', 'spec_slug');
    optionalString(record, 'state', 'spec_relpath');
    optionalStringArray(record, 'state', 'agents');
    optionalString(record, 'state', 'integration_branch');
    optionalString(record, 'state', 'model_tier');
    optionalString(record, 'state', 'content_hash');
    return record;
}
export function assertTasks(value) {
    validateTasks(value);
    assertTasksJsonSchema(value);
    return value;
}
export function assertStory(value) {
    const record = assertRecord(value, 'story');
    assertAllowed(record, 'story', [
        'title',
        'status',
        'goal',
        'user_value',
        'context',
        'acceptance_criteria',
        'scope',
        'implementation_notes',
        'tests',
        'definition_of_done',
    ]);
    requiredString(record, 'story', 'title');
    requiredString(record, 'story', 'status');
    requiredString(record, 'story', 'goal');
    assertStoryUserValue(record.user_value);
    requiredString(record, 'story', 'context');
    requiredStringArray(record, 'story', 'acceptance_criteria');
    assertStoryScope(record.scope);
    assertStoryImplementationNotes(record.implementation_notes);
    assertStoryTests(record.tests);
    requiredStringArray(record, 'story', 'definition_of_done');
    return record;
}
export function assertDesignLedger(value) {
    const record = assertRecord(value, 'design ledger');
    assertAllowed(record, 'design ledger', ['entries', 'content_hash']);
    optionalString(record, 'design ledger', 'content_hash');
    if ('entries' in record)
        requiredArray(record, 'design ledger', 'entries').forEach(assertDesignLedgerEntry);
    return record;
}
export function assertReviewVerdict(value) {
    const record = assertRecord(value, 'review verdict');
    assertAllowed(record, 'review verdict', [
        'satisfied',
        'reasons',
        'issues',
        'task_id',
        'reviewer',
        'engine',
        'model_tier',
        'content_hash',
    ]);
    requiredBoolean(record, 'review verdict', 'satisfied');
    requiredString(record, 'review verdict', 'reasons');
    requiredStringArray(record, 'review verdict', 'issues');
    optionalString(record, 'review verdict', 'task_id');
    optionalString(record, 'review verdict', 'reviewer');
    optionalString(record, 'review verdict', 'model_tier');
    optionalString(record, 'review verdict', 'content_hash');
    return record;
}
export function assertRoutingVerdict(value) {
    const record = assertRecord(value, 'routing verdict');
    assertAllowed(record, 'routing verdict', [
        'route',
        'reasons',
        'task_id',
        'candidate_routes',
        'engine',
        'model_tier',
        'context_refs',
        'content_hash',
    ]);
    requiredString(record, 'routing verdict', 'route');
    requiredString(record, 'routing verdict', 'reasons');
    optionalString(record, 'routing verdict', 'task_id');
    optionalStringArray(record, 'routing verdict', 'candidate_routes');
    optionalStringArray(record, 'routing verdict', 'context_refs');
    optionalString(record, 'routing verdict', 'model_tier');
    optionalString(record, 'routing verdict', 'content_hash');
    return record;
}
export function assertAmendment(value) {
    const record = assertRecord(value, 'amendment');
    assertAllowed(record, 'amendment', [
        'id',
        'summary',
        'reason',
        'status',
        'task_refs',
        'supersedes',
        'context_refs',
        'discovered_from',
        'engine',
        'model_tier',
        'content_hash',
    ]);
    requiredString(record, 'amendment', 'id');
    requiredString(record, 'amendment', 'summary');
    optionalString(record, 'amendment', 'reason');
    optionalString(record, 'amendment', 'status');
    optionalStringArray(record, 'amendment', 'task_refs');
    optionalStringArray(record, 'amendment', 'supersedes');
    optionalStringArray(record, 'amendment', 'context_refs');
    optionalString(record, 'amendment', 'discovered_from');
    optionalString(record, 'amendment', 'model_tier');
    optionalString(record, 'amendment', 'content_hash');
    return record;
}
export function assertWorkerResult(value, taskId) {
    const record = assertRecord(value, 'worker result');
    assertAllowed(record, 'worker result', [
        'task_id',
        'title',
        'model',
        'suggested_model',
        'engine',
        'model_tier',
        'branch',
        'worktree',
        'committed',
        'summary',
        'files_changed',
        'out_of_bounds',
        'verify_rc',
        'verify_output',
        'verdict',
        'merge',
        'status',
        'error',
        'content_hash',
    ]);
    requiredString(record, 'worker result', 'task_id');
    requiredString(record, 'worker result', 'status');
    if (taskId !== undefined && record.task_id !== taskId) {
        fail(`worker result task_id must match path task id: ${taskId}`);
    }
    optionalString(record, 'worker result', 'title');
    optionalString(record, 'worker result', 'model');
    optionalEnum(record, 'worker result', 'suggested_model', ['haiku', 'sonnet', 'opus']);
    optionalString(record, 'worker result', 'model_tier');
    optionalString(record, 'worker result', 'branch');
    optionalString(record, 'worker result', 'worktree');
    optionalBoolean(record, 'worker result', 'committed');
    optionalString(record, 'worker result', 'summary');
    optionalStringArray(record, 'worker result', 'files_changed');
    optionalStringArray(record, 'worker result', 'out_of_bounds');
    optionalIntegerOrNull(record, 'worker result', 'verify_rc');
    optionalString(record, 'worker result', 'verify_output');
    if (record.verdict !== undefined && record.verdict !== null)
        assertReviewVerdict(record.verdict);
    optionalString(record, 'worker result', 'merge');
    optionalString(record, 'worker result', 'error');
    optionalString(record, 'worker result', 'content_hash');
    return record;
}
export function assertLegacyReport(value) {
    const record = assertRecord(value, 'legacy report');
    requiredString(record, 'legacy report', 'run');
    optionalString(record, 'legacy report', 'integration_branch');
    optionalString(record, 'legacy report', 'integration_worktree');
    requiredArray(record, 'legacy report', 'waves').forEach((wave) => {
        assertStringArray(wave, 'legacy report wave');
    });
    requiredArray(record, 'legacy report', 'tasks').forEach(assertLegacyTaskReport);
    return record;
}
export function assertRunStoreEvent(value) {
    const record = assertRecord(value, 'run store event');
    if (record.type === 'review_verdict') {
        return { type: 'review_verdict', payload: assertReviewVerdict(record.payload) };
    }
    if (record.type === 'routing_verdict') {
        return { type: 'routing_verdict', payload: assertRoutingVerdict(record.payload) };
    }
    if (record.type === 'amendment') {
        return { type: 'amendment', payload: assertAmendment(record.payload) };
    }
    fail(`unsupported run store event type: ${formatJsonValue(record.type)}`);
}
export function assertRecord(value, label) {
    if (value === null || typeof value !== 'object' || Array.isArray(value))
        fail(`${label} must be an object`);
    return value;
}
export function copyOptionalString(from, to, field) {
    if (typeof from[field] === 'string')
        to[field] = from[field];
}
export function copyOptionalInteger(from, to, field) {
    if (Number.isInteger(from[field]))
        to[field] = from[field];
}
export function assertPathSegment(label, value) {
    if (value.length === 0)
        fail(`${label} must not be empty`);
    if (value.includes('/') || value.includes('\\') || value.includes('\0')) {
        fail(`${label} must be a single path segment`);
    }
}
export function isErrno(error, code) {
    return error instanceof Error && 'code' in error && error.code === code;
}
function assertStoryUserValue(value) {
    const record = assertRecord(value, 'story.user_value');
    assertAllowed(record, 'story.user_value', ['actor', 'capability', 'outcome']);
    requiredString(record, 'story.user_value', 'actor');
    requiredString(record, 'story.user_value', 'capability');
    requiredString(record, 'story.user_value', 'outcome');
}
function assertStoryScope(value) {
    const record = assertRecord(value, 'story.scope');
    assertAllowed(record, 'story.scope', ['in_scope', 'out_of_scope']);
    requiredStringArray(record, 'story.scope', 'in_scope');
    requiredStringArray(record, 'story.scope', 'out_of_scope');
}
function assertStoryImplementationNotes(value) {
    const record = assertRecord(value, 'story.implementation_notes');
    assertAllowed(record, 'story.implementation_notes', [
        'files',
        'patterns',
        'dependencies',
        'data_config_migration',
    ]);
    requiredStringArray(record, 'story.implementation_notes', 'files');
    requiredStringArray(record, 'story.implementation_notes', 'patterns');
    requiredStringArray(record, 'story.implementation_notes', 'dependencies');
    requiredStringArray(record, 'story.implementation_notes', 'data_config_migration');
}
function assertStoryTests(value) {
    const record = assertRecord(value, 'story.tests');
    assertAllowed(record, 'story.tests', ['unit', 'integration', 'manual_or_workflow']);
    requiredStringArray(record, 'story.tests', 'unit');
    requiredStringArray(record, 'story.tests', 'integration');
    requiredStringArray(record, 'story.tests', 'manual_or_workflow');
}
function assertDesignLedgerEntry(value) {
    const record = assertRecord(value, 'design ledger entry');
    assertAllowed(record, 'design ledger entry', [
        'id',
        'decision',
        'rationale',
        'status',
        'task_refs',
        'context_refs',
        'supersedes',
        'content_hash',
    ]);
    requiredString(record, 'design ledger entry', 'id');
    requiredString(record, 'design ledger entry', 'decision');
    optionalString(record, 'design ledger entry', 'rationale');
    optionalString(record, 'design ledger entry', 'status');
    optionalStringArray(record, 'design ledger entry', 'task_refs');
    optionalStringArray(record, 'design ledger entry', 'context_refs');
    optionalStringArray(record, 'design ledger entry', 'supersedes');
    optionalString(record, 'design ledger entry', 'content_hash');
}
function assertLegacyTaskReport(value) {
    const record = assertRecord(value, 'legacy task report');
    requiredString(record, 'legacy task report', 'task_id');
    optionalString(record, 'legacy task report', 'status');
    optionalString(record, 'legacy task report', 'merge');
    optionalString(record, 'legacy task report', 'model');
    optionalStringArray(record, 'legacy task report', 'files_changed');
    optionalIntegerOrNull(record, 'legacy task report', 'verify_rc');
    optionalBoolean(record, 'legacy task report', 'verifier_satisfied');
    optionalStringArray(record, 'legacy task report', 'out_of_bounds');
    optionalString(record, 'legacy task report', 'branch');
    optionalBoolean(record, 'legacy task report', 'good');
}
function assertAllowed(record, label, allowed) {
    const allowedSet = new Set(allowed);
    Object.keys(record).forEach((key) => {
        if (!allowedSet.has(key))
            fail(`${label}.${key} is not allowed`);
    });
}
function requiredString(record, label, field) {
    const value = record[field];
    if (typeof value !== 'string')
        fail(`${label}.${field} must be a string`);
    return value;
}
function optionalString(record, label, field) {
    if (record[field] !== undefined && typeof record[field] !== 'string')
        fail(`${label}.${field} must be a string`);
}
function requiredBoolean(record, label, field) {
    if (typeof record[field] !== 'boolean')
        fail(`${label}.${field} must be a boolean`);
}
function optionalBoolean(record, label, field) {
    if (record[field] !== undefined && typeof record[field] !== 'boolean')
        fail(`${label}.${field} must be a boolean`);
}
function optionalEnum(record, label, field, values) {
    const value = record[field];
    if (value !== undefined && (typeof value !== 'string' || !values.includes(value))) {
        fail(`${label}.${field} must be one of: ${values.join(', ')}`);
    }
}
function optionalInteger(record, label, field) {
    if (record[field] !== undefined && !Number.isInteger(record[field]))
        fail(`${label}.${field} must be an integer`);
}
function optionalIntegerOrNull(record, label, field) {
    if (record[field] !== undefined && record[field] !== null && !Number.isInteger(record[field])) {
        fail(`${label}.${field} must be an integer or null`);
    }
}
function requiredArray(record, label, field) {
    const value = record[field];
    if (!Array.isArray(value))
        fail(`${label}.${field} must be an array`);
    return value;
}
function requiredStringArray(record, label, field) {
    assertStringArray(requiredArray(record, label, field), `${label}.${field}`);
}
function optionalStringArray(record, label, field) {
    if (record[field] !== undefined)
        assertStringArray(record[field], `${label}.${field}`);
}
function assertStringArray(value, label) {
    if (!Array.isArray(value) || value.some((item) => typeof item !== 'string')) {
        fail(`${label} must be an array of strings`);
    }
}
function formatJsonValue(value) {
    return typeof value === 'string' ? value : JSON.stringify(value);
}
function fail(message) {
    throw new Error(message);
}
