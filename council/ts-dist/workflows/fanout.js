export async function fanoutWorkflow(input, deps) {
    const summary = await deps.status({ runDir: input.runDir });
    const github = await resolveGithub(input.github, input.dryRun, summary.run, deps.createPullRequest);
    return {
        github: github.kind,
        ...(github.url ? { prUrl: github.url } : {}),
        run: summary.run,
        tasks: summary.tasks,
        waves: summary.waves,
    };
}
async function resolveGithub(github, dryRun, run, createPullRequest) {
    if (!github)
        return { kind: 'disabled' };
    if (dryRun)
        return { kind: 'dry-run' };
    return { kind: 'created', url: await createPullRequest(run) };
}
