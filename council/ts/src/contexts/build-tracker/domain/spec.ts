import { isAbsolute, join } from 'node:path'

/** A repository on a git host. */
export interface RepositoryName {
  readonly host: string
  readonly owner: string
  readonly repo: string
}

/** What a `--spec` argument names, before the repository it lives in is known. */
export type SpecArgument =
  | { readonly kind: 'issue'; readonly number: number; readonly repository: RepositoryName | undefined }
  | { readonly kind: 'path'; readonly path: string }

export const GITHUB_HOST = 'github.com'

/**
 * Reads `12`, `#12`, `owner/repo#12`, `host/owner/repo#12` or an issue URL as
 * an issue; a bare number names an issue of the current repository. Anything
 * else is a path to a local markdown spec.
 */
export function parseSpecArgument(text: string): SpecArgument {
  const issue = issueReference(text.trim())
  return issue ?? { kind: 'path', path: text }
}

function issueReference(text: string): SpecArgument | undefined {
  const local = /^#?([1-9]\d*)$/.exec(text)
  if (local !== null) return { kind: 'issue', number: Number(local[1]), repository: undefined }
  const named = /^(?:([\w.-]+)\/)?([\w.-]+)\/([\w.-]+)#([1-9]\d*)$/.exec(text)
  if (named !== null) return issue(named[1] ?? GITHUB_HOST, named[2], named[3], named[4])
  const url = /^https?:\/\/([^/\s]+)\/([^/\s]+)\/([^/\s]+)\/issues\/([1-9]\d*)(?:[/?#]\S*)?$/i.exec(text)
  return url === null ? undefined : issue(url[1], url[2], url[3], url[4])
}

function issue(host = '', owner = '', repo = '', number = ''): SpecArgument {
  return { kind: 'issue', number: Number(number), repository: { host: host.toLowerCase(), owner, repo } }
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

/** `<base>/<host>/<owner>/<repo>/<spec>/state.json`, with `page.html` beside it. */
export function stateLocation(base: string, spec: ResolvedSpec): StateLocation {
  const { host, owner, repo } = spec.repository
  const name = spec.kind === 'github' ? String(spec.number) : specSlug(spec.path)
  const dir = join(base, host.toLowerCase(), owner.toLowerCase(), repo.toLowerCase(), name)
  return { page: join(dir, 'page.html'), state: join(dir, 'state.json') }
}
