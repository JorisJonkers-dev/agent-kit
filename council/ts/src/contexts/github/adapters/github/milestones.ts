import { chooseMilestone } from '../../../../domain/github/index.js'
import type { ExistingGithubMilestone } from '../../../../domain/github/index.js'
import {
  hasMilestoneShape,
  isDryRun,
  logFailure,
  parseJsonArray,
  parseJsonObject,
} from './helpers.js'
import type { GithubAdapterContext, MethodOptions } from './types.js'

export async function listMilestones(
  context: GithubAdapterContext,
  cwd: string,
): Promise<readonly ExistingGithubMilestone[]> {
  try {
    const result = await context.client.gh(cwd, [
      'api',
      'repos/{owner}/{repo}/milestones',
      '--paginate',
      '-f',
      'state=all',
    ])
    return parseJsonArray(result.stdout).filter(hasMilestoneShape)
  } catch (error) {
    logFailure(context, 'listMilestones', error)
    return []
  }
}

export async function ensureMilestone(
  context: GithubAdapterContext,
  cwd: string,
  title: string,
  options: MethodOptions = {},
): Promise<ExistingGithubMilestone | undefined> {
  try {
    const decision = chooseMilestone(title, await listMilestones(context, cwd))
    if (decision.kind === 'reuse') {
      return decision.milestone
    }
    if (isDryRun(context, options)) {
      return undefined
    }

    const result = await context.client.gh(cwd, [
      'api',
      'repos/{owner}/{repo}/milestones',
      '-f',
      `title=${title}`,
    ])
    return parseJsonObject<ExistingGithubMilestone>(result.stdout, hasMilestoneShape)
  } catch (error) {
    logFailure(context, 'ensureMilestone', error)
    return undefined
  }
}
