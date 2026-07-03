import { hasName, isBootstrapEnabled, isDryRun, logFailure, parseJsonArray, } from './helpers.js';
export async function listVerifiedLabels(context, cwd) {
    try {
        const result = await context.client.gh(cwd, [
            'label',
            'list',
            '--json',
            'name',
            '--limit',
            '1000',
        ]);
        return parseJsonArray(result.stdout).filter(hasName);
    }
    catch (error) {
        logFailure(context, 'listVerifiedLabels', error);
        return [];
    }
}
export async function bootstrapLabels(context, cwd, labels, options = {}) {
    try {
        if (!isBootstrapEnabled(context, options) || isDryRun(context, options)) {
            return;
        }
        for (const label of labels) {
            await context.client.gh(cwd, ['label', 'create', label, '--force']);
        }
    }
    catch (error) {
        logFailure(context, 'bootstrapLabels', error);
    }
}
