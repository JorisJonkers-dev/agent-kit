import { isDryRun, logFailure } from './helpers.js';
export async function addComment(context, cwd, request, options = {}) {
    try {
        if (isDryRun(context, options)) {
            return;
        }
        await context.client.gh(cwd, [
            request.kind,
            'comment',
            String(request.number),
            '--body',
            request.body,
        ]);
    }
    catch (error) {
        logFailure(context, 'addComment', error);
    }
}
