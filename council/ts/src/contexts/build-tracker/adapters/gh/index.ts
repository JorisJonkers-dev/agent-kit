import type { BuildTrackerIssuesPort, IssueRecord } from '../../application/ports.js'
import type { RepositoryName } from '../../domain/spec.js'
import type { BuildTrackerProcess } from '../process/index.js'

/**
 * Reads issues through `gh api`. The REST API is used because `gh issue
 * view` does not show sub-issues or dependencies. Any failure rejects with
 * one line that says what gh reported.
 */
export class GhCliBuildTrackerIssues implements BuildTrackerIssuesPort {
  constructor(private readonly process: BuildTrackerProcess) {}

  async issue(repository: RepositoryName, number: number): Promise<IssueRecord> {
    const parsed = await this.api(repository, `issues/${String(number)}`, false)
    if (!isRestIssue(parsed)) throw new Error('gh api printed something that is not an issue')
    return record(parsed, repository)
  }

  subIssues(repository: RepositoryName, number: number): Promise<readonly IssueRecord[]> {
    return this.list(repository, `issues/${String(number)}/sub_issues`)
  }

  blockedBy(repository: RepositoryName, number: number): Promise<readonly IssueRecord[]> {
    return this.list(repository, `issues/${String(number)}/dependencies/blocked_by`)
  }

  /** Every page of one of an issue's lists: `--paginate --slurp` prints one array per page inside an outer array. */
  private async list(repository: RepositoryName, path: string): Promise<readonly IssueRecord[]> {
    const parsed = await this.api(repository, `${path}?per_page=100`, true)
    const issues = Array.isArray(parsed) && parsed.every(Array.isArray) ? (parsed as unknown[][]).flat() : undefined
    if (!issues?.every(isRestIssue)) throw new Error('gh api printed something that is not a list of issues')
    return issues.map((issue) => record(issue, repository))
  }

  private async api(repository: RepositoryName, path: string, paginate: boolean): Promise<unknown> {
    const endpoint = `repos/${repository.owner}/${repository.repo}/${path}`
    const args = ['api', '--hostname', repository.host, ...(paginate ? ['--paginate', '--slurp'] : []), endpoint]
    const result = await this.process.exec({ args, command: 'gh' })
    if (result.exitCode !== 0) {
      const detail = firstLine(result.stderr) ?? firstLine(result.stdout) ?? `exit code ${String(result.exitCode)}`
      throw new Error(`gh api ${endpoint} exited ${String(result.exitCode)}: ${detail}`)
    }
    try {
      return JSON.parse(result.stdout) as unknown
    } catch {
      return undefined
    }
  }
}

interface RestIssue {
  readonly number: number
  readonly html_url?: unknown
  readonly title?: unknown
  readonly state?: unknown
}

function isRestIssue(value: unknown): value is RestIssue {
  return typeof value === 'object' && value !== null && typeof (value as RestIssue).number === 'number'
}

/** The issue's repository comes from its URL; one with no readable URL is taken to be in the repository asked. */
function record(issue: RestIssue, asked: RepositoryName): IssueRecord {
  const url = typeof issue.html_url === 'string' ? /^https?:\/\/([^/]+)\/([^/]+)\/([^/]+)\/(?:issues|pull)\/\d+/.exec(issue.html_url) : null
  return {
    number: issue.number,
    repository: url === null ? asked : { host: String(url[1]).toLowerCase(), owner: String(url[2]), repo: String(url[3]) },
    state: issue.state === 'closed' ? 'closed' : 'open',
    title: typeof issue.title === 'string' ? issue.title : '',
  }
}

function firstLine(text: string): string | undefined {
  return text
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line.length > 0)
}
