import { basename } from 'node:path';
import { planWaves } from '../contexts/graph/index.js';
import { assertTasksBijection, parseTasksMd, renderTasksMd, validateTasks } from '../contexts/tasks/index.js';
export async function fleetWorkflow(input, deps) {
    const tasks = await readTasksJson(input.tasksPath, deps.readText);
    const waves = planWaves(tasks);
    const ids = tasks.map((task) => task.id);
    const agents = assignAgents(ids, parseAgentsPool(input.agents));
    const run = basename(input.tasksPath, '.json');
    const github = await resolveGithub(input.github, input.dryRun, run, deps.createPullRequest);
    return {
        agents: stringifyAssignments(agents),
        github: github.kind,
        ...(github.url ? { prUrl: github.url } : {}),
        run,
        tasks,
        waves,
    };
}
export async function roundTripTasksMarkdownWorkflow(tasksPath, deps) {
    const tasks = await readTasksJson(tasksPath, deps.readText);
    const records = tasks;
    const markdown = renderTasksMd(records);
    assertTasksBijection(records, markdown);
    return parseTasksMd(markdown);
}
export function parseEngineSpec(spec) {
    const [cli, ...rest] = spec.split(':');
    const model = rest.join(':');
    if ((cli !== 'claude' && cli !== 'codex') || model.trim().length === 0) {
        throw new Error(`engine must be claude:<model> or codex:<model>, got ${JSON.stringify(spec)}`);
    }
    return { cli, label: `${cli}:${model}`, model };
}
export function parseAgentsPool(spec) {
    if (spec.trim().length === 0)
        throw new Error('agents pool must not be empty');
    return spec.split(',').flatMap((part) => {
        const pieces = part.trim().split('*');
        if (pieces.length > 2)
            throw new Error(`malformed agent spec ${JSON.stringify(part)}`);
        const [engineRaw, countRaw] = pieces;
        if (!engineRaw)
            throw new Error(`malformed agent spec ${JSON.stringify(part)}`);
        const engine = parseEngineSpec(engineRaw);
        const count = countRaw === undefined ? 1 : Number.parseInt(countRaw, 10);
        if (!Number.isInteger(count) || count < 1 || String(count) !== String(countRaw ?? 1)) {
            throw new Error(`agent count must be a positive integer in ${JSON.stringify(part)}`);
        }
        return Array.from({ length: count }, () => engine);
    });
}
export function assignAgents(taskIds, agents) {
    const [head, ...tail] = agents;
    if (head === undefined)
        throw new Error('agents pool must not be empty');
    const pool = [head, ...tail];
    return new Map(taskIds.map((taskId, index) => [taskId, pool[index % pool.length] ?? head]));
}
export function stringifyAssignments(assignments) {
    return Object.fromEntries([...assignments.entries()].map(([taskId, engine]) => [taskId, `${engine.cli}:${engine.model}`]));
}
async function readTasksJson(path, readText) {
    const parsed = JSON.parse(await readText(path));
    validateTasks(parsed);
    return parsed;
}
async function resolveGithub(github, dryRun, run, createPullRequest) {
    if (!github)
        return { kind: 'disabled' };
    if (dryRun)
        return { kind: 'dry-run' };
    return { kind: 'created', url: await createPullRequest(run) };
}
