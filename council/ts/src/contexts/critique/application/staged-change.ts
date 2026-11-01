import type { JsonFields } from '../domain/json.js'
import { CritiqueError } from '../domain/outcomes.js'
import type { CritiqueStage } from '../domain/stage.js'
import { stagedDiffHash } from '../domain/ticket-review.js'
import type { CritiqueDeps } from './ports.js'

/** A staged change: the commit it is staged on, and the hash that names it. */
export interface StagedDiff {
  /** HEAD when the change was read. */
  readonly head_commit: string
  /** `stagedDiffHash` of HEAD's tree and the staged entries. */
  readonly sha256: string
}

/** A staged change made into a commit a code stage can review. */
export interface StagedSnapshot extends StagedDiff {
  /** A commit holding the index, with HEAD as its parent; no ref points at it. */
  readonly commit: string
}

const SNAPSHOT_MESSAGE = 'critique: the staged change under review'

/**
 * Reads the change staged in a repository's index. `read` writes nothing,
 * so a commit gate can call it on every commit; `snapshot` also writes the
 * index as a tree and a commit object, so the change can be reviewed like
 * any commit, and still moves no ref, index entry or file.
 */
export class StagedDiffReader {
  constructor(private readonly deps: Pick<CritiqueDeps, 'git'>) {}

  async read(root: string): Promise<StagedDiff> {
    const git = this.deps.git
    let head: string
    try {
      head = await git.resolveCommit(root, 'HEAD')
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error)
      throw new CritiqueError('invalid-input', `staged needs a commit at HEAD to diff the index against: ${detail}`)
    }
    const [tree, entries] = await Promise.all([git.treeOf(root, head), git.stagedEntries(root)])
    return { head_commit: head, sha256: stagedDiffHash(tree, entries) }
  }

  async snapshot(root: string): Promise<StagedSnapshot> {
    const staged = await this.read(root)
    const tree = await this.deps.git.writeIndexTree(root)
    return { ...staged, commit: await this.deps.git.commitTree(root, tree, staged.head_commit, SNAPSHOT_MESSAGE) }
  }
}

/**
 * Whether a command's input asks for the staged change: the index, made
 * into a commit on HEAD. It stands in for `head`, and only a ticket review,
 * which runs before its ticket is committed, takes it.
 */
export function stagedInput(fields: JsonFields, stage: CritiqueStage): boolean {
  if (!fields.has('staged') || !fields.boolean('staged')) return false
  if (stage !== 'ticket-review') throw new CritiqueError('invalid-input', `only ticket-review takes staged; ${stage} reviews commits`)
  if (fields.has('head')) throw new CritiqueError('invalid-input', 'staged reviews the index: leave out head')
  return true
}
