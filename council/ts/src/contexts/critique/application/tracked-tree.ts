import { excludedByPath, isGenerated, type ExclusionKind, type PacketLimits, type TrackedFile } from '../domain/repository-packet.js'
import type { CritiqueDeps } from './ports.js'

/** How many files of the tree are read from git at once. */
const READ_CONCURRENCY = 16

/** A commit's tracked files a packet may send, and how many each exclusion left out. */
export interface TrackedTree {
  /** Every tracked path not excluded, sorted, with its text, or null for a binary file. */
  readonly tracked: readonly TrackedFile[]
  readonly excluded: Record<ExclusionKind, number>
}

/**
 * Reads every tracked path at a commit from git's object store (so no
 * `.git` and no worktree metadata), sorted. A path routing's exclusion globs
 * match is counted and never read; a file whose first lines say a tool
 * generated it is counted and left out; a file holding a NUL byte is binary,
 * kept without its text.
 */
export async function readTrackedTree(deps: Pick<CritiqueDeps, 'git'>, root: string, commit: string, limits: PacketLimits): Promise<TrackedTree> {
  const excluded: Record<ExclusionKind, number> = { generated: 0, lock: 0, vendored: 0 }
  const paths = (await deps.git.treeFiles(root, commit)).filter((path) => {
    const kind = excludedByPath(path, limits)
    if (kind !== undefined) excluded[kind] += 1
    return kind === undefined
  })
  const tracked: TrackedFile[] = []
  for (let start = 0; start < paths.length; start += READ_CONCURRENCY) {
    const batch = paths.slice(start, start + READ_CONCURRENCY)
    const texts = await Promise.all(batch.map((path) => deps.git.fileAt(root, commit, path)))
    batch.forEach((path, i) => {
      const text = texts[i] ?? ''
      if (text.includes('\u0000')) tracked.push({ path, text: null })
      else if (isGenerated(text)) excluded.generated += 1
      else tracked.push({ path, text })
    })
  }
  return { excluded, tracked }
}
