import { readFile, readdir } from 'node:fs/promises';
import { basename, join } from 'node:path';
import { createTaskGraph } from '../../../graph/index.js';
import { RESULT_FILE, WORKERS_DIR, assertLegacyReport, assertRecord, assertRunState, assertTasks, assertWorkerResult, copyOptionalInteger, copyOptionalString, isErrno, parseJson, } from './artifact-codec.js';
export async function normalizeLegacyRunDir(runDir) {
    const rawState = await readStandaloneJson(join(runDir, 'state.json'));
    const tasks = assertTasks(await readStandaloneJson(join(runDir, 'tasks.json')));
    const report = await readOptionalLegacyReport(join(runDir, 'report.json'));
    const workerResults = await readWorkerResults(runDir);
    const graph = createTaskGraph(tasks, {
        idStrategy: tasks.some((task) => task.id.startsWith('T')) ? 'legacy-ordinal' : 'content-hash',
    });
    return {
        graph,
        report,
        runId: basename(runDir),
        state: normalizeLegacyState(rawState),
        tasks,
        workerResults,
    };
}
function normalizeLegacyState(value) {
    const record = assertRecord(value, 'state');
    const state = {};
    copyOptionalString(record, state, 'stage');
    copyOptionalString(record, state, 'intensity');
    copyOptionalInteger(record, state, 'rounds');
    copyOptionalInteger(record, state, 'task_count');
    copyOptionalString(record, state, 'spec_id');
    copyOptionalString(record, state, 'spec_slug');
    copyOptionalString(record, state, 'spec_relpath');
    copyOptionalString(record, state, 'integration_branch');
    return assertRunState(state);
}
async function readWorkerResults(runDir) {
    const workersDir = join(runDir, WORKERS_DIR);
    let taskIds;
    try {
        taskIds = await readdir(workersDir);
    }
    catch (error) {
        if (isErrno(error, 'ENOENT'))
            return new Map();
        /* c8 ignore next */
        throw error;
    }
    const results = new Map();
    for (const taskId of taskIds.sort()) {
        results.set(taskId, assertWorkerResult(await readStandaloneJson(join(workersDir, taskId, RESULT_FILE)), taskId));
    }
    return results;
}
async function readOptionalLegacyReport(path) {
    try {
        return assertLegacyReport(await readStandaloneJson(path));
    }
    catch (error) {
        if (isErrno(error, 'ENOENT'))
            return undefined;
        /* c8 ignore next */
        throw error;
    }
}
async function readStandaloneJson(path) {
    return parseJson(await readFile(path, 'utf8'));
}
