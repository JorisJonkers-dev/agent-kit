import { resolveCouncilConfig } from '../contexts/config/index.js';
import { classifyTriage } from '../contexts/triage/index.js';
export function planWorkflow(input = {}) {
    const config = resolveCouncilConfig(input.config === undefined ? {} : { flags: input.config });
    const triage = input.triage ? classifyTriage(input.triage) : undefined;
    const taskLimit = triage?.route === 'direct' ? 1 : undefined;
    return {
        command: 'plan',
        config,
        designRequested: input.design ?? false,
        directTierPolicy: 'shrink-dag-only',
        executesWorkers: false,
        estimatedModelCalls: 2 + config.rounds * 4 + 1,
        ...(input.runDir ? { runDir: input.runDir } : {}),
        ...(taskLimit === undefined ? {} : { taskLimit }),
        ...(triage ? { triage } : {}),
    };
}
