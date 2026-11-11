/**
 * The two ways a command ends without doing its work. A refusal is a normal
 * result: the family allowlist says this repository sends nothing, or its
 * tracker cannot give prepare the artifact. An error means the command could
 * not run: its input, a file it reads, or the run it was pointed at is wrong.
 * The runner prints a refusal and exits 0, and prints an error with its code
 * and exits 1.
 */

export type RefusalReason =
  | 'no-remote'
  | 'nedap'
  | 'not-opted-in'
  /** A code stage, for a repository whose allowlist entry allows documents only. */
  | 'code-not-allowed'
  /** The issue-tracker doc names a tracker the critique cannot read. */
  | 'unsupported-tracker'
  /** gh failed, or the issue could not be read. */
  | 'tracker-unreachable'
  /** The run named as the first round was already a second round. */
  | 'round-limit'
  /** The tickets stage, for a spec that has no tickets to check. */
  | 'no-tickets'
  /** The decisions stage found no decision docs, and the caller gave no decision summary. */
  | 'no-decision-docs'
  /** An ADR review found no decision records to review. */
  | 'no-adrs'
  /** The plan stage found no refactoring plan at the path its draft's tracker convention names. */
  | 'no-plan'
  /** The facts command, for a repository with no commit to read them at. */
  | 'no-commit'
  /** A repository packet whose documents, manifest and facts alone pass routing's packet cap. */
  | 'packet-over-cap'
  /** This machine's model gateway configuration, which the plan limit is read through. */
  | 'invalid-gateway-config'

export interface Refusal {
  readonly status: 'refused'
  readonly reason: RefusalReason
  readonly message: string
}

export function refusal(reason: RefusalReason, message: string): Refusal {
  return { message, reason, status: 'refused' }
}

export type CritiqueErrorCode =
  /** The JSON a command was given. */
  | 'invalid-input'
  /** This machine's family allowlist file. */
  | 'invalid-allowlist'
  /** The shipped or overriding routing configuration. */
  | 'invalid-routing'
  /** A run directory's manifest or merged findings, or a run directory that holds none. */
  | 'invalid-run'
  /** A spec or reference that resolves outside the repository. */
  | 'outside-repository'
  /** A run whose verdicts are already recorded. */
  | 'already-recorded'
  /** A run that already had its refute round. */
  | 'already-refuted'
  /** A run whose rewritten records were already cold-read. */
  | 'already-cold-read'
  /** A line of the user's ledger that record could not have written. */
  | 'invalid-ledger'
  /** This machine's setup results file. */
  | 'invalid-setup'
  /** The tracker could not be read when a feature review's gate needed the spec's tickets. */
  | 'tracker-unreachable'

export class CritiqueError extends Error {
  override readonly name = 'CritiqueError'

  constructor(
    readonly code: CritiqueErrorCode,
    message: string,
  ) {
    super(message)
  }
}
