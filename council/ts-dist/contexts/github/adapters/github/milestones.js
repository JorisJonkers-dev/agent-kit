import { chooseMilestone } from '../../../github/index.js';
import { hasMilestoneShape, isDryRun, logFailure, parseJsonArray, parseJsonObject, } from './helpers.js';
export async function listMilestones(context, cwd) {
    try {
        const result = await context.client.gh(cwd, [
            'api',
            'repos/{owner}/{repo}/milestones',
            '--paginate',
            '-f',
            'state=all',
        ]);
        return parseJsonArray(result.stdout).filter(hasMilestoneShape);
    }
    catch (error) {
        logFailure(context, 'listMilestones', error);
        return [];
    }
}
export async function ensureMilestone(context, cwd, title, options = {}) {
    try {
        const decision = chooseMilestone(title, await listMilestones(context, cwd));
        if (decision.kind === 'reuse') {
            return decision.milestone;
        }
        if (isDryRun(context, options)) {
            return undefined;
        }
        const result = await context.client.gh(cwd, [
            'api',
            'repos/{owner}/{repo}/milestones',
            '-f',
            `title=${title}`,
        ]);
        return parseJsonObject(result.stdout, hasMilestoneShape);
    }
    catch (error) {
        logFailure(context, 'ensureMilestone', error);
        return undefined;
    }
}
