export async function reviewPackWorkflow(input, deps) {
    const summary = await deps.status({ runDir: input.runDir });
    return {
        gate: input.gate,
        run: summary.run,
        task_count: summary.tasks.length,
        waves: summary.waves,
        worker_results: summary.workerResults.length,
    };
}
