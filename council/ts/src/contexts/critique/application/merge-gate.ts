import type { GitCommitLink } from '../../../ports/index.js'
import type { TriageIndexRecord } from '../domain/gate.js'
import { diffOfPaths, differsOnlyInPins, isManifest } from '../domain/merge-commit.js'
import { excludedByPath, isGenerated, type PacketLimits } from '../domain/repository-packet.js'
import { stagedDiffHash, type TicketReviewRecord } from '../domain/ticket-review.js'
import { codeChange, riskSignals, type RiskRules } from '../domain/triage.js'
import type { CritiqueDeps } from './ports.js'

/** What a merge check reads besides git: the repository's ticket reviews and triages, oldest first, and the rules that judge them. */
export interface MergeEvidence {
  readonly root: string
  readonly reviews: readonly TicketReviewRecord[]
  readonly triages: readonly TriageIndexRecord[]
  readonly rules: RiskRules
  readonly packet: PacketLimits
  /** The most commits a merge may bring in and still be checked one by one. */
  readonly limit: number
}

/** How many commits are checked at once: their git calls run side by side. */
const CONCURRENT_CHECKS = 8

/** What a merge check found. */
export type MergeVerdict =
  /** Every commit brought in is vouched for, and the merge adds nothing of its own beyond pins and rebuilt generated files. */
  | { readonly kind: 'vouched' }
  /** Every commit brought in is vouched for; the merge's own changes beyond them are these, to be judged like any change. */
  | { readonly kind: 'own-changes'; readonly diff: string; readonly paths: readonly string[] }
  /** A commit the merge brings in is not vouched for, so the merge is judged whole. */
  | { readonly kind: 'unvouched'; readonly commit: string }
  /** The merge brings in more commits than the limit, so it is judged whole without reading them one by one. */
  | { readonly kind: 'too-many'; readonly count: number }

/**
 * Decides what a merge of `theirs` into `ours` adds that no review has
 * vouched for. A commit it brings in is vouched for when the latest ticket
 * review that names it, by the commit reviewed or by the staged change it
 * was made from, passed and was not degraded; with no such review, when
 * it is a commit the commit gate lets through alone (low-risk), or a merge
 * that itself adds nothing of its own. The merge's own changes are what its
 * result holds beyond the clean auto-merge of the two, apart from sha256
 * pins in a manifest and generated files, which a rebuild rewrites.
 */
export class MergeCheck {
  constructor(private readonly deps: Pick<CritiqueDeps, 'git'>) {}

  /** `result` is the tree the merge commit records; null for the clean auto-merge, which adds nothing of its own. */
  async judge(evidence: MergeEvidence, ours: string, theirs: string, result: string | null): Promise<MergeVerdict> {
    const links = await this.deps.git.commitsBetween(evidence.root, ours, theirs)
    if (links.length > evidence.limit) return { count: links.length, kind: 'too-many' }
    const trees = new Map(links.map((link) => [link.commit, link.tree]))
    for (let start = 0; start < links.length; start += CONCURRENT_CHECKS) {
      const batch = links.slice(start, start + CONCURRENT_CHECKS)
      const vouched = await Promise.all(batch.map((link) => this.vouched(evidence, link, trees)))
      const first = batch.find((_link, n) => vouched[n] !== true)
      if (first !== undefined) return { commit: first.commit, kind: 'unvouched' }
    }
    const own = result === null ? null : await this.ownChanges(evidence, ours, theirs, result)
    return own === null ? { kind: 'vouched' } : { kind: 'own-changes', ...own }
  }

  /** `trees` holds the tree of each commit the merge brings in, so only a parent outside them costs a git call. */
  private async vouched(evidence: MergeEvidence, { commit, parents }: GitCommitLink, trees: ReadonlyMap<string, string>): Promise<boolean> {
    const git = this.deps.git
    const [first, second, ...more] = parents
    // The staged change the commit was made from: its parent's tree, and its own tree as the index held it.
    const staged =
      first === undefined
        ? null
        : stagedDiffHash(trees.get(first) ?? (await git.treeOf(evidence.root, first)), await git.treeEntries(evidence.root, commit))
    const review = evidence.reviews.filter((record) => record.head_commit === commit || (staged !== null && record.staged_diff === staged)).at(-1)
    if (review !== undefined) return review.review === 'pass' && !review.degraded
    if (first === undefined) return false
    if (second !== undefined) return more.length === 0 && (await this.ownChanges(evidence, first, second, commit)) === null
    const [diff, paths] = await Promise.all([git.diffCommits(evidence.root, first, commit), git.changedPaths(evidence.root, first, commit)])
    const labels = evidence.triages.filter((record) => record.staged_on === first).at(-1)?.labels ?? []
    return riskSignals(codeChange(diff, paths), labels, evidence.rules).length === 0
  }

  /** The merge's own changes beyond the clean auto-merge, pins and generated files apart; null when there are none. */
  private async ownChanges(
    evidence: MergeEvidence,
    ours: string,
    theirs: string,
    result: string,
  ): Promise<{ readonly diff: string; readonly paths: readonly string[] } | null> {
    const git = this.deps.git
    const auto = (await git.mergeTree(evidence.root, ours, theirs)).tree
    const own: string[] = []
    for (const path of await git.changedPaths(evidence.root, auto, result)) {
      if (!(await this.rebuilt(evidence, auto, result, path))) own.push(path)
    }
    if (own.length === 0) return null
    return { diff: diffOfPaths(await git.diffCommits(evidence.root, auto, result), own), paths: own }
  }

  /** Whether a path the merge changed beyond the auto-merge is a generated file, or a manifest that differs only in its pins. */
  private async rebuilt(evidence: MergeEvidence, auto: string, result: string, path: string): Promise<boolean> {
    if (excludedByPath(path, evidence.packet) === 'generated') return true
    const [before, after] = await Promise.all([this.textAt(evidence.root, auto, path), this.textAt(evidence.root, result, path)])
    if (isGenerated(after ?? before ?? '')) return true
    return isManifest(path) && before !== undefined && after !== undefined && differsOnlyInPins(before, after)
  }

  /** A file's text in a tree; undefined when the tree does not hold it. */
  private async textAt(root: string, tree: string, path: string): Promise<string | undefined> {
    try {
      return await this.deps.git.fileAt(root, tree, path)
    } catch {
      return undefined
    }
  }
}
