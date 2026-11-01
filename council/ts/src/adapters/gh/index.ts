import type {
  GhIssue,
  GhIssueComment,
  GhIssueLocator,
  GhIssueRequest,
  GhLinkedIssue,
  GhPort,
  ProcessPort,
} from '../../ports/index.js'

/** Reads issues with the `gh` CLI. Any failure rejects with one line that says what gh reported. */
export class GhCliAdapter implements Pick<GhPort, 'viewIssue' | 'subIssues' | 'blockedBy'> {
  constructor(private readonly process: ProcessPort) {}

  async viewIssue(request: GhIssueRequest): Promise<GhIssue> {
    const fields = request.comments ? 'number,title,body,url,labels,comments' : 'number,title,body,url,labels'
    const args = ['issue', 'view', String(request.number), '--repo', request.repo, '--json', fields]
    const result = await this.process.exec({ args, command: 'gh', cwd: request.cwd })
    if (result.exitCode !== 0) {
      const detail = firstLine(result.stderr) ?? firstLine(result.stdout) ?? `exit code ${String(result.exitCode)}`
      throw new Error(`gh issue view exited ${String(result.exitCode)}: ${detail}`)
    }
    const issue = readIssue(result.stdout)
    if (issue === undefined) throw new Error('gh issue view printed something that is not an issue')
    return issue
  }

  async subIssues(request: GhIssueLocator): Promise<readonly GhIssue[]> {
    return (await this.restList(request, 'sub_issues')).map((issue) => ({
      body: typeof issue.body === 'string' ? issue.body : '',
      comments: [],
      labels: list(issue.labels).flatMap((label) => (isRecord(label) && typeof label.name === 'string' ? [label.name] : [])),
      number: issue.number,
      ...(issue.state === 'open' || issue.state === 'closed' ? { state: issue.state } : {}),
      title: typeof issue.title === 'string' ? issue.title : '',
      url: issue.html_url,
    }))
  }

  async blockedBy(request: GhIssueLocator): Promise<readonly GhLinkedIssue[]> {
    return (await this.restList(request, 'dependencies/blocked_by')).map((issue) => ({ number: issue.number, url: issue.html_url }))
  }

  /**
   * Every page of one of an issue's REST lists, on the request's host. The
   * REST API is used because `gh issue view` does not show sub-issues or
   * dependencies.
   */
  private async restList(request: GhIssueLocator, list: string): Promise<readonly RestIssue[]> {
    const [host, owner, repo, ...rest] = request.repo.split('/')
    if (host === undefined || owner === undefined || repo === undefined || rest.length > 0) {
      throw new Error(`${request.repo} is not a host/owner/repo repository`)
    }
    const path = `repos/${owner}/${repo}/issues/${String(request.number)}/${list}?per_page=100`
    const args = ['api', '--hostname', host, '--paginate', '--slurp', path]
    const result = await this.process.exec({ args, command: 'gh', cwd: request.cwd })
    if (result.exitCode !== 0) {
      const detail = firstLine(result.stderr) ?? firstLine(result.stdout) ?? `exit code ${String(result.exitCode)}`
      throw new Error(`gh api exited ${String(result.exitCode)}: ${detail}`)
    }
    const issues = readPages(result.stdout)
    if (issues === undefined) throw new Error('gh api printed something that is not a list of issues')
    return issues
  }
}

interface RestIssue extends Readonly<Record<string, unknown>> {
  readonly number: number
  readonly html_url: string
}

/** `--paginate --slurp` prints one array per page inside an outer array. */
function readPages(stdout: string): readonly RestIssue[] | undefined {
  let parsed: unknown
  try {
    parsed = JSON.parse(stdout)
  } catch {
    return undefined
  }
  if (!Array.isArray(parsed) || !parsed.every(Array.isArray)) return undefined
  const issues = (parsed as unknown[][]).flat()
  return issues.every(isRestIssue) ? issues : undefined
}

function isRestIssue(value: unknown): value is RestIssue {
  return isRecord(value) && typeof value.number === 'number' && typeof value.html_url === 'string'
}

function firstLine(text: string): string | undefined {
  return text
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line.length > 0)
}

function readIssue(stdout: string): GhIssue | undefined {
  let parsed: unknown
  try {
    parsed = JSON.parse(stdout)
  } catch {
    return undefined
  }
  if (!isRecord(parsed)) return undefined
  const { body, number, title, url } = parsed
  if (typeof number !== 'number' || typeof title !== 'string' || typeof body !== 'string' || typeof url !== 'string') {
    return undefined
  }
  return {
    body,
    comments: list(parsed.comments).flatMap(readComment),
    labels: list(parsed.labels).flatMap((label) => (isRecord(label) && typeof label.name === 'string' ? [label.name] : [])),
    number,
    title,
    url,
  }
}

function readComment(comment: unknown): GhIssueComment[] {
  if (!isRecord(comment) || typeof comment.body !== 'string') return []
  const author = isRecord(comment.author) && typeof comment.author.login === 'string' ? comment.author.login : 'unknown'
  const createdAt = typeof comment.createdAt === 'string' ? comment.createdAt : ''
  return [{ author, body: comment.body, createdAt }]
}

function list(value: unknown): readonly unknown[] {
  return Array.isArray(value) ? value : []
}

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
