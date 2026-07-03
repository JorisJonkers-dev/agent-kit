import { CouncilApp, } from '../app/index.js';
const COMMANDS = [
    { help: 'validate amendment payloads and append them to a run', name: 'amend' },
    { help: 'show or change council.toml while preserving unrelated lines', name: 'config' },
    { help: 'assemble context packs for downstream stages', name: 'context' },
    { help: 'run design stages D0-D5', name: 'design' },
    { help: 'execute a planned task DAG', name: 'fanout' },
    { help: 'round-robin a task DAG across an explicit agent pool', name: 'fleet' },
    { help: 'adversarially question task readiness', name: 'grill' },
    { help: 'inject operator guidance into a supervised worker', name: 'inject' },
    { help: 'compose planning stages without auto-executing workers', name: 'plan' },
    { help: 'recommend council lenses for a problem profile', name: 'recommend' },
    { help: 'assemble checkpoint review packs', name: 'review-pack' },
    { help: 'run TS parity checks for Python self-test cases', name: 'self-test' },
    { help: 'extract a subtree into a destination repo', name: 'split' },
    { help: 'summarize a run directory', name: 'status' },
    { help: 'supervise a worker process with watchdog controls', name: 'supervise' },
    { help: 'survey repository context', name: 'survey' },
    { help: 'synchronize BMAD assets', name: 'sync-bmad' },
    { help: 'synchronize council skills', name: 'sync-skills' },
    { help: 'tail one task log', name: 'tail' },
    { help: 'classify request routing before planning', name: 'triage' },
];
export function commandRegistry() {
    return COMMANDS;
}
export async function runCli(argv, runtime = {}) {
    const app = runtime.app ?? new CouncilApp();
    const [command, ...rest] = argv;
    try {
        if (command === undefined || command === '--help' || command === '-h') {
            return ok(renderHelp());
        }
        if (command === '--self-test' || command === 'self-test') {
            return ok(JSON.stringify(await appSelfTest(), null, 2));
        }
        if (!isCommand(command)) {
            return fail(`unknown command: ${command}`);
        }
        switch (command) {
            case 'plan':
                return okJson(await app.plan(parsePlan(rest)));
            case 'recommend':
                return okJson(await app.recommend(parseRecommend(rest)));
            case 'fanout':
                return okJson(await app.fanout(parseFanout(rest)));
            case 'fleet':
                return okJson(await app.fleet(parseFleet(rest)));
            case 'config':
                return okJson(await app.config({
                    ...parseConfig(rest),
                    paths: runtime.configPaths ?? defaultConfigPaths(),
                }));
            case 'status':
                return okJson(await app.status({ runDir: requireFlag(parseFlags(rest), 'run') }));
            case 'review-pack':
                return okJson(await app.readReviewPack(parseReviewPack(rest)));
            case 'triage':
                return okJson((await app.plan({ triage: parseTriage(rest) })).triage ?? {});
            case 'design':
            case 'amend':
            case 'context':
            case 'grill':
            case 'inject':
            case 'split':
            case 'supervise':
            case 'survey':
            case 'sync-bmad':
            case 'sync-skills':
            case 'tail':
                return okJson({ command, compiled: true });
        }
        return fail(`unknown command: ${command}`);
    }
    catch (error) {
        return fail(error instanceof Error ? error.message : String(error));
    }
}
async function appSelfTest() {
    const { pythonSelfTestGolden } = await import('../app/index.js');
    return pythonSelfTestGolden();
}
function parsePlan(argv) {
    const flags = parseFlags(argv);
    return {
        config: configOverrides(flags),
        design: flags.has('design'),
        ...(flags.get('brief') ? { brief: requireFlag(flags, 'brief') } : {}),
        ...(flags.get('run') ? { runDir: requireFlag(flags, 'run') } : {}),
        ...(flags.has('triage') ? { triage: parseTriageFlag(requireFlag(flags, 'triage')) } : {}),
    };
}
function parseFanout(argv) {
    const flags = parseFlags(argv);
    return {
        dryRun: flags.has('dry-run'),
        github: flags.has('github'),
        runDir: requireFlag(flags, 'run'),
    };
}
function parseRecommend(argv) {
    return { profile: JSON.parse(requireFlag(parseFlags(argv), 'input')) };
}
function parseFleet(argv) {
    const flags = parseFlags(argv);
    return {
        agents: requireFlag(flags, 'agents'),
        dryRun: flags.has('dry-run'),
        github: flags.has('github'),
        tasksPath: requireFlag(flags, 'tasks'),
    };
}
function parseConfig(argv) {
    const positional = argv.filter((arg) => !arg.startsWith('--'));
    const flags = parseFlags(argv);
    const action = positional[0];
    if (!isConfigAction(action))
        throw new Error('config requires action show|get|set|unset|path');
    return {
        action,
        ...(positional[1] ? { key: positional[1] } : {}),
        project: flags.has('project'),
        ...(positional[2] ? { value: positional[2] } : {}),
    };
}
function parseReviewPack(argv) {
    const flags = parseFlags(argv);
    const gate = requireFlag(flags, 'gate');
    if (gate !== '1' && gate !== 'design' && gate !== '2')
        throw new Error('--gate must be 1, design, or 2');
    return { gate, runDir: requireFlag(flags, 'run') };
}
function parseTriage(argv) {
    return parseTriageFlag(requireFlag(parseFlags(argv), 'input'));
}
function parseTriageFlag(raw) {
    const parsed = JSON.parse(raw);
    return parsed;
}
function parseFlags(argv) {
    const flags = new Map();
    for (let index = 0; index < argv.length; index += 1) {
        const arg = argv[index];
        if (!arg?.startsWith('--'))
            continue;
        const key = arg.slice(2);
        const next = argv[index + 1];
        if (next === undefined || next.startsWith('--')) {
            flags.set(key, 'true');
        }
        else {
            flags.set(key, next);
            index += 1;
        }
    }
    return flags;
}
function configOverrides(flags) {
    const config = {};
    const intensity = flags.get('intensity');
    const rounds = flags.get('rounds');
    const plannerA = flags.get('planner-a');
    const plannerB = flags.get('planner-b');
    const consolidator = flags.get('consolidator');
    const codexEffort = flags.get('codex-effort');
    if (intensity !== undefined)
        config.intensity = intensity;
    if (rounds !== undefined)
        config.rounds = Number.parseInt(rounds, 10);
    if (plannerA !== undefined)
        config.planner_a = plannerA;
    if (plannerB !== undefined)
        config.planner_b = plannerB;
    if (consolidator !== undefined)
        config.consolidator = consolidator;
    if (codexEffort !== undefined)
        config.codex_effort = codexEffort;
    return config;
}
function requireFlag(flags, name) {
    const value = flags.get(name);
    if (value === undefined || value === 'true')
        throw new Error(`--${name} is required`);
    return value;
}
function isCommand(value) {
    return COMMANDS.some((command) => command.name === value);
}
function isConfigAction(value) {
    return value === 'show' || value === 'get' || value === 'set' || value === 'unset' || value === 'path';
}
function renderHelp() {
    return COMMANDS.map((command) => `${command.name}\t${command.help}`).join('\n');
}
function defaultConfigPaths() {
    return {
        project: '.council.toml',
        user: `${process.env.HOME ?? '.'}/.config/council/council.toml`,
    };
}
function ok(stdout) {
    return { exitCode: 0, stderr: '', stdout: `${stdout.trimEnd()}\n` };
}
function okJson(value) {
    return ok(JSON.stringify(value, null, 2));
}
function fail(stderr) {
    return { exitCode: 2, stderr: `${stderr.trimEnd()}\n`, stdout: '' };
}
/* c8 ignore start -- process entry bootstrap; not unit-testable */
if (import.meta.url === `file://${process.argv[1] ?? ''}`) {
    void runCli(process.argv.slice(2)).then((result) => {
        if (result.stdout)
            process.stdout.write(result.stdout);
        if (result.stderr)
            process.stderr.write(result.stderr);
        process.exitCode = result.exitCode;
    });
}
/* c8 ignore stop */
