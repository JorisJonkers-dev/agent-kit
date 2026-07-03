import { renderPullRequestBody } from '../../../../domain/github/index.js'
import type { GhPr } from '../../../../ports/index.js'
import {
  hasPullRequestShape,
  isDryRun,
  logFailure,
  optionalTrimmed,
  parseJsonObject,
} from './helpers.js'
import type {
  GithubAdapterContext,
  GithubPullRequestMirrorRequest,
  MethodOptions,
} from './types.js'

export async function detectDefaultBranch(
  context: GithubAdapterContext,
  cwd: string,
): Promise<string | undefined> {
  try {
    const result = await context.client.gh(cwd, [
      'repo',
      'view',
      '--json',
      'defaultBranchRef',
      '--jq',
      '.defaultBranchRef.name',
    ])
    return optionalTrimmed(result.stdout)
  } catch (error) {
    logFailure(context, 'detectDefaultBranch', error)
    return undefined
  }
}

export async function createPullRequest(
  context: GithubAdapterContext,
  request: GithubPullRequestMirrorRequest,
  options: MethodOptions = {},
): Promise<GhPr | undefined> {
  try {
    if (isDryRun(context, options)) {
      return undefined
    }

    const body = renderPullRequestBody({
      summary: request.summary,
      ...(request.closingIssueNumbers
        ? { closingIssueNumbers: request.closingIssueNumbers }
        : {}),
      ...(request.referenceIssueNumbers
        ? { referenceIssueNumbers: request.referenceIssueNumbers }
        : {}),
      ...(request.extraSections ? { extraSections: request.extraSections } : {}),
    })
    const base = request.base ?? (await detectDefaultBranch(context, request.cwd))
    const args = [
      'pr',
      'create',
      '--title',
      request.title,
      '--body',
      body,
      '--json',
      'number,url',
    ]
    if (base) {
      args.push('--base', base)
    }
    if (request.head) {
      args.push('--head', request.head)
    }
    if (request.draft === true) {
      args.push('--draft')
    }

    return parseJsonObject<GhPr>(
      await context.client.ghJson(request.cwd, args),
      hasPullRequestShape,
    )
  } catch (error) {
    logFailure(context, 'createPullRequest', error)
    return undefined
  }
}

export async function viewPullRequest(
  context: GithubAdapterContext,
  cwd: string,
  number: number,
): Promise<GhPr | undefined> {
  try {
    return parseJsonObject<GhPr>(
      await context.client.ghJson(cwd, ['pr', 'view', String(number), '--json', 'number,url']),
      hasPullRequestShape,
    )
  } catch (error) {
    logFailure(context, 'viewPullRequest', error)
    return undefined
  }
}
