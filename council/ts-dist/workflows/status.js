import { planWaves } from '../contexts/graph/index.js';
export async function statusWorkflow(input, deps) {
    const normalized = await deps.normalizeRunDir(input.runDir);
    return {
        ...(normalized.report ? { report: normalized.report } : {}),
        run: normalized.runId,
        state: normalized.state,
        tasks: normalized.tasks,
        waves: normalized.report?.waves ?? planWaves(normalized.tasks),
        workerResults: [...normalized.workerResults.values()],
    };
}
