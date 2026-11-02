import { filterDiff, type LeftOutFile } from '../domain/diff-filter.js'
import { CritiqueError } from '../domain/outcomes.js'
import { exclusionOf, exclusionReason, type PacketLimits } from '../domain/repository-packet.js'
import { codeChange, type CodeChange } from '../domain/triage.js'
import type { CritiqueDeps } from './ports.js'

/** A change between two commits, as git reports it: the commits, the diff and what the risk rules read. */
export interface ReadChange {
  /** Where the change left its base: the merge base of the base and the head. */
  readonly base_commit: string
  readonly head_commit: string
  /** The diff, without the hunks of the files `left_out` names. */
  readonly diff: string
  /** Every path is touched; only the lines of files not left out are counted. */
  readonly change: CodeChange
  /** The generated, vendored and lock files whose hunks the diff leaves out. */
  readonly left_out: readonly LeftOutFile[]
}

/**
 * Reads a change from git alone, with no model and no network: the diff from
 * the merge base of `base` and `head` to `head`, and every path it touches.
 * A file routing's exclusion globs match, or whose first lines say a tool
 * generated it (at the head, or at the base for a deleted file), keeps one
 * line in the diff instead of its hunks and adds nothing to the changed-line
 * count. A code packet and risk triage read a change the same way.
 */
export class CodeChangeReader {
  constructor(private readonly deps: Pick<CritiqueDeps, 'git'>) {}

  async read(root: string, base: string, head: string, limits: PacketLimits): Promise<ReadChange> {
    const headCommit = await this.commit(root, 'head', head)
    const baseCommit = await this.deps.git.mergeBase(root, await this.commit(root, 'base', base), headCommit)
    const raw = await this.deps.git.diffCommits(root, baseCommit, headCommit)
    const paths = await this.deps.git.changedPaths(root, baseCommit, headCommit)
    const reasons = new Map<string, string>()
    for (const path of paths) {
      const text = exclusionOf(path, undefined, limits) === undefined ? await this.textOf(root, headCommit, baseCommit, path) : undefined
      const exclusion = exclusionOf(path, text, limits)
      if (exclusion !== undefined) reasons.set(path, exclusionReason(exclusion))
    }
    const { diff, left_out: leftOut } = filterDiff(raw, (path) => reasons.get(path))
    return { base_commit: baseCommit, change: codeChange(diff, paths), diff, head_commit: headCommit, left_out: leftOut }
  }

  /** The path's text at the head, or at the base when the change deleted it. */
  private async textOf(root: string, head: string, base: string, path: string): Promise<string> {
    try {
      return await this.deps.git.fileAt(root, head, path)
    } catch {
      return this.deps.git.fileAt(root, base, path)
    }
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
