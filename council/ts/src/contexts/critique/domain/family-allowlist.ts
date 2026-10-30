import { MODEL_FAMILIES, type ModelFamily } from './family.js'
import { isOneOf } from './json.js'
import { CritiqueError, refusal, type Refusal } from './outcomes.js'

/** The GitHub organisation whose repositories never send content to any critic. */
const DENIED_OWNER = 'nedap'

export interface GitRemote {
  readonly name: string
  readonly url: string
}

export interface RemoteLocation {
  readonly host: string
  readonly owner: string
  readonly repo: string
}

export function parseModelFamilies(value: readonly string[]): readonly ModelFamily[] {
  return value.map((family) => {
    if (!isOneOf(MODEL_FAMILIES, family)) throw new CritiqueError('invalid-input', `unknown model family: ${family}`)
    return family
  })
}

/**
 * Reads `host`, `owner` and `repo` from an ssh (`git@host:owner/repo.git`,
 * `ssh://git@host/owner/repo`) or https remote URL. Host aliases such as
 * `github-work` stay as written.
 */
export function parseRemote(url: string): RemoteLocation | undefined {
  const scp = /^[^@/\s]+@([^:/\s]+):(.+)$/.exec(url)
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

export type RepositoryDecision = Refusal | { readonly status: 'identified'; readonly key: string }

/**
 * Decides which allowlist key a repository has. Any remote owned by `nedap`,
 * on any host or alias, denies the repository outright; otherwise the key is
 * `origin`'s location, or the first remote's when there is no `origin`.
 */
export function identifyRepository(remotes: readonly GitRemote[]): RepositoryDecision {
  const located = remotes.flatMap((remote) => {
    const location = parseRemote(remote.url)
    return location === undefined ? [] : [{ name: remote.name, location }]
  })
  const denied = located.find((remote) => remote.location.owner.split('/')[0] === DENIED_OWNER)
  if (denied !== undefined) {
    return refusal('nedap', `remote ${denied.name} belongs to the ${DENIED_OWNER} organisation; nothing is sent to any critic`)
  }
  const primary = located.find((remote) => remote.name === 'origin') ?? located[0]
  if (primary === undefined) {
    return refusal('no-remote', 'the repository has no remote, so it counts as not opted in')
  }
  const { host, owner, repo } = primary.location
  return { key: `${host}/${owner}/${repo}`, status: 'identified' }
}

export interface AllowlistEntry {
  readonly families: readonly ModelFamily[]
  readonly updated_at: string
}

export interface FamilyAllowlist {
  readonly version: 1
  readonly repositories: Readonly<Record<string, AllowlistEntry>>
}

export const EMPTY_ALLOWLIST: FamilyAllowlist = { version: 1, repositories: {} }

export function parseAllowlist(text: string): FamilyAllowlist {
  return JSON.parse(text) as FamilyAllowlist
}

export function withEntry(
  allowlist: FamilyAllowlist,
  key: string,
  families: readonly ModelFamily[],
  at: Date,
): FamilyAllowlist {
  return {
    version: 1,
    repositories: { ...allowlist.repositories, [key]: { families, updated_at: at.toISOString() } },
  }
}
