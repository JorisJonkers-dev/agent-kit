import { isDryRun, logFailure } from './helpers.js'
import type {
  GithubAdapterContext,
  GithubCommentRequest,
  MethodOptions,
} from './types.js'

export async function addComment(
  context: GithubAdapterContext,
  cwd: string,
  request: GithubCommentRequest,
  options: MethodOptions = {},
): Promise<void> {
  try {
    if (isDryRun(context, options)) {
      return
    }

    await context.client.gh(cwd, [
      request.kind,
      'comment',
      String(request.number),
      '--body',
      request.body,
    ])
  } catch (error) {
    logFailure(context, 'addComment', error)
  }
}
