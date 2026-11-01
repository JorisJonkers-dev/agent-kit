import { join } from 'node:path'

import { budgetReached } from '../domain/budget.js'
import { compactTimestamp, sha256 } from '../domain/manifest.js'
import { CritiqueError, type Refusal } from '../domain/outcomes.js'
import type { Routing, TriageRouting } from '../domain/routing.js'
import {
  readTriageAnswer,
  riskSignals,
  triageLedgerLine,
  triageSummary,
  type RiskSignal,
  type TriageDecision,
  type TriageSignal,
  type UnavailableReason,
} from '../domain/triage.js'
import { CodeChangeReader } from './code-change.js'
import { FlowBudget } from './budget.js'
import { readInput } from './decode.js'
import { ledgerPath } from './locations.js'
import type { CritiqueCommand, CritiqueDeps, ModelAnswerReport } from './ports.js'
import { RepositoryGate, type ResolvedRepository } from './repository.js'
import { RoutingSource } from './routing.js'
import { StagedDiffReader, stagedInput } from './staged-change.js'
import { TrackerResolver } from './tracker.js'
import { appendTriageIndex } from './triage-index.js'

interface TriageInput {
  readonly repo: string
  /** The ticket the change delivers, named the way prepare names it; its labels are read for `risk:high`. */
  readonly ticket: string
  readonly base: string
  readonly head: string
  /** Whether the change is the staged one rather than `head`. */
  readonly staged: boolean
  /** The spec the ticket belongs to, named the way prepare names it; only its reference is read, for the default flow. */
  readonly spec: string | undefined
  /** The flow whose Vibe spend the triage call adds to; the spec's reference, else the ticket's, by default. */
  readonly flow: string | undefined
}

/** What the triage model was asked, and what it answered. */
export interface TriageModelCall {
  readonly client: string
  readonly model: string
  readonly cost_usd: number | null
  readonly charged_usd: number
  /** Absent when the model gave no yes or no. */
  readonly answer?: 'yes' | 'no'
}

export interface TriageResult {
  /** `low-risk` means the review is skipped, and the skip is logged. */
  readonly status: TriageDecision
  readonly run_id: string
  /** The signal that decided it. */
  readonly signal: TriageSignal
  /** Every risk rule that fired, the deciding one first; empty when the model decided. */
  readonly signals: readonly RiskSignal[]
  readonly ticket: { readonly reference: string; readonly labels: readonly string[] }
  readonly flow: string
  readonly change: {
    readonly base_commit: string
    readonly head_commit: string
    readonly changed_files: number
    readonly changed_lines: number
    readonly changed_lines_threshold: number
  }
  /** Only when the model was launched. */
  readonly model?: TriageModelCall
  /** One line for the stage's summary, naming the decision and why. */
  readonly summary: string
}

const TRIAGE_PROMPT = 'prompts/triage.md'
const DIFF_PATH = join('diff', 'change.diff')

/**
 * Risk triage for ticket review. The risk rules decide first, from git and
 * the ticket's labels alone: a touched risk path, a change above the size
 * threshold, a risk label. Only when none fires is the triage model asked
 * whether the change warrants a review, and only where the family allowlist
 * lets the repository send code to the model's family and the flow's Vibe
 * budget is not spent. A model that cannot run, or gives no yes or no,
 * leaves the change risky. Every decision is logged; a low-risk one as a run
 * with status `skipped-low-risk`.
 */
export class TriageCommand implements CritiqueCommand<TriageResult | Refusal> {
  private readonly repositories: RepositoryGate
  private readonly trackers: TrackerResolver
  private readonly changes: CodeChangeReader
  private readonly routing: RoutingSource
  private readonly budget: FlowBudget
  private readonly staged: StagedDiffReader

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
    this.trackers = new TrackerResolver(deps)
    this.changes = new CodeChangeReader(deps)
    this.routing = new RoutingSource(deps)
    this.budget = new FlowBudget(deps)
    this.staged = new StagedDiffReader(deps)
  }

  async run(raw: unknown): Promise<TriageResult | Refusal> {
    const input = readInput(raw, 'triage', (fields): TriageInput => ({
      base: fields.string('base'),
      flow: fields.optionalString('flow'),
      head: fields.optionalString('head') ?? 'HEAD',
      repo: fields.string('repo'),
      spec: fields.optionalString('spec'),
      staged: stagedInput(fields, 'ticket-review'),
      ticket: fields.string('ticket'),
    }))
    const repository = await this.repositories.resolve(input.repo)
    if (repository.status === 'refused') return repository
    const routing = await this.routing.load()
    const triage = routing.triage
    if (triage === undefined) throw new CritiqueError('invalid-routing', 'routing has no triage, so a change cannot be triaged')
    const tracker = await this.trackers.resolve(repository)
    if (tracker.status === 'refused') return tracker
    const ticket = await tracker.artifact({ comments: false, reference: input.ticket })
    if (ticket.status === 'refused') return ticket
    const spec = input.spec === undefined ? undefined : await tracker.artifact({ comments: false, reference: input.spec })
    if (spec?.status === 'refused') return spec
    const snapshot = input.staged ? await this.staged.snapshot(repository.root) : undefined
    const head = snapshot?.commit ?? input.head
    const read = await this.changes.read(repository.root, input.base, head)
    const runId = await this.freshRunId(repository)
    const runDir = join(repository.commonDir, 'critique', runId)
    const flow = input.flow ?? spec?.reference ?? ticket.reference
    const signals = riskSignals(read.change, ticket.labels, triage)
    const [first] = signals
    const asked: Decided =
      first === undefined
        ? await this.ask({ diff: read.diff, flow, repository, routing, runDir, runId, triage })
        : { decision: 'risky', signal: first }
    const signal = asked.signal
    const result: TriageResult = {
      change: {
        base_commit: read.base_commit,
        changed_files: read.change.paths.length,
        changed_lines: read.change.changed_lines,
        changed_lines_threshold: triage.changed_lines_threshold,
        head_commit: read.head_commit,
      },
      flow,
      ...(asked.model === undefined ? {} : { model: asked.model }),
      run_id: runId,
      signal,
      signals,
      status: asked.decision,
      summary: triageSummary(asked.decision, signal, triage.model.client),
      ticket: { labels: ticket.labels, reference: ticket.reference },
    }
    await this.deps.files.writeText(join(runDir, 'triage.json'), `${JSON.stringify(result, null, 2)}\n`)
    const recordedAt = this.deps.clock.now().toISOString()
    const line = triageLedgerLine({
      ...result,
      prompt_hash: asked.promptHash ?? null,
      recorded_at: recordedAt,
      repository: repository.key,
    })
    await this.deps.files.appendText(ledgerPath(this.deps.env), `${JSON.stringify(line)}\n`)
    if (snapshot !== undefined) {
      await appendTriageIndex(this.deps.files, repository.commonDir, {
        labels: ticket.labels,
        recorded_at: recordedAt,
        repository: repository.key,
        run_id: runId,
        staged_diff: snapshot.sha256,
        staged_on: snapshot.head_commit,
        status: result.status,
        ticket: ticket.reference,
        version: 1,
      })
    }
    return result
  }

  /**
   * Asks the triage model, when it may run, whether the change warrants a
   * second review. Each reason it may not run leaves the change risky.
   */
  private async ask(request: TriageAsk): Promise<Decided> {
    const { repository, routing, triage } = request
    const config = triage.model.client_config
    const unavailable = (reason: UnavailableReason, detail: string): Decided => ({
      decision: 'risky',
      signal: { detail, kind: 'model-unavailable', reason },
    })
    const entry = (await this.repositories.allowlist()).repositories[repository.key]
    if (entry === undefined) return unavailable('not-opted-in', `${repository.key} has no family allowlist entry on this machine`)
    if (!entry.code) return unavailable('code-not-allowed', `${repository.key} allows documents but not code`)
    if (!entry.families.includes(config.family)) {
      return unavailable('family-not-allowed', `${repository.key} does not allow the ${config.family} family`)
    }
    const spent = budgetReached(await this.budget.vibeSpend({ flow: request.flow, repository: repository.key }), routing.limits)
    if (spent !== undefined) return unavailable('budget', spent)
    const ready = await this.deps.clients.vibe.readiness(config)
    if (!ready.ready) return unavailable('unavailable', ready.detail)

    const promptAsset = await this.deps.files.readText(join(this.deps.assetsRoot, TRIAGE_PROMPT))
    const workDir = join(request.runDir, 'packet')
    const promptPath = join(request.runDir, 'prompt.md')
    await this.deps.files.writeText(join(workDir, DIFF_PATH), request.diff)
    await this.deps.files.writeText(
      promptPath,
      `${promptAsset.trim()}\n\n# Files\n\n- \`${DIFF_PATH}\`: the change under review, as a diff against its base\n`,
    )
    const report = await this.deps.clients.vibe.ask(
      { max_price_usd: triage.model.max_price_usd, outputDir: request.runDir, promptPath, time_limit_s: triage.model.time_limit_s, workDir },
      config,
    )
    await this.deps.files.writeText(join(request.runDir, 'stdout.json'), report.stdout)
    await this.deps.files.writeText(join(request.runDir, 'stderr.txt'), report.stderr)
    const call = { client: triage.model.client, family: config.family, model: config.model }
    await this.budget.logInvocation(
      { flow: request.flow, repository: repository.key, round: 1, run_id: request.runId, stage: 'ticket-review' },
      { ...call, charged_usd: report.chargedUsd, critic_id: 'triage', status: report.answer.status === 'answered' ? 'ok' : 'failed' },
      'vibe',
    )
    return { ...answered(report), model: modelCall(call, report), promptHash: sha256(promptAsset) }
  }

  private async freshRunId(repository: ResolvedRepository): Promise<string> {
    const base = `${compactTimestamp(this.deps.clock.now())}-triage`
    let candidate = base
    for (let n = 2; await this.deps.files.exists(join(repository.commonDir, 'critique', candidate, 'triage.json')); n += 1) {
      candidate = `${base}-${String(n)}`
    }
    return candidate
  }
}

interface TriageAsk {
  readonly repository: ResolvedRepository
  readonly routing: Routing
  readonly triage: TriageRouting
  readonly runDir: string
  readonly runId: string
  readonly flow: string
  readonly diff: string
}

/** A decision, and when the model was launched, what it was asked and answered. */
interface Decided {
  readonly decision: TriageDecision
  readonly signal: TriageSignal
  readonly model?: TriageModelCall
  /** The triage prompt's hash, when the model was launched. */
  readonly promptHash?: string
}

/** A yes is risky and a no low-risk; a failed run or any other answer leaves the change risky. */
function answered(report: ModelAnswerReport): Decided {
  const { answer } = report
  if (answer.status === 'failed') {
    return { decision: 'risky', signal: { detail: `${answer.failure}: ${answer.detail}`, kind: 'model-unavailable', reason: 'failed' } }
  }
  const word = readTriageAnswer(answer.text)
  if (word === undefined) {
    return {
      decision: 'risky',
      signal: { detail: 'the triage model answered neither yes nor no', kind: 'model-unavailable', reason: 'unclear-answer' },
    }
  }
  return { decision: word === 'yes' ? 'risky' : 'low-risk', signal: { answer: word, kind: 'model' } }
}

function modelCall(call: { readonly client: string; readonly model: string }, report: ModelAnswerReport): TriageModelCall {
  const word = report.answer.status === 'answered' ? readTriageAnswer(report.answer.text) : undefined
  return {
    charged_usd: report.chargedUsd,
    client: call.client,
    cost_usd: report.answer.cost_usd,
    model: call.model,
    ...(word === undefined ? {} : { answer: word }),
  }
}
