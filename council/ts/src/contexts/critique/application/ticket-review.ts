import { dirname, join } from 'node:path'

import { specReference, type RunManifest } from '../domain/manifest.js'
import { CritiqueError } from '../domain/outcomes.js'
import type { TicketReviewRecord } from '../domain/ticket-review.js'
import { unfixedCritical, type AdjudicatedRun } from '../domain/verdicts.js'
import type { CritiqueDeps } from './ports.js'
import { StagedDiffReader } from './staged-change.js'

/** The review record a ticket-review run holds, in the run directory. */
export function ticketReviewPath(runDir: string): string {
  return join(runDir, 'review.json')
}

/**
 * Every recorded ticket review of the repository, one JSON line per run in
 * the critique directory of the git common directory, oldest first: the last
 * line whose `staged_diff` matches the change about to be committed is that
 * change's review.
 */
export function ticketReviewIndexPath(critiqueDir: string): string {
  return join(critiqueDir, 'ticket-reviews.jsonl')
}

/**
 * Records what a ticket review decided, for a commit gate to read without a
 * model call. A review fails while an accepted critical finding is unfixed.
 * A review of the staged change also hashes the index as it is at record
 * time, after the fixes and the kept regression tests were staged: that is
 * the change the review vouches for.
 */
export class TicketReviewLog {
  private readonly staged: StagedDiffReader

  constructor(private readonly deps: Pick<CritiqueDeps, 'clock' | 'files' | 'git'>) {
    this.staged = new StagedDiffReader(deps)
  }

  async compute(manifest: RunManifest, run: AdjudicatedRun): Promise<TicketReviewRecord> {
    const code = manifest.code
    if (code === undefined) throw new CritiqueError('invalid-run', `run ${manifest.run_id} is a ticket review without the commits it reviewed`)
    const reviewed = code.staged_diff ?? null
    const unfixed = unfixedCritical(run).map((finding) => finding.id)
    return {
      base_commit: code.base_commit,
      degraded: run.degraded,
      head_commit: code.head_commit,
      recorded_at: this.deps.clock.now().toISOString(),
      repository: manifest.repository,
      review: unfixed.length === 0 ? 'pass' : 'fail',
      reviewed_staged_diff: reviewed,
      run_id: manifest.run_id,
      spec: specReference(manifest),
      stage: 'ticket-review',
      staged_diff: reviewed === null ? null : (await this.staged.read(manifest.repo_root)).sha256,
      ticket: manifest.artifact.reference,
      unfixed_critical: unfixed,
      version: 1,
    }
  }

  /** Writes the record into the run and appends it to the repository's review index. */
  async save(manifest: RunManifest, record: TicketReviewRecord): Promise<void> {
    await this.deps.files.writeText(ticketReviewPath(manifest.run_dir), `${JSON.stringify(record, null, 2)}\n`)
    await this.deps.files.appendText(ticketReviewIndexPath(dirname(manifest.run_dir)), `${JSON.stringify(record)}\n`)
  }
}
