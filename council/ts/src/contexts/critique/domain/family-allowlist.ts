import { DENIED_OWNER, identifyRepository as identify, type GitRemote } from '../../../shared-kernel/index.js'
import { ALLOWLIST_FAMILIES, type AllowlistFamily } from './family.js'
import { JsonFields } from './json.js'
import { refusal, type Refusal } from './outcomes.js'

export type RepositoryDecision = Refusal | { readonly status: 'identified'; readonly key: string }

/**
 * Decides which allowlist key a repository has. Any remote owned by `nedap`,
 * on any host or alias, denies the repository outright; otherwise the key is
 * `origin`'s location, or the first remote's when there is no `origin`.
 */
export function identifyRepository(remotes: readonly GitRemote[]): RepositoryDecision {
  const identity = identify(remotes)
  if (identity.status === 'denied-owner') {
    return refusal('nedap', `remote ${identity.remote} belongs to the ${DENIED_OWNER} organisation; nothing is sent to any critic`)
  }
  if (identity.status === 'no-remote') {
    return refusal('no-remote', 'the repository has no remote, so it counts as not opted in')
  }
  return { key: identity.key, status: 'identified' }
}

/**
 * A repository as the roster sees it: one that is not `nedap`'s keeps the
 * allowlist key it always had, and a `nedap` one gets a work-only roster
 * whatever its entry says, keyed by its own location so its tracker reads.
 */
export type RosterRepositoryDecision = Refusal | { readonly status: 'identified'; readonly key: string; readonly work: boolean }

export function identifyForRoster(remotes: readonly GitRemote[]): RosterRepositoryDecision {
  const identity = identify(remotes)
  if (identity.status !== 'denied-owner') {
    const decision = identifyRepository(remotes)
    return decision.status === 'refused' ? decision : { ...decision, work: false }
  }
  const { host, owner, repo } = identity.location
  return { key: `${host}/${owner}/${repo}`, status: 'identified', work: true }
}

/** What a `nedap` repository's entry is, whatever its allowlist says: the work families, with code and history. */
export const WORK_ENTRY: AllowlistEntry = { code: true, families: ['anthropic', 'openai'], history: true, updated_at: '' }

export interface AllowlistEntry {
  readonly families: readonly AllowlistFamily[]
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

/** Reads the allowlist file; an entry naming a family no client serves does not parse. */
export function parseAllowlist(text: string): FamilyAllowlist {
  const file = JsonFields.parse(text)
  return {
    repositories: file.entries('repositories', (entry) => ({
      code: entry.has('code') ? entry.boolean('code') : false,
      families: entry.oneOfList('families', ALLOWLIST_FAMILIES),
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
