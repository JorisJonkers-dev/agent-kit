import { isObject, ModelGatewayError } from '../domain/config.js'
import { messageOf } from './json-file.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

/** One worktree `worktrees clean` removed. */
export interface RemovedWorktree {
  readonly run_id: string
  readonly workflow: string
  readonly worktree: string
  readonly branch: string
}

/** One worktree `worktrees clean` could not remove, and why; it stays on the kept list. */
export interface KeptFailure {
  readonly run_id: string
  readonly worktree: string
  readonly reason: string
}

/** A branch left behind although its worktree was removed (it is checked out elsewhere, or already gone), with git's message. */
export interface KeptBranch {
  readonly run_id: string
  readonly branch: string
  readonly reason: string
}

export interface WorktreesCleanResult {
  readonly status: 'ok'
  readonly removed: readonly RemovedWorktree[]
  readonly failed: readonly KeptFailure[]
  /** Removed worktrees whose branch git would not delete; they are off the kept list all the same. */
  readonly branch_kept: readonly KeptBranch[]
}

/** Which kept worktrees to remove: those of the workflow that ended, or all of them. */
type CleanSelection = { readonly workflow: string } | { readonly all: true }

function readSelection(raw: unknown): CleanSelection {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
  const named = raw.workflow !== undefined
  const all = raw.all !== undefined
  if (named && all) throw new ModelGatewayError('invalid-input', 'worktrees clean takes the workflow that ended, or all, not both')
  if (named) {
    if (typeof raw.workflow !== 'string' || raw.workflow === '') {
      throw new ModelGatewayError('invalid-input', 'workflow must be the non-empty name of the workflow that ended')
    }
    return { workflow: raw.workflow }
  }
  if (raw.all !== true) {
    throw new ModelGatewayError('invalid-input', 'worktrees clean needs "workflow", the workflow that ended, or "all": true')
  }
  return { all: true }
}

/**
 * Removes the writing worktrees the gateway kept for a workflow that has
 * ended: each worktree and the branch it was on, which the caller has taken
 * what it wanted from by then. One it cannot remove stays on the list and is
 * reported, so a later call tries it again. Once the worktree is gone the
 * entry is too: a branch git will not delete (checked out elsewhere, or
 * already gone) is reported in `branch_kept`, not retried for ever.
 */
export class WorktreesCleanCommand implements ModelGatewayCommand<unknown, WorktreesCleanResult> {
  constructor(private readonly deps: ModelGatewayDependencies) {}

  async run(raw: unknown): Promise<WorktreesCleanResult> {
    const selection = readSelection(raw)
    const chosen = (await this.deps.kept.list()).filter((worktree) => 'all' in selection || worktree.workflow === selection.workflow)
    const removed: RemovedWorktree[] = []
    const failed: KeptFailure[] = []
    const branchKept: KeptBranch[] = []
    for (const worktree of chosen) {
      try {
        await this.deps.git.removeWorktree(worktree.repo, worktree.path)
      } catch (error) {
        failed.push({ reason: messageOf(error), run_id: worktree.run_id, worktree: worktree.path })
        continue
      }
      removed.push({ branch: worktree.branch, run_id: worktree.run_id, workflow: worktree.workflow, worktree: worktree.path })
      try {
        await this.deps.git.deleteBranch(worktree.repo, worktree.branch)
      } catch (error) {
        branchKept.push({ branch: worktree.branch, reason: messageOf(error), run_id: worktree.run_id })
      }
    }
    await this.deps.kept.remove(removed.map((worktree) => worktree.run_id))
    return { branch_kept: branchKept, failed, removed, status: 'ok' }
  }
}
