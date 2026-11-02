import { JsonFields } from './json.js'
import { sha256 } from './manifest.js'

/**
 * The hash that names one staged change: the tree of the commit it is
 * staged on and every staged entry (mode, object id and path). Restaging
 * any file, or staging on another commit, changes it; two reads of the same
 * index give the same hash. Neither git's diff settings nor the working
 * tree enter it, so a commit gate computes it the same way prepare did.
 */
export function stagedDiffHash(headTree: string, stagedEntries: string): string {
  return sha256(`staged-diff v1\0${headTree}\0${stagedEntries}`)
}

/** What a ticket review decided. */
export const TICKET_REVIEWS = ['pass', 'fail', 'degraded'] as const
export type TicketReview = (typeof TICKET_REVIEWS)[number]

/** A critic that did not report: its id, and its failure (or its status, for a critic that was skipped or discarded). */
export interface CriticFailureLine {
  readonly critic_id: string
  readonly reason: string
}

/**
 * A recorded ticket review, written so a commit gate can read it without
 * any model call: `review.json` in the run, and one line of the critique
 * directory's review index.
 */
export interface TicketReviewRecord {
  readonly version: 1
  readonly stage: 'ticket-review'
  readonly run_id: string
  readonly recorded_at: string
  readonly repository: string
  /** The spec's reference, as the packet's spec file names it. */
  readonly spec: string
  /** The ticket's reference, as the manifest's artifact names it. */
  readonly ticket: string
  readonly base_commit: string
  /** The commit the critics read: for a staged review, a commit made of the index. */
  readonly head_commit: string
  /** The hash of the staged change the critics read; null when the review was of a commit. */
  readonly reviewed_staged_diff: string | null
  /**
   * The hash of the staged change when the review was recorded, after the
   * accepted findings were fixed and their tests kept: the change this
   * review vouches for, and the one a commit gate matches. Null when the
   * review was of a commit.
   */
  readonly staged_diff: string | null
  /**
   * `fail` while an accepted critical finding is unfixed; `degraded` when no
   * critic reported, so nobody reviewed the change and the commit gate stays
   * shut until a run in which a critic reported.
   */
  readonly review: TicketReview
  /** True when no critic produced a usable review. */
  readonly degraded: boolean
  /** Every critic of the run that did not report, and why; empty when all reported. */
  readonly critic_failures: readonly CriticFailureLine[]
  /** The accepted critical findings not yet fixed. */
  readonly unfixed_critical: readonly string[]
}

/** Reads one review record, as `review.json` or a line of the review index holds it. */
export function parseTicketReviewRecord(text: string): TicketReviewRecord {
  const file = JsonFields.parse(text)
  return {
    base_commit: file.string('base_commit'),
    // Records written before #259 carry no critic failures.
    critic_failures: file.has('critic_failures') ? file.list('critic_failures', readCriticFailure) : [],
    degraded: file.boolean('degraded'),
    head_commit: file.string('head_commit'),
    recorded_at: file.string('recorded_at'),
    repository: file.string('repository'),
    review: file.oneOf('review', TICKET_REVIEWS),
    reviewed_staged_diff: file.nullableString('reviewed_staged_diff'),
    run_id: file.string('run_id'),
    spec: file.string('spec'),
    stage: file.literal('stage', 'ticket-review'),
    staged_diff: file.nullableString('staged_diff'),
    ticket: file.string('ticket'),
    unfixed_critical: file.strings('unfixed_critical'),
    version: file.literal('version', 1),
  }
}

function readCriticFailure(fields: JsonFields): CriticFailureLine {
  return { critic_id: fields.string('critic_id'), reason: fields.string('reason') }
}
