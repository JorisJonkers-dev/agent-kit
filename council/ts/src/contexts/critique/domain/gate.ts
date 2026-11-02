import { JsonFields } from './json.js'
import type { RiskSignal } from './triage.js'

/** The two gates: before a commit, and before a PR or a push to a branch that is not a feature branch. */
export type GateName = 'commit' | 'pr'

/** Why a gate let the command through. */
export const ALLOW_REASONS = [
  /** `CRITIQUE_OFF=1` is set. */
  'off',
  /** This machine's critique configuration says `"disabled": true`. */
  'disabled',
  /** The command runs no commit, push or `gh pr create`. */
  'not-gated',
  'no-remote',
  'nedap',
  'not-opted-in',
  /** The repository's allowlist entry allows documents only, so no code stage could ever pass the gate. */
  'code-not-allowed',
  /** No deterministic risk rule fired on the staged change. */
  'low-risk',
  /** The last ticket review of exactly this staged change passed. */
  'reviewed',
  /** Every commit the merge brings in is vouched for, and it adds nothing of its own beyond manifest pins and rebuilt generated files. */
  'merge-vouched',
  /** The merge conflicts, so git stops before committing it; the `git commit` that concludes it is gated then. */
  'merge-stops',
  /** Every branch the push updates is a feature branch. */
  'feature-branch',
  /** The latest feature review of the commit being sent says `GATE: PASS`. */
  'gate-pass',
  /** The gate could not decide: its own error, an unreadable state file, or its deadline. */
  'failed-open',
] as const
export type AllowReason = (typeof ALLOW_REASONS)[number]

/** Why a gate blocked the command. */
export const BLOCK_REASONS = [
  /** A risky staged change with no ticket review of it. */
  'unreviewed',
  /** The last ticket review of this staged change failed. */
  'review-failed',
  /** A risky commit that records more than is staged, which no ticket review vouches for. */
  'unstaged',
  /** No feature review recorded a gate for the commit being sent. */
  'no-feature-review',
  /** The latest feature review of the commit being sent says `GATE: FAIL`. */
  'gate-fail',
  /** The same command moves HEAD before it pushes, opens the PR or merges, so the commit sent, or the merge's HEAD, is not known yet. */
  'head-unknown',
] as const
export type BlockReason = (typeof BLOCK_REASONS)[number]

/** Why a gate failed open. */
export type GateFailure = 'error' | 'unreadable-state' | 'deadline'

/** What a `PreToolUse` hook prints to block the tool call, in both Claude Code's and Codex's shape (ADR-0002). */
export interface HookDeny {
  readonly hookSpecificOutput: {
    readonly hookEventName: 'PreToolUse'
    readonly permissionDecision: 'deny'
    readonly permissionDecisionReason: string
  }
}

/**
 * What the gate decided. A hook prints `hook_output` when it is there and
 * nothing otherwise, and exits 0 either way.
 */
export type GateResult =
  | {
      readonly status: 'allowed'
      /** The gate that decided; null when none applied. */
      readonly gate: GateName | null
      readonly reason: AllowReason
      readonly line: null
      /** Only when the gate failed open. */
      readonly failure?: GateFailure
      readonly detail?: string
    }
  | {
      readonly status: 'blocked'
      readonly gate: GateName
      readonly reason: BlockReason
      /** One line that explains the block and names the skill to run. */
      readonly line: string
      readonly hook_output: HookDeny
    }

export function allowed(gate: GateName | null, reason: Exclude<AllowReason, 'failed-open'>): GateResult {
  return { gate, line: null, reason, status: 'allowed' }
}

export function failedOpen(gate: GateName | null, failure: GateFailure, detail: string): GateResult {
  return { detail, failure, gate, line: null, reason: 'failed-open', status: 'allowed' }
}

export function blocked(gate: GateName, reason: BlockReason, line: string): GateResult {
  return {
    gate,
    hook_output: { hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny', permissionDecisionReason: line } },
    line,
    reason,
    status: 'blocked',
  }
}

/** Why a staged change is risky, in a few words: the deciding signal, and how many more fired. */
export function riskReason(signals: readonly [RiskSignal, ...RiskSignal[]]): string {
  const [first, ...rest] = signals
  const why =
    first.kind === 'risk-path'
      ? `${first.path} is a risk path`
      : first.kind === 'size'
        ? `${String(first.changed_lines)} changed lines, above ${String(first.threshold)}`
        : `the ticket carries ${first.label}`
  return rest.length === 0 ? why : `${why}, and ${String(rest.length)} more risk signal${rest.length === 1 ? '' : 's'}`
}

/** Routing's `gate`: the branches a push may update without a feature review, and how many commits a merge check reads one by one. */
export interface GateRouting {
  /** Globs over branch names (see `matchesGlob`); a push that updates only such branches is not gated. */
  readonly feature_branches: readonly string[]
  /**
   * The most commits a merge may bring in and still be checked commit by
   * commit; a merge that brings in more is judged whole, so the check stays
   * within the gate's deadline. 50 when routing leaves it out.
   */
  readonly merge_commits: number
}

export const DEFAULT_MERGE_COMMITS = 50

export function readGateRouting(fields: JsonFields): GateRouting {
  return {
    feature_branches: fields.strings('feature_branches'),
    merge_commits: fields.has('merge_commits') ? fields.positiveInteger('merge_commits') : DEFAULT_MERGE_COMMITS,
  }
}

/** This machine's critique configuration, `~/.config/agent-kit/critique.json`. */
export interface CritiqueConfig {
  /** The per-machine kill switch: every gate allows. */
  readonly disabled: boolean
}

export function parseCritiqueConfig(text: string): CritiqueConfig {
  const file = JsonFields.parse(text)
  return { disabled: file.has('disabled') ? file.boolean('disabled') : false }
}

/**
 * One staged change's triage, as the triage index keeps it: which ticket was
 * staged on which commit, and the labels the tracker gave it, so a commit
 * gate can read the ticket's `risk:high` without calling the tracker.
 */
export interface TriageIndexRecord {
  readonly version: 1
  readonly run_id: string
  readonly recorded_at: string
  readonly repository: string
  readonly ticket: string
  readonly labels: readonly string[]
  /** HEAD when the change was triaged: the commit it is staged on. */
  readonly staged_on: string
  /** `stagedDiffHash` of the change triaged. */
  readonly staged_diff: string
  readonly status: 'risky' | 'low-risk'
}

export function parseTriageIndexRecord(text: string): TriageIndexRecord {
  const file = JsonFields.parse(text)
  return {
    labels: file.strings('labels'),
    recorded_at: file.string('recorded_at'),
    repository: file.string('repository'),
    run_id: file.string('run_id'),
    staged_diff: file.string('staged_diff'),
    staged_on: file.string('staged_on'),
    status: file.oneOf('status', ['risky', 'low-risk'] as const),
    ticket: file.string('ticket'),
    version: file.literal('version', 1),
  }
}
