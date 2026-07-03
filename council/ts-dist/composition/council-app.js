import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname } from 'node:path';
import { normalizeLegacyRunDir } from '../adapters/runstore/index.js';
import { resolveCouncilConfig } from '../contexts/config/index.js';
import { planWaves } from '../contexts/graph/index.js';
import { recommendLenses } from '../contexts/triage/index.js';
import { assignAgents, configWorkflow, fanoutWorkflow, fleetWorkflow, parseAgentsPool, planWorkflow, reviewPackWorkflow, roundTripTasksMarkdownWorkflow, statusWorkflow, stringifyAssignments, } from '../workflows/index.js';
export class CouncilApp {
    gh;
    readText;
    writeText;
    constructor(deps = {}) {
        this.gh = deps.gh;
        this.readText = deps.readText ?? ((path) => readFile(path, 'utf8'));
        this.writeText = deps.writeText ?? writeTextFile;
    }
    plan(input = {}) {
        return Promise.resolve(planWorkflow(input));
    }
    fanout(input) {
        return fanoutWorkflow(input, {
            createPullRequest: (run) => this.createPullRequest(run),
            status: (statusInput) => this.status(statusInput),
        });
    }
    fleet(input) {
        return fleetWorkflow(input, {
            createPullRequest: (run) => this.createPullRequest(run),
            readText: this.readText,
        });
    }
    status(input) {
        return statusWorkflow(input, {
            normalizeRunDir: normalizeLegacyRunDir,
        });
    }
    readReviewPack(input) {
        return reviewPackWorkflow(input, {
            status: (statusInput) => this.status(statusInput),
        });
    }
    config(input) {
        return configWorkflow(input, {
            readText: this.readText,
            writeText: this.writeText,
        });
    }
    recommend(input = {}) {
        return Promise.resolve(recommendLenses(input.profile));
    }
    roundTripTasksMarkdown(tasksPath) {
        return roundTripTasksMarkdownWorkflow(tasksPath, { readText: this.readText });
    }
    async createPullRequest(run) {
        if (!this.gh)
            throw new Error('--github requires a gh adapter');
        const pr = await this.gh.createPullRequest({
            body: `Council run ${run}`,
            cwd: '.',
            draft: true,
            title: `Council ${run}`,
        });
        return pr.url;
    }
}
export function extractJson(text) {
    const fenced = /```json\s*([\s\S]*?)\s*```/u.exec(text);
    if (fenced?.[1])
        return JSON.parse(fenced[1]);
    const start = text.indexOf('{');
    if (start < 0)
        throw new Error('no JSON object found');
    for (let end = text.length; end > start; end -= 1) {
        const candidate = text.slice(start, end).trim();
        if (!candidate.endsWith('}'))
            continue;
        try {
            return JSON.parse(candidate);
        }
        catch {
            continue;
        }
    }
    throw new Error('no JSON object found');
}
export function renderTemplate(template, values) {
    return Object.entries(values).reduce((rendered, [key, value]) => rendered.replaceAll(`{{${key}}}`, value), template);
}
export function splitDestUrl(owner, name) {
    return `git@github.com:${owner}/${name}.git`;
}
export function localizeVerify(command, repoRoot, worktree) {
    return command.replaceAll(repoRoot, worktree);
}
export function pythonSelfTestGolden() {
    const tasks = [
        taskForSelfTest('T1', []),
        taskForSelfTest('T2', ['T1']),
        taskForSelfTest('T3', ['T1']),
        taskForSelfTest('T4', ['T2', 'T3']),
    ];
    const config = resolveCouncilConfig({ flags: { intensity: 'quick', rounds: 5 } });
    const agents = assignAgents(['t1', 't2', 't3'], parseAgentsPool('claude:haiku,codex:gpt-5.5'));
    return {
        agents: stringifyAssignments(agents),
        config: {
            defaultIntensity: resolveCouncilConfig().intensity,
            quickRoundOverride: config.rounds,
            thoroughWorker: resolveCouncilConfig({ flags: { intensity: 'thorough' } }).worker,
        },
        splitDestUrl: splitDestUrl('o', 'n'),
        verify: {
            localized: localizeVerify('cd /workspace/services/foo && npm test', '/workspace', '/tmp/wt/T1'),
            relative: localizeVerify('npm test', '/workspace', '/tmp/wt/T1'),
        },
        waves: planWaves(tasks),
    };
}
function taskForSelfTest(id, dependsOn) {
    return {
        boundaries: 'Stay in scope',
        depends_on: dependsOn,
        difficulty: 'moderate',
        id,
        model: 'haiku',
        objective: `Task ${id}`,
        output_format: 'Code edits',
        paths: [`${id}.txt`],
        title: id,
        verify: 'npm test',
    };
}
async function writeTextFile(path, text) {
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, text, 'utf8');
}
