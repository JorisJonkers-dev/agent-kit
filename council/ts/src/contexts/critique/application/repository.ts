import { EMPTY_ALLOWLIST, identifyForRoster, identifyRepository, WORK_ENTRY, parseAllowlist, type AllowlistEntry, type FamilyAllowlist } from '../domain/family-allowlist.js'
import type { Refusal } from '../domain/outcomes.js'
import { readJsonFile } from './decode.js'
import { allowlistPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'

export interface ResolvedRepository {
  readonly status: 'resolved'
  readonly root: string
  readonly commonDir: string
  readonly key: string
}

/** Decides whether a repository may send content to critics, and reads this machine's family allowlist. */
export class RepositoryGate {
  constructor(private readonly deps: Pick<CritiqueDeps, 'env' | 'files' | 'git'>) {}

  /** The repository's allowlist key and git directories, or the refusal its remotes earn. */
  async resolve(cwd: string): Promise<ResolvedRepository | Refusal> {
    const decision = identifyRepository(await this.deps.git.remotes(cwd))
    if (decision.status === 'refused') return decision
    return {
      commonDir: await this.deps.git.commonDir(cwd),
      key: decision.key,
      root: await this.deps.git.root(cwd),
      status: 'resolved',
    }
  }

  /**
   * Like `resolve`, but a `nedap` repository resolves, flagged as work, to
   * the work-only roster rather than being refused; only prepare asks.
   */
  async resolveForRoster(cwd: string): Promise<(ResolvedRepository & { readonly work: boolean }) | Refusal> {
    const decision = identifyForRoster(await this.deps.git.remotes(cwd))
    if (decision.status === 'refused') return decision
    return {
      commonDir: await this.deps.git.commonDir(cwd),
      key: decision.key,
      root: await this.deps.git.root(cwd),
      status: 'resolved',
      work: decision.work,
    }
  }

  /** The entry a work repository has: the work families, with code and history, whatever its file says. */
  workEntry(): AllowlistEntry {
    return WORK_ENTRY
  }

  /** This machine's family allowlist; a missing file is an empty one. */
  async allowlist(): Promise<FamilyAllowlist> {
    const path = allowlistPath(this.deps.env)
    if (!(await this.deps.files.exists(path))) return EMPTY_ALLOWLIST
    return readJsonFile(this.deps.files, path, 'invalid-allowlist', parseAllowlist)
  }
}
