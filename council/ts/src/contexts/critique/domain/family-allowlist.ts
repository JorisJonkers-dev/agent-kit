import type { GitRemote } from '../../../shared-kernel/index.js'
import { MODEL_FAMILIES, type ModelFamily } from './family.js'
import { JsonFields } from './json.js'
import { refusal, type Refusal } from './outcomes.js'

/** The GitHub organisation whose repositories never send content to any critic. */
const DENIED_OWNER = 'nedap'

export interface RemoteLocation {
  readonly host: string
  readonly owner: string
  readonly repo: string
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
  /**
   * Whether the repository may send code, not only documents: the code
   * stages refuse while it is false. An entry recorded before the
   * permission existed has none, and reads as false.
   */
  readonly code: boolean
  /**
   * Whether the repository may send its history, commit subjects and bodies
   * and issue text: a stage that wants history leaves it out while it is
   * false. An entry recorded before the permission existed reads as false.
   */
  readonly history: boolean
  readonly updated_at: string
}

export interface FamilyAllowlist {
  readonly version: 1
  readonly repositories: Readonly<Record<string, AllowlistEntry>>
}

export const EMPTY_ALLOWLIST: FamilyAllowlist = { version: 1, repositories: {} }

/** Reads the allowlist file; an entry naming a family no critic comes from does not parse. */
export function parseAllowlist(text: string): FamilyAllowlist {
  const file = JsonFields.parse(text)
  return {
    repositories: file.entries('repositories', (entry) => ({
      code: entry.has('code') ? entry.boolean('code') : false,
      families: entry.oneOfList('families', MODEL_FAMILIES),
      history: entry.has('history') ? entry.boolean('history') : false,
      updated_at: entry.string('updated_at'),
    })),
    version: file.literal('version', 1),
  }
}

export function withEntry(
  allowlist: FamilyAllowlist,
  key: string,
  permission: Pick<AllowlistEntry, 'families' | 'code' | 'history'>,
  at: Date,
): FamilyAllowlist {
  return {
    version: 1,
    repositories: {
      ...allowlist.repositories,
      [key]: { code: permission.code, families: permission.families, history: permission.history, updated_at: at.toISOString() },
    },
  }
}
