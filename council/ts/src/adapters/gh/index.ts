import type { GhIssue, GhIssueComment, GhIssueRequest, GhPort, ProcessPort } from '../../ports/index.js'

/** Reads issues with the `gh` CLI. Any failure rejects with one line that says what gh reported. */
export class GhCliAdapter implements Pick<GhPort, 'viewIssue'> {
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
