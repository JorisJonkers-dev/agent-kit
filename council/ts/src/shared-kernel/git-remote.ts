/** One remote of a repository, as `git config` names it. */
export interface GitRemote {
  readonly name: string
  readonly url: string
}

/** Where a remote points: its host, owner and repository, all lowercased. */
export interface RemoteLocation {
  readonly host: string
  readonly owner: string
  readonly repo: string
}

/** The GitHub organisation whose repositories never send content to any critic. */
export const DENIED_OWNER = 'nedap'

/**
 * Reads `host`, `owner` and `repo` from an ssh (`git@host:owner/repo.git`,
 * `ssh://git@host/owner/repo`) or https remote URL. Host aliases such as
 * `github-work` stay as written.
 */
export function parseRemote(url: string): RemoteLocation | undefined {
  const scp = /^(?:[^@/\s]+@)?([^:/\s]+):(?!\/\/)(.+)$/.exec(url)
  const standard = /^[a-z+]+:\/\/(?:[^@/]+@)?([^/:]+)(?::\d+)?\/(.+)$/i.exec(url)
  const match = scp ?? standard
  if (match === null) return undefined
  const host = (match[1] ?? '').toLowerCase()
  const segments = (match[2] ?? '')
    .replace(/\/+$/, '')
    .replace(/\.git$/, '')
    .split('/')
    .filter((segment) => segment.length > 0)
  const repo = segments.pop()
  const owner = segments.join('/')
  if (repo === undefined || owner.length === 0) return undefined
  return { host, owner: owner.toLowerCase(), repo: repo.toLowerCase() }
}

/**
 * What a repository's remotes say about it: the allowlist key it is known by,
 * that it belongs to the denied organisation, or that it has no remote at all.
 */
export type RepositoryIdentity =
  | { readonly status: 'denied-owner'; readonly remote: string; readonly owner: string; readonly location: RemoteLocation }
  | { readonly status: 'no-remote' }
  | { readonly status: 'identified'; readonly key: string; readonly location: RemoteLocation }

/**
 * Decides which allowlist key a repository has. Any remote owned by `nedap`,
 * on any host or alias, denies the repository outright; otherwise the key is
 * `origin`'s location, or the first remote's when there is no `origin`.
 */
export function identifyRepository(remotes: readonly GitRemote[]): RepositoryIdentity {
  const located = remotes.flatMap((remote) => {
    const location = parseRemote(remote.url)
    return location === undefined ? [] : [{ name: remote.name, location }]
  })
  const denied = located.find((remote) => remote.location.owner.split('/')[0] === DENIED_OWNER)
  if (denied !== undefined) {
    return { location: denied.location, owner: denied.location.owner, remote: denied.name, status: 'denied-owner' }
  }
  const primary = located.find((remote) => remote.name === 'origin') ?? located[0]
  if (primary === undefined) return { status: 'no-remote' }
  const { host, owner, repo } = primary.location
  return { key: `${host}/${owner}/${repo}`, location: primary.location, status: 'identified' }
}
