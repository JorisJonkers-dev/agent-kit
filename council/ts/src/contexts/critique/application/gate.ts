import { join } from 'node:path'

import { parseGateRecord, type GateRecord } from '../domain/feature-review.js'
import {
  allowed,
  blocked,
  DEFAULT_MERGE_COMMITS,
  failedOpen,
  parseCritiqueConfig,
  parseTriageIndexRecord,
  riskReason,
  type GateName,
  type GateResult,
} from '../domain/gate.js'
import { isJsonObject, JsonFields } from '../domain/json.js'
import { gatedInvocations, type GatedInvocation, type PushTarget } from '../domain/shell-command.js'
import { parseTicketReviewRecord, type TicketReviewRecord } from '../domain/ticket-review.js'
import { codeChange, matchesGlob, riskSignals, type RiskRules } from '../domain/triage.js'
import { gateIndexPath } from './feature-gate.js'
import { critiqueConfigPath, gateLogPath } from './locations.js'
import { MergeCheck, type MergeEvidence, type MergeVerdict } from './merge-gate.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RepositoryGate, type ResolvedRepository } from './repository.js'
import { RoutingSource } from './routing.js'
import { StagedDiffReader } from './staged-change.js'
import { ticketReviewIndexPath } from './ticket-review.js'
import { triageIndexPath } from './triage-index.js'

/**
 * The gate's own deadline. A hook must finish well under a second; past
 * this the gate stops waiting and allows, whatever it was still reading.
 */
export const GATE_DEADLINE_MS = 700

/** A state file the gate relies on could not be read; the gate fails open rather than guess. */
class UnreadableState extends Error {}

/**
 * The commit and PR gates the critique hooks stand on (ADR-0002). Given a
 * `PreToolUse` payload for `Bash`, it finds the commits, pushes and
 * `gh pr create` calls the command runs and decides each from git and the
 * critique's local files alone, with no model and no network:
 *
 * - a commit is blocked while its staged change is risky by the
 *   deterministic triage rules and the last ticket review of exactly that
 *   staged change is missing or failed;
 * - a merge commit, whether `git merge` makes it or a `git commit` with
 *   `MERGE_HEAD` set, is let through when every commit it brings in is
 *   vouched for and it adds nothing of its own beyond manifest pins and
 *   rebuilt generated files (`MergeCheck`); otherwise it is judged like a
 *   commit, on its own changes when only they are not vouched for. Its
 *   check writes the auto-merge's objects into git's object store, and
 *   moves no ref, index entry or file;
 * - a `gh pr create`, or a push that updates a branch outside the feature
 *   branch patterns, is blocked unless the latest feature review of the
 *   commit it sends says `GATE: PASS`.
 *
 * It does nothing in a repository that is not opted in for code, has a
 * `nedap` remote or none, and under the kill switch. Anything that keeps it
 * from deciding (its own error, a state file it cannot read, its deadline)
 * allows the command and is logged.
 */
export class GateCommand implements CritiqueCommand<GateResult> {
  private readonly repositories: RepositoryGate
  private readonly routing: RoutingSource
  private readonly staged: StagedDiffReader
  private readonly merges: MergeCheck

  constructor(private readonly deps: CritiqueDeps) {
    this.merges = new MergeCheck(deps)
    this.repositories = new RepositoryGate(deps)
    this.routing = new RoutingSource(deps)
    this.staged = new StagedDiffReader(deps)
  }

  async run(raw: unknown): Promise<GateResult> {
    if (this.deps.env.get('CRITIQUE_OFF') === '1') return allowed(null, 'off')
    const deadline = this.deps.clock
      .sleep(GATE_DEADLINE_MS)
      .then(() => failedOpen(null, 'deadline', `the gate did not decide within ${String(GATE_DEADLINE_MS)} ms`))
    const result = await Promise.race([this.decide(raw).catch((error: unknown) => failure(null, error)), deadline])
    if (result.status === 'allowed' && result.reason === 'failed-open') await this.log(raw, result)
    return result
  }

  private async decide(raw: unknown): Promise<GateResult> {
    const configPath = critiqueConfigPath(this.deps.env)
    if (await this.deps.files.exists(configPath)) {
      const config = readState(configPath, await this.deps.files.readText(configPath), parseCritiqueConfig)
      if (config.disabled) return allowed(null, 'disabled')
    }
    const input = readPayload(raw)
    if (input.tool !== undefined && input.tool !== 'Bash') return allowed(null, 'not-gated')
    const invocations = gatedInvocations(input.command, input.cwd, this.deps.env.get('HOME')).filter(
      (invocation) => input.gate === undefined || gateOf(invocation) === input.gate,
    )
    if (invocations.length === 0) return allowed(null, 'not-gated')
    const results: GateResult[] = []
    for (const invocation of invocations) {
      results.push(await this.check(invocation).catch((error: unknown) => failure(gateOf(invocation), error)))
    }
    return (
      results.find((result) => result.status === 'blocked') ??
      results.find((result) => result.reason === 'failed-open') ??
      results[0] ??
      allowed(null, 'not-gated')
    )
  }

  private async check(invocation: GatedInvocation): Promise<GateResult> {
    const gate = gateOf(invocation)
    if (invocation.cwd === null) throw new Error('the command line does not say which directory the command runs in')
    const repository = await this.repositories.resolve(invocation.cwd)
    if (repository.status === 'refused') return allowed(gate, repository.reason === 'nedap' ? 'nedap' : 'no-remote')
    const allowlist = await this.repositories.allowlist().catch((error: unknown) => {
      throw new UnreadableState(error instanceof Error ? error.message : String(error))
    })
    const entry = allowlist.repositories[repository.key]
    if (entry === undefined) return allowed(gate, 'not-opted-in')
    if (!entry.code) return allowed(gate, 'code-not-allowed')
    if (invocation.kind === 'commit') return this.commitGate(repository, invocation.widened)
    if (invocation.kind === 'merge') return this.mergeGate(repository, invocation)
    return this.prGate(repository, invocation)
  }

  private async commitGate(repository: ResolvedRepository, widened: 'none' | 'tracked' | 'untracked'): Promise<GateResult> {
    const triage = (await this.routing.load()).triage
    if (triage === undefined) throw new Error('routing has no triage, so the commit gate has no risk rules')
    const root = repository.root
    const git = this.deps.git
    const staged = await this.staged.read(root)
    const from = widened === 'none' ? 'index' : 'working-tree'
    const [diff, paths, untracked] = await Promise.all([
      git.diffAgainstHead(root, from),
      git.pathsAgainstHead(root, from),
      widened === 'untracked' ? git.untrackedPaths(root) : Promise.resolve([]),
    ])
    const triaged = await this.lastLine(join(repository.commonDir, 'critique'), triageIndexPath, parseTriageIndexRecord, (record) =>
      record.repository === repository.key && record.staged_on === staged.head_commit,
    )
    const merge = widened === 'none' ? await this.mergeInProgress(repository, staged.head_commit) : undefined
    if (merge?.kind === 'vouched') return allowed('commit', 'merge-vouched')
    const own = merge?.kind === 'own-changes' ? merge : undefined
    const change = own === undefined ? codeChange(diff, [...paths, ...untracked]) : codeChange(own.diff, own.paths)
    const [first, ...more] = riskSignals(change, triaged?.labels ?? [], triage)
    if (first === undefined) return allowed('commit', 'low-risk')
    const why = riskReason([first, ...more])
    if (widened !== 'none' && (untracked.length > 0 || diff !== (await git.diffAgainstHead(root, 'index')))) {
      return blocked(
        'commit',
        'unstaged',
        `Commit blocked: the change is risky (${why}) and this command commits changes that are not staged yet, so no /review-ticket run can vouch for them. Stage them in a command of their own, run /review-ticket, then commit.`,
      )
    }
    const review = await this.lastLine(join(repository.commonDir, 'critique'), ticketReviewIndexPath, parseTicketReviewRecord, (record) =>
      record.repository === repository.key && record.staged_diff === staged.sha256,
    )
    if (review === undefined) {
      return blocked(
        'commit',
        'unreviewed',
        own === undefined
          ? `Commit blocked: the staged change is risky (${why}) and no passing /review-ticket run matches it. Run /review-ticket, then commit.`
          : `Commit blocked: this merge's own changes beyond the reviewed commits it brings in are risky (${why}) and no passing /review-ticket run matches the staged merge. Run /review-ticket, then commit.`,
      )
    }
    if (review.review === 'fail') return blocked('commit', 'review-failed', reviewFailedLine(review))
    return allowed('commit', 'reviewed')
  }

  /**
   * A `git merge` or `git pull` about to run: it records a merge commit
   * unless it fast-forwards, has nothing to merge, or stops on a conflict
   * (the `git commit` that concludes that merge is gated then). Each merge
   * is let through when every commit it brings in is vouched for, since the
   * clean merge adds nothing of its own, and is otherwise judged on what it
   * changes against HEAD.
   */
  private async mergeGate(repository: ResolvedRepository, invocation: Extract<GatedInvocation, { kind: 'merge' }>): Promise<GateResult> {
    if (invocation.head_moves) {
      return blocked(
        'commit',
        'head-unknown',
        'Merge blocked: this command moves HEAD before it merges, so the gate cannot tell what the merge brings in. Run the merge in a command of its own.',
      )
    }
    const root = repository.root
    const git = this.deps.git
    const evidence = await this.mergeEvidence(repository)
    const head = await git.resolveCommit(root, 'HEAD')
    const reasons: Parameters<typeof allowed>[1][] = []
    for (const revision of invocation.revisions) {
      const theirs = await git.resolveCommit(root, revision)
      const base = await git.mergeBase(root, head, theirs)
      if (base === theirs || (base === head && invocation.fast_forward)) continue
      const verdict = await this.merges.judge(evidence, head, theirs, null)
      if (verdict.kind === 'vouched' || verdict.kind === 'own-changes') {
        reasons.push('merge-vouched')
        continue
      }
      const auto = await git.mergeTree(root, head, theirs)
      if (auto.conflicts) {
        reasons.push('merge-stops')
        continue
      }
      const [diff, paths] = await Promise.all([git.diffCommits(root, head, auto.tree), git.changedPaths(root, head, auto.tree)])
      const labels = evidence.triages.filter((record) => record.staged_on === head).at(-1)?.labels ?? []
      const [first, ...more] = riskSignals(codeChange(diff, paths), labels, evidence.rules)
      if (first === undefined) {
        reasons.push('low-risk')
        continue
      }
      return blocked(
        'commit',
        'unreviewed',
        `Merge blocked: merging ${revision} is risky (${riskReason([first, ...more])}) and ${unvouchedReason(verdict, evidence.limit)}. Stage the merge with git merge --no-commit ${revision}, run /review-ticket, then commit.`,
      )
    }
    return allowed('commit', reasons.find((reason) => reason === 'low-risk') ?? reasons[0] ?? 'not-gated')
  }

  /** The check of the merge a `git commit` concludes while `MERGE_HEAD` is set, of the staged merge; none when no merge is in progress. */
  private async mergeInProgress(repository: ResolvedRepository, head: string): Promise<MergeVerdict | undefined> {
    const git = this.deps.git
    const theirs = await git.resolveCommit(repository.root, 'MERGE_HEAD').catch(() => undefined)
    if (theirs === undefined) return undefined
    return this.merges.judge(await this.mergeEvidence(repository), head, theirs, await git.writeIndexTree(repository.root))
  }

  /** What a merge check reads: this repository's ticket reviews and triages, and the rules that judge them. */
  private async mergeEvidence(repository: ResolvedRepository): Promise<MergeEvidence> {
    const routing = await this.routing.load()
    const critiqueDir = join(repository.commonDir, 'critique')
    const ours = (record: { readonly repository: string }) => record.repository === repository.key
    return {
      packet: routing.packet,
      reviews: await this.lines(critiqueDir, ticketReviewIndexPath, parseTicketReviewRecord, ours),
      root: repository.root,
      limit: routing.gate?.merge_commits ?? DEFAULT_MERGE_COMMITS,
      rules: riskRules(routing.triage),
      triages: await this.lines(critiqueDir, triageIndexPath, parseTriageIndexRecord, ours),
    }
  }

  private async prGate(repository: ResolvedRepository, invocation: Extract<GatedInvocation, { kind: 'push' | 'pr-create' }>): Promise<GateResult> {
    const root = repository.root
    const sent: { readonly what: string; readonly retry: string; readonly source: string | null }[] = []
    if (invocation.kind === 'pr-create') {
      sent.push({ retry: 'open the PR', source: invocation.head, what: 'PR' })
    } else {
      const patterns = (await this.routing.load()).gate?.feature_branches ?? []
      const targets: readonly PushTarget[] = invocation.targets === 'all' ? [{ branch: null, source: null }] : invocation.targets
      for (const target of targets) {
        const branch = invocation.targets === 'all' ? null : (target.branch ?? ((await this.deps.git.currentBranch(root)) || null))
        if (branch !== null && patterns.some((pattern) => matchesGlob(branch, pattern))) continue
        sent.push({ retry: 'push', source: target.source, what: `Push to ${branch ?? 'every branch'}` })
      }
    }
    if (sent.length === 0) return allowed('pr', 'feature-branch')
    if (invocation.head_moves) {
      const verb = invocation.kind === 'pr-create' ? 'opens the PR' : 'pushes'
      return blocked(
        'pr',
        'head-unknown',
        `${sent[0]?.what ?? 'Push'} blocked: this command moves HEAD before it ${verb}, so no /review-feature gate can hold for what it sends. Run that part in a command of its own after /review-feature.`,
      )
    }
    for (const target of sent) {
      const head = await this.deps.git.resolveCommit(root, target.source ?? 'HEAD')
      const record = await this.lastLine(join(repository.commonDir, 'critique'), gateIndexPath, parseGateRecord, (line) =>
        line.repository === repository.key && line.head_commit === head,
      )
      if (record === undefined) {
        return blocked(
          'pr',
          'no-feature-review',
          `${target.what} blocked: no /review-feature run has recorded a gate for ${short(head)}. Run /review-feature, then ${target.retry}.`,
        )
      }
      if (record.gate === 'fail') return blocked('pr', 'gate-fail', gateFailLine(target.what, target.retry, record, head))
    }
    return allowed('pr', 'gate-pass')
  }

  /** The last line of a JSON-lines index that `matches`; none when the file is absent. Any line that does not parse is unreadable state. */
  private async lastLine<T>(
    critiqueDir: string,
    pathOf: (critiqueDir: string) => string,
    parse: (text: string) => T,
    matches: (record: T) => boolean,
  ): Promise<T | undefined> {
    return (await this.lines(critiqueDir, pathOf, parse, matches)).at(-1)
  }

  /** Every line of a JSON-lines index that `matches`, oldest first; none when the file is absent. Any line that does not parse is unreadable state. */
  private async lines<T>(
    critiqueDir: string,
    pathOf: (critiqueDir: string) => string,
    parse: (text: string) => T,
    matches: (record: T) => boolean,
  ): Promise<readonly T[]> {
    const path = pathOf(critiqueDir)
    if (!(await this.deps.files.exists(path))) return []
    const lines = (await this.deps.files.readText(path)).split('\n').filter((line) => line.trim().length > 0)
    const records = lines.map((line, n) => readState(`${path} line ${String(n + 1)}`, line, parse))
    return records.filter(matches)
  }

  /** Appends one line to the gate log; a log that cannot be written is left alone, so logging never blocks. */
  private async log(raw: unknown, result: Extract<GateResult, { status: 'allowed' }>): Promise<void> {
    const cwd = isJsonObject(raw) && typeof raw.cwd === 'string' ? raw.cwd : null
    const line = { cwd, detail: result.detail, failure: result.failure, gate: result.gate, recorded_at: this.deps.clock.now().toISOString() }
    try {
      await this.deps.files.appendText(gateLogPath(this.deps.env), `${JSON.stringify(line)}\n`)
    } catch {
      // Nowhere to log; the command is allowed all the same.
    }
  }
}

interface GatePayload {
  readonly cwd: string
  readonly command: string
  readonly tool: string | undefined
  readonly gate: GateName | undefined
}

/** The hook payload: `cwd`, `tool_input.command`, the tool's name, and optionally which gate to run. */
function readPayload(raw: unknown): GatePayload {
  const fields = JsonFields.of(raw, '')
  return {
    command: fields.object('tool_input').optionalText('command') ?? '',
    cwd: fields.string('cwd'),
    gate: fields.has('gate') ? fields.oneOf('gate', ['commit', 'pr'] as const) : undefined,
    tool: fields.optionalString('tool_name'),
  }
}

/** Why a merge's commits do not vouch for it, as the end of its block line. */
function unvouchedReason(verdict: Extract<MergeVerdict, { kind: 'unvouched' | 'too-many' }>, limit: number): string {
  return verdict.kind === 'too-many'
    ? `it brings in ${String(verdict.count)} commits, more than the ${String(limit)} the gate checks one by one`
    : `${short(verdict.commit)}, a commit it brings in, has no passing /review-ticket run`
}

function gateOf(invocation: GatedInvocation): GateName {
  return invocation.kind === 'commit' || invocation.kind === 'merge' ? 'commit' : 'pr'
}

/** The commit gate's risk rules; routing without triage has none, and the gate cannot judge a commit. */
function riskRules(triage: RiskRules | undefined): RiskRules {
  if (triage === undefined) throw new Error('routing has no triage, so the commit gate has no risk rules')
  return triage
}

function readState<T>(where: string, text: string, parse: (text: string) => T): T {
  try {
    return parse(text)
  } catch (error) {
    throw new UnreadableState(`${where}: ${error instanceof Error ? error.message : String(error)}`)
  }
}

function failure(gate: GateName | null, error: unknown): GateResult {
  const detail = error instanceof Error ? error.message : String(error)
  return failedOpen(gate, error instanceof UnreadableState ? 'unreadable-state' : 'error', detail)
}

function short(commit: string): string {
  return commit.slice(0, 12)
}

function reviewFailedLine(review: TicketReviewRecord): string {
  const findings = review.unfixed_critical.join(', ')
  return `Commit blocked: /review-ticket run ${review.run_id} failed for this staged change, with critical ${findings} accepted and not fixed. Fix and stage ${review.unfixed_critical.length === 1 ? 'it' : 'them'}, then run /review-ticket again.`
}

function gateFailLine(what: string, retry: string, record: GateRecord, head: string): string {
  return `${what} blocked: /review-feature says ${record.line} for ${short(head)}. Fix and close the critical follow-ups, run /review-feature on the new head, then ${retry}.`
}
