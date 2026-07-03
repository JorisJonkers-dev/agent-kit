export const RUNSTORE_STATE_FILE = 'state.json';
export const RUNSTORE_TASKS_FILE = 'tasks.json';
export const RUNSTORE_EVENTS_FILE = 'events.jsonl';
export const RUNSTORE_EVENTS_LOCK_FILE = 'events.jsonl.lock';
export function reviewVerdictEvent(payload) {
    return {
        type: 'review_verdict',
        payload,
    };
}
export function routingVerdictEvent(payload) {
    return {
        type: 'routing_verdict',
        payload,
    };
}
export function amendmentEvent(payload) {
    return {
        type: 'amendment',
        payload,
    };
}
export function planStateWrite(runId, state, tempId) {
    return planAtomicJsonWrite({
        runId,
        target: RUNSTORE_STATE_FILE,
        tempId,
        value: state,
    });
}
export function planTasksWrite(runId, tasks, tempId) {
    return planAtomicJsonWrite({
        runId,
        target: RUNSTORE_TASKS_FILE,
        tempId,
        value: tasks,
    });
}
export function planAtomicJsonWrite(input) {
    assertPathSegment('runId', input.runId);
    assertPathSegment('tempId', input.tempId);
    const finalPath = runStorePath(input.runId, input.target);
    const tempPath = runStorePath(input.runId, `.${input.target}.${input.tempId}.tmp`);
    const bytes = serializeJson(input.value, 2);
    return {
        kind: 'atomic-json-write',
        runId: input.runId,
        target: input.target,
        finalPath,
        tempPath,
        bytes,
        steps: [
            {
                kind: 'write-temp-file',
                path: tempPath,
                bytes,
            },
            {
                kind: 'sync-file',
                path: tempPath,
            },
            {
                kind: 'rename-file',
                fromPath: tempPath,
                toPath: finalPath,
            },
            {
                kind: 'sync-directory',
                path: input.runId,
            },
        ],
    };
}
export function planEventAppend(runId, event) {
    return planEventsAppend({
        runId,
        events: [event],
    });
}
export function planEventsAppend(input) {
    assertPathSegment('runId', input.runId);
    if (input.events.length === 0) {
        throw new Error('events must not be empty');
    }
    const eventPath = runStorePath(input.runId, RUNSTORE_EVENTS_FILE);
    const lockPath = runStorePath(input.runId, RUNSTORE_EVENTS_LOCK_FILE);
    const bytes = input.events.map((event) => serializeJson(event, 0)).join('');
    return {
        kind: 'locked-event-append',
        runId: input.runId,
        eventPath,
        lockPath,
        events: input.events,
        bytes,
        steps: [
            {
                kind: 'acquire-lock',
                path: lockPath,
            },
            {
                kind: 'append-file',
                path: eventPath,
                bytes,
            },
            {
                kind: 'sync-file',
                path: eventPath,
            },
            {
                kind: 'release-lock',
                path: lockPath,
            },
        ],
    };
}
function runStorePath(runId, fileName) {
    return `${runId}/${fileName}`;
}
function serializeJson(value, space) {
    const serialized = JSON.stringify(value, null, space);
    if (typeof serialized !== 'string') {
        throw new Error('runstore values must be JSON serializable');
    }
    return `${serialized}\n`;
}
function assertPathSegment(label, value) {
    if (value.length === 0) {
        throw new Error(`${label} must not be empty`);
    }
    if (value.includes('/') || value.includes('\\') || value.includes('\0')) {
        throw new Error(`${label} must be a single path segment`);
    }
}
