/**
 * Where a repository keeps its specs and tickets, read the way Pocock's
 * `setup-matt-pocock-skills` records it, and how one spec reference names an
 * artifact there.
 */

export const TRACKER_KINDS = ['github', 'local-markdown'] as const
export type TrackerKind = (typeof TRACKER_KINDS)[number]

/** The doc Pocock's setup writes to say where a repository's issues live. */
export const TRACKER_DOC = 'docs/agents/issue-tracker.md'

export type TrackerChoice =
  | { readonly status: 'known'; readonly kind: TrackerKind }
  | { readonly status: 'unsupported'; readonly name: string }

/**
 * Reads the tracker from the doc's `# Issue tracker: <name>` heading. A
 * tracker the critique cannot read (GitLab, or the free-form description
 * Pocock's setup writes for any other) is unsupported, and so is a doc with no
 * such heading.
 */
export function trackerFromDoc(text: string): TrackerChoice {
  const name = /^#\s+Issue tracker:\s*(.+?)\s*$/im.exec(text)?.[1]
  const normalised = name?.toLowerCase()
  if (normalised === 'github') return { kind: 'github', status: 'known' }
  if (normalised === 'local markdown') return { kind: 'local-markdown', status: 'known' }
  return { name: name ?? 'no tracker', status: 'unsupported' }
}

/** With no tracker doc, a repository whose allowlist key is on github.com tracks on GitHub; any other on local markdown. */
export function defaultTracker(repositoryKey: string): TrackerKind {
  return repositoryKey.startsWith('github.com/') ? 'github' : 'local-markdown'
}

export interface IssueReference {
  /** The issue's repository as `host/owner/repo`, the form of a family allowlist key. */
  readonly repository: string
  readonly number: number
}

/**
 * Reads `167`, `#167`, `owner/repo#167` or an issue URL. A bare number names
 * an issue of `repositoryKey`. Anything else is not an issue reference.
 */
export function parseIssueReference(reference: string, repositoryKey: string): IssueReference | undefined {
  const text = reference.trim()
  const local = /^#?(\d+)$/.exec(text)
  if (local !== null) return { number: Number(local[1]), repository: repositoryKey }
  const short = /^([\w.-]+)\/([\w.-]+)#(\d+)$/.exec(text)
  if (short !== null) return { number: Number(short[3]), repository: key('github.com', short[1], short[2]) }
  const url = /^https?:\/\/([^/\s]+)\/([^/\s]+)\/([^/\s]+)\/issues\/(\d+)(?:[/?#]\S*)?$/i.exec(text)
  if (url !== null) return { number: Number(url[4]), repository: key(url[1], url[2], url[3]) }
  return undefined
}

function key(host = '', owner = '', repo = ''): string {
  return `${host}/${owner}/${repo}`.toLowerCase()
}

export interface IssueText {
  readonly title: string
  readonly body: string
  readonly comments: readonly { readonly author: string; readonly createdAt: string; readonly body: string }[]
}

/**
 * The issue as the packet holds it: its title as a heading, then its body,
 * then each comment under `## Comments`. Line endings become `\n`, so a quote
 * a critic copies from the packet matches it.
 */
export function renderIssue(issue: IssueText): string {
  const body = issue.body.trim()
  const parts = body.length > 0 ? [`# ${issue.title}`, body] : [`# ${issue.title}`]
  if (issue.comments.length > 0) {
    parts.push('## Comments', ...issue.comments.map((comment) => `### ${comment.author}, ${comment.createdAt}\n\n${comment.body.trim()}`))
  }
  return `${parts.join('\n\n').replace(/\r\n?/g, '\n')}\n`
}
