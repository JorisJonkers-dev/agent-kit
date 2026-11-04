import { randomBytes } from 'node:crypto'

import {
  batchStatus,
  dependencyProblems,
  estimatedPrompt,
  PlanRefusedError,
  promptWithOutputs,
  readBatchInput,
  type BatchDelegation,
  type BatchInput,
  type BatchStatus,
  type DelegationStatus,
  type DependencyOutput,
  type PlanProblem,
} from '../domain/batch.js'
import { DEFAULT_MAX_ATTEMPTS, type AttemptSummary } from '../domain/escalation.js'
import { ModelGatewayError, type GatewayConfig } from '../domain/config.js'
import { isRunRecord, isSettled, type LedgerEntry, type LedgerRecord, type ReservationRecord } from '../domain/ledger.js'
import { round } from '../domain/caps.js'
import { runId } from '../domain/run.js'
import { CapGuard, type PlannedReservation, type Reservation } from './caps.js'
import { ClientSlots } from './slots.js'
import { loadConfig } from './config.js'
import { overridePath } from './locations.js'
import { messageOf } from './json-file.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'
import type { BatchTag, Checked, DelegationOptions, RunCommand, RunOutcome } from './run.js'

/** One delegation of a batch, as the result lists it. */
export interface DelegationReport {
  readonly id: string
  readonly status: DelegationStatus
  readonly client: string
  readonly model: string
  /** What every attempt of the delegation cost; 0 for one that never ran. */
  readonly cost_usd: number
  /** The file the delegation's full output is in; null for one that never produced any. */
  readonly output_path: string | null
  /** The delegation's own summary, cut at `summary_max_chars`; for a failure that produced none, what went wrong. */
  readonly summary: string
  /** Why a failed delegation failed: its last run's reason, `unavailable`, or the code of the refusal that stopped it. */
  readonly reason?: string
  /** The attempts that led to the result, when there was more than one. */
  readonly attempts?: readonly AttemptSummary[]
  /** The delegation that failed for good, which made the gateway skip this one. */
  readonly skipped_because?: string
  /** The worktree and branch of a writing delegation, where its staged change is kept until `worktrees clean`. */
  readonly worktree?: string
  readonly branch?: string
}

/** What a batch plan came to. */
export interface BatchResult {
  readonly status: BatchStatus
  readonly batch_id: string
  readonly workflow: string
  readonly delegations: readonly DelegationReport[]
  /** What the whole plan cost: every attempt of every delegation. */
  readonly cost_usd: number
  /** The up-front reservations given back because their delegation never ran. */
  readonly reservations_released_usd: number
}

/** One delegation after the plan was checked: what it is, and what it waits for. */
interface Planned {
  readonly delegation: BatchDelegation
  readonly checked: Checked
}

/**
 * Hands a whole plan of delegations to the gateway and reports each one.
 *
 * The plan is checked as a whole before anything runs: its dependencies, then
 * every delegation's client model, data policy and grant, and last the sum of
 * the metered estimates, which are reserved together in one locked ledger
 * update. A plan with any problem is refused with all of them, launches
 * nothing, creates no worktree and starts no workflow. Each delegation then
 * goes through `RunCommand`, so retry, climb, fall-through, policy, caps and
 * grants all apply; independent ones run in parallel, at most
 * `max_parallel_per_client` at once per client, and one that fails for good
 * skips only the delegations that depend on it.
 */
export class BatchCommand implements ModelGatewayCommand<unknown, BatchResult> {
  private readonly caps: CapGuard

  constructor(
    private readonly deps: ModelGatewayDependencies,
    private readonly runs: RunCommand,
  ) {
    this.caps = new CapGuard(deps)
  }

  async run(raw: unknown): Promise<BatchResult> {
    this.runs.refuseNested()
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const input = readBatchInput(raw, config.settings.max_batch_delegations)
    const planned = await this.check(input, config)
    const held = await this.reserve(input, planned, config)
    const batchId = this.batchId()
    const unrun = [...held.values()].map((reservation) => reservation.record)
    let reports: ReadonlyMap<string, DelegationReport>
    try {
      reports = await this.execute(input, config, held, batchId)
    } finally {
      await this.caps.releaseUnrun(unrun, this.deps.clock.now())
    }
    return this.result(batchId, input, reports, unrun)
  }

  /**
   * Every delegation's problems before anything is launched: the plan's
   * dependencies, and for each delegation what a run of it would be refused
   * for. A plan with any is refused with all of them.
   */
  private async check(input: BatchInput, config: GatewayConfig): Promise<readonly Planned[]> {
    const problems: PlanProblem[] = [...dependencyProblems(input.delegations)]
    const planned: Planned[] = []
    for (const delegation of input.delegations) {
      const prompt = estimatedPrompt(delegation.input.prompt, delegation.depends_on, config.settings.summary_max_chars)
      try {
        planned.push({ checked: await this.runs.check({ ...delegation.input, prompt }), delegation })
      } catch (error) {
        problems.push({ code: error instanceof ModelGatewayError ? error.code : 'unexpected', id: delegation.id, message: messageOf(error) })
      }
    }
    if (problems.length > 0) throw new PlanRefusedError(problems)
    return planned
  }

  /**
   * Reserves the estimates of every metered delegation in one locked ledger
   * update. A plan whose sum does not fit is refused, and holds nothing.
   */
  private async reserve(input: BatchInput, planned: readonly Planned[], config: GatewayConfig): Promise<ReadonlyMap<string, Reservation>> {
    const now = this.deps.clock.now()
    const window = planWindow(input)
    const plan: PlannedReservation[] = []
    for (const { checked, delegation } of planned) {
      if (checked.estimate === undefined) continue
      const { runDir, runId: id } = await this.runs.allot(checked.root, now)
      const prompt = estimatedPrompt(delegation.input.prompt, delegation.depends_on, config.settings.summary_max_chars)
      plan.push({
        cap: checked.cap,
        costClass: checked.costClass,
        estimate: checked.estimate,
        id: delegation.id,
        input: { ...delegation.input, prompt },
        runDir,
        runId: id,
        window_s: window,
      })
    }
    const reserved = await this.caps.reservePlan(plan, config, now)
    if (reserved.refused.length > 0) {
      throw new PlanRefusedError(reserved.refused.map((refusal) => ({ code: 'cap-exceeded', id: refusal.id, message: refusal.message })))
    }
    return reserved.held
  }

  /**
   * Runs the plan: a delegation starts when everything it waits for is done
   * and its client has a free slot, and one whose dependency failed for good
   * (or was itself skipped) is skipped, its reservation given back at once.
   * The slots a delegation is scheduled by are those of the client it was
   * planned for; each attempt then takes a slot of the client it really runs
   * on. A delegation that can never start is reported skipped, never dropped.
   */
  private async execute(
    input: BatchInput,
    config: GatewayConfig,
    held: ReadonlyMap<string, Reservation>,
    batchId: string,
  ): Promise<ReadonlyMap<string, DelegationReport>> {
    const limit = config.settings.max_parallel_per_client
    const reports = new Map<string, DelegationReport>()
    const outputs = new Map<string, DependencyOutput>()
    const pending = [...input.delegations]
    const running = new Set<Promise<void>>()
    const planned = new Map<string, number>()
    const attempts = new ClientSlots(limit)
    try {
      for (;;) {
        await this.skipBlocked(pending, reports, held)
        for (const delegation of [...pending]) {
          const client = delegation.input.client
          const waits = delegation.depends_on.map((id) => outputs.get(id))
          if ((planned.get(client) ?? 0) >= limit || waits.includes(undefined)) continue
          pending.splice(pending.indexOf(delegation), 1)
          planned.set(client, (planned.get(client) ?? 0) + 1)
          const options = { slot: (actual: string) => attempts.acquire(actual), tag: { batch_id: batchId, delegation: delegation.id } }
          const start: Promise<void> = this.delegate(delegation, waits.filter((output) => output !== undefined), { ...options, ...heldBy(held, delegation.id) }).then(
            (report) => {
              reports.set(delegation.id, report)
              if (report.status === 'done') {
                outputs.set(delegation.id, { id: delegation.id, output_path: report.output_path, summary: report.summary })
              }
              planned.set(client, (planned.get(client) ?? 1) - 1)
              running.delete(start)
            },
          )
          running.add(start)
        }
        if (running.size === 0) break
        await Promise.race(running)
      }
    } finally {
      await Promise.allSettled(running)
    }
    for (const delegation of [...pending]) await this.skip(delegation, 'could-not-start', pending, reports, held)
    return reports
  }

  /** Skips every pending delegation that waits for one that failed or was skipped, to the end of the chain. */
  private async skipBlocked(
    pending: BatchDelegation[],
    reports: Map<string, DelegationReport>,
    held: ReadonlyMap<string, Reservation>,
  ): Promise<void> {
    for (;;) {
      const stopped = (id: string): DelegationReport | undefined => {
        const report = reports.get(id)
        return report?.status === 'done' ? undefined : report
      }
      const blocked = pending.find((delegation) => delegation.depends_on.some((id) => stopped(id) !== undefined))
      if (blocked === undefined) return
      const cause = blocked.depends_on.map(stopped).find((report) => report !== undefined)
      await this.skip(blocked, cause?.skipped_because ?? cause?.id ?? '', pending, reports, held)
    }
  }

  /** Takes a delegation off the pending list as skipped, because of the delegation named, and gives its reservation back. */
  private async skip(
    delegation: BatchDelegation,
    because: string,
    pending: BatchDelegation[],
    reports: Map<string, DelegationReport>,
    held: ReadonlyMap<string, Reservation>,
  ): Promise<void> {
    pending.splice(pending.indexOf(delegation), 1)
    reports.set(delegation.id, {
      client: delegation.input.client,
      cost_usd: 0,
      id: delegation.id,
      model: delegation.input.model,
      output_path: null,
      skipped_because: because,
      status: 'skipped',
      summary: '',
    })
    const reservation = held.get(delegation.id)
    if (reservation !== undefined) await this.caps.releaseUnrun([reservation.record], this.deps.clock.now())
  }

  /**
   * One delegation through the single-delegation path, whatever it comes to: a
   * refusal or a crash is a failed delegation, never a lost batch. One that
   * threw reports what its attempts cost, from the ledger lines they left,
   * and gives its reservation back (or settles it at what its run cost).
   */
  private async delegate(delegation: BatchDelegation, outputs: readonly DependencyOutput[], options: DelegationOptions): Promise<DelegationReport> {
    const input = { ...delegation.input, prompt: promptWithOutputs(delegation.input.prompt, outputs) }
    try {
      return reportOf(delegation, await this.runs.delegation(input, options))
    } catch (error) {
      if (options.reservation !== undefined) await this.caps.releaseUnrun([options.reservation.record], this.deps.clock.now())
      const lines = taggedLines(await this.deps.ledger.read(), options.tag)
      return {
        ...(lines.length === 0 ? {} : { attempts: lines.map(attemptOf) }),
        client: delegation.input.client,
        cost_usd: costOf(lines),
        id: delegation.id,
        model: delegation.input.model,
        output_path: null,
        reason: error instanceof ModelGatewayError ? error.code : 'unexpected',
        status: 'failed',
        summary: messageOf(error),
      }
    }
  }

  /** The batch's one result: each delegation in plan order with what its attempts cost, and what the plan gave back. */
  private async result(
    batchId: string,
    input: BatchInput,
    reports: ReadonlyMap<string, DelegationReport>,
    reservations: readonly ReservationRecord[],
  ): Promise<BatchResult> {
    const ledger = await this.deps.ledger.read()
    const delegations = input.delegations.flatMap((delegation) => {
      const report = reports.get(delegation.id)
      return report === undefined ? [] : [{ ...report, cost_usd: costOf(taggedLines(ledger, { batch_id: batchId, delegation: delegation.id })) }]
    })
    const released = reservations.filter((record) => ledger.some((entry) => isSettled(entry) && entry.run_id === record.run_id && entry.reason === 'not-run'))
    return {
      batch_id: batchId,
      cost_usd: round(delegations.reduce((total, report) => total + report.cost_usd, 0), 6),
      delegations,
      reservations_released_usd: round(released.reduce((total, record) => total + record.estimate_usd, 0), 6),
      status: batchStatus(delegations.map((report) => report.status)),
      workflow: input.workflow,
    }
  }

  private batchId(): string {
    return `batch-${runId(this.deps.clock.now(), randomBytes(3).toString('hex'))}`
  }
}

/** What a delegation's result says: its last run's, and the attempts that led to it. */
function reportOf(delegation: BatchDelegation, outcome: RunOutcome): DelegationReport {
  if (outcome.status === 'unavailable') {
    return {
      attempts: outcome.attempts,
      client: outcome.client,
      cost_usd: 0,
      id: delegation.id,
      model: outcome.model,
      output_path: null,
      reason: 'unavailable',
      status: 'failed',
      summary: outcome.message,
    }
  }
  return {
    ...(outcome.attempts.length > 1 ? { attempts: outcome.attempts } : {}),
    client: outcome.client,
    cost_usd: outcome.cost_usd,
    id: delegation.id,
    model: outcome.model,
    output_path: outcome.output_path,
    ...(outcome.reason === undefined ? {} : { reason: outcome.reason }),
    ...(outcome.worktree === undefined ? {} : { branch: outcome.branch, worktree: outcome.worktree }),
    status: outcome.status === 'ok' ? 'done' : 'failed',
    summary: outcome.summary,
  }
}

/** The ledger lines the attempts of one batch delegation left: each is a run of its own, tagged with the batch and the delegation. */
function taggedLines(ledger: readonly LedgerEntry[], tag: BatchTag | undefined): readonly LedgerRecord[] {
  return ledger.filter(isRunRecord).filter((line) => tag !== undefined && line.batch_id === tag.batch_id && line.delegation === tag.delegation)
}

/** What a delegation's attempts cost: their ledger lines added up. */
function costOf(lines: readonly LedgerRecord[]): number {
  return round(lines.reduce((total, line) => total + line.cost_usd, 0), 6)
}

/** One ledger line, as an attempt the result lists. */
function attemptOf(line: LedgerRecord): AttemptSummary {
  return { client: line.client, model: line.model, run_id: line.run_id, status: line.outcome, ...(line.reason === undefined ? {} : { reason: line.reason }) }
}

/** What the batch hands the delegation that holds a reservation: nothing for one that holds none. */
function heldBy(held: ReadonlyMap<string, Reservation>, id: string): { readonly reservation?: Reservation } {
  const reservation = held.get(id)
  return reservation === undefined ? {} : { reservation }
}

/**
 * How long the plan's reservations hold if their runs die: as long as every
 * delegation could take in turn, with all its attempts. The window only
 * matters to a run that died; a living one settles its own.
 */
function planWindow(input: BatchInput): number {
  return input.delegations.reduce(
    (total, delegation) =>
      total + delegation.input.time_limit_s * (delegation.input.escalate === false ? 1 : (delegation.input.max_attempts ?? DEFAULT_MAX_ATTEMPTS)),
    0,
  )
}
