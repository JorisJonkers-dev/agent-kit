/**
 * The two ways a command ends without doing its work. A refusal is a normal
 * result: the family allowlist says this repository sends nothing. An error
 * means the command could not run: its input, a file it reads, or the run it
 * was pointed at is wrong. The runner prints a refusal and exits 0, and
 * prints an error with its code and exits 1.
 */

export type RefusalReason = 'no-remote' | 'nedap' | 'not-opted-in'

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

export class CritiqueError extends Error {
  override readonly name = 'CritiqueError'

  constructor(
    readonly code: CritiqueErrorCode,
    message: string,
  ) {
    super(message)
  }
}
