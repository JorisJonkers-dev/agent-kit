import { basename, dirname, isAbsolute, join } from 'node:path'

/** A repository on a git host. Make one with `repositoryName`, which reads the host in lower case. */
export interface RepositoryName {
  readonly host: string
  readonly owner: string
  readonly repo: string
}

export const GITHUB_HOST = 'github.com'

/** A host name ignores case, so the host is kept in lower case; owner and repo keep the case they were written in. */
export function repositoryName(host: string, owner: string, repo: string): RepositoryName {
  return { host: host.toLowerCase(), owner, repo }
}

/** Whether two names are one repository: a git host ignores the case of owner and repo names. */
export function sameRepository(left: RepositoryName, right: RepositoryName): boolean {
  return left.host === right.host && left.owner.toLowerCase() === right.owner.toLowerCase() && left.repo.toLowerCase() === right.repo.toLowerCase()
}

/** An issue in a known repository. */
export interface IssueReference {
  readonly repository: RepositoryName
  readonly number: number
}

/** `owner/repo#12`, or `host/owner/repo#12` on a host other than github.com. */
const ISSUE_REFERENCE = /^(?:([\w.-]+)\/)?([\w.-]+)\/([\w.-]+)#([1-9]\d*)$/

/** An issue's or pull request's page: `https://host/owner/repo/issues/12`, or `/pull/12`. */
const ISSUE_URL = /^https?:\/\/([^/\s]+)\/([^/\s]+)\/([^/\s]+)\/(issues|pull)\/([1-9]\d*)(?:[/?#]\S*)?$/i

/** Reads `owner/repo#12` or `host/owner/repo#12`. */
export function parseIssueReference(text: string): IssueReference | undefined {
  const match = ISSUE_REFERENCE.exec(text)
  if (match === null) return undefined
  return { number: Number(match[4]), repository: repositoryName(match[1] ?? GITHUB_HOST, String(match[2]), String(match[3])) }
}

/** Reads an issue's page address or a pull request's, and says which it is. */
export function parseIssueUrl(url: string): (IssueReference & { readonly pull: boolean }) | undefined {
  const match = ISSUE_URL.exec(url)
  if (match === null) return undefined
  return {
    number: Number(match[5]),
    pull: String(match[4]).toLowerCase() === 'pull',
    repository: repositoryName(String(match[1]), String(match[2]), String(match[3])),
  }
}

/** What a `--spec` argument names, before the repository it lives in is known. */
export type SpecArgument =
  | { readonly kind: 'issue'; readonly number: number; readonly repository: RepositoryName | undefined }
  | { readonly kind: 'path'; readonly path: string }

/**
 * Reads `12`, `#12`, `owner/repo#12`, `host/owner/repo#12` or an issue URL as
 * an issue; a bare number names an issue of the current repository. Anything
 * else is a path to a local markdown spec.
 */
export function parseSpecArgument(argument: string): SpecArgument {
  const text = argument.trim()
  const local = /^#?([1-9]\d*)$/.exec(text)
  if (local !== null) return { kind: 'issue', number: Number(local[1]), repository: undefined }
  const url = parseIssueUrl(text)
  const issue = parseIssueReference(text) ?? (url?.pull === false ? url : undefined)
  return issue === undefined ? { kind: 'path', path: argument } : { kind: 'issue', number: issue.number, repository: issue.repository }
}

/** `owner/repo#12`, with the host in front when it is not github.com. */
export function issueReferenceText(repository: RepositoryName, number: number): string {
  const name = `${repository.owner}/${repository.repo}#${String(number)}`
  return repository.host === GITHUB_HOST ? name : `${repository.host}/${name}`
}

/** A spec whose repository is known: where its state lives and how the state names it. */
export type ResolvedSpec =
  | { readonly kind: 'github'; readonly repository: RepositoryName; readonly number: number }
  | {
      readonly kind: 'local'
      readonly repository: RepositoryName
      /** The spec file's path under the repository root, with `/` separators. */
      readonly path: string
    }

/** How the state file and the command output name the spec. */
export function specText(spec: ResolvedSpec): string {
  return spec.kind === 'github' ? issueReferenceText(spec.repository, spec.number) : spec.path
}

/** A repository with no remote is named after its main working tree: `local/<parent>/<name>`. */
export function localRepositoryName(mainRoot: string): RepositoryName {
  return repositoryName('local', basename(dirname(mainRoot)) || 'local', basename(mainRoot) || 'local')
}

/** Where the page links a local ticket's file: the remote's file view, or the working tree on disk. */
export type TicketFiles = { readonly kind: 'remote'; readonly repository: RepositoryName } | { readonly kind: 'disk'; readonly root: string }

/** The hosts whose file view is `https://<host>/<owner>/<repo>/blob/<ref>/<path>`. */
const FILE_VIEW_HOSTS: readonly string[] = [GITHUB_HOST]

/** The remote's file view when the remote is on a host whose file view is known, else the working tree. */
export function ticketFiles(root: string, remote: RepositoryName | undefined): TicketFiles {
  return remote !== undefined && FILE_VIEW_HOSTS.includes(remote.host) ? { kind: 'remote', repository: remote } : { kind: 'disk', root }
}

/**
 * The address of a file under the repository root on the remote's file view;
 * undefined for files only on disk, since a published page must carry no
 * local path (it would show the machine's user name and folders).
 */
export function fileLink(files: TicketFiles, path: string): string | undefined {
  if (files.kind === 'disk') return undefined
  const { host, owner, repo } = files.repository
  return `https://${host}/${owner}/${repo}/blob/HEAD/${path.split('/').map(encodeURIComponent).join('/')}`
}

/**
 * A local spec path as one directory name: lower case, every run of other
 * characters one `-`, then a short hash of the exact path, so two paths that
 * read alike (`v1.2.md`, `v1_2.md`) never share a state.
 */
export function specSlug(path: string): string {
  const readable = path
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  return `${readable}-${pathHash(path)}`
}

/** FNV-1a over the path's UTF-16 code units, as 8 hex digits. */
function pathHash(path: string): string {
  let hash = 0x811c9dc5
  for (let index = 0; index < path.length; index += 1) {
    hash ^= path.charCodeAt(index)
    hash = Math.imul(hash, 0x01000193) >>> 0
  }
  return hash.toString(16).padStart(8, '0')
}

/**
 * Where every state lives: `$XDG_STATE_HOME/agent-kit/build-tracker`, or
 * `~/.local/state/agent-kit/build-tracker` when XDG_STATE_HOME is unset or
 * not an absolute path (as the XDG spec says to ignore it then). Without an
 * absolute HOME either there is none: the caller refuses rather than fall
 * back to the working directory.
 */
export function stateBase(env: Readonly<Record<string, string | undefined>>): string | undefined {
  const xdg = env.XDG_STATE_HOME
  if (xdg !== undefined && isAbsolute(xdg)) return join(xdg, 'agent-kit', 'build-tracker')
  const home = env.HOME
  if (home === undefined || !isAbsolute(home)) return undefined
  return join(home, '.local', 'state', 'agent-kit', 'build-tracker')
}

export interface StateLocation {
  readonly state: string
  readonly page: string
}

/** `<base>/<host>/<owner>/<repo>/<spec>/state.json`, with `page.html` beside it; owner and repo in lower case, as the host ignores their case. */
export function stateLocation(base: string, spec: ResolvedSpec): StateLocation {
  const { host, owner, repo } = spec.repository
  const name = spec.kind === 'github' ? String(spec.number) : specSlug(spec.path)
  const dir = join(base, host, owner.toLowerCase(), repo.toLowerCase(), name)
  return { page: join(dir, 'page.html'), state: join(dir, 'state.json') }
}
