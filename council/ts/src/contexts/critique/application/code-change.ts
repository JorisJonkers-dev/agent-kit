import { CritiqueError } from '../domain/outcomes.js'
import { codeChange, type CodeChange } from '../domain/triage.js'
import type { CritiqueDeps } from './ports.js'

/** A change between two commits, as git reports it: the commits, the diff and what the risk rules read. */
export interface ReadChange {
  /** Where the change left its base: the merge base of the base and the head. */
  readonly base_commit: string
  readonly head_commit: string
  readonly diff: string
  readonly change: CodeChange
}

/**
 * Reads a change from git alone, with no model and no network: the diff from
 * the merge base of `base` and `head` to `head`, and every path it touches.
 * A code packet and risk triage read a change the same way.
 */
export class CodeChangeReader {
  constructor(private readonly deps: Pick<CritiqueDeps, 'git'>) {}

  async read(root: string, base: string, head: string): Promise<ReadChange> {
    const headCommit = await this.commit(root, 'head', head)
    const baseCommit = await this.deps.git.mergeBase(root, await this.commit(root, 'base', base), headCommit)
    const diff = await this.deps.git.diffCommits(root, baseCommit, headCommit)
    const paths = await this.deps.git.changedPaths(root, baseCommit, headCommit)
    return { base_commit: baseCommit, change: codeChange(diff, paths), diff, head_commit: headCommit }
  }

  private async commit(root: string, field: 'base' | 'head', revision: string): Promise<string> {
    try {
      return await this.deps.git.resolveCommit(root, revision)
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error)
      throw new CritiqueError('invalid-input', `${field} ${revision} names no commit: ${detail}`)
    }
  }
}
