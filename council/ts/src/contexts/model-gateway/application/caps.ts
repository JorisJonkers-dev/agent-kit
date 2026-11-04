import { join } from 'node:path'

import {
  capRefusalMessage,
  checkCaps,
  estimateRun,
  liveReservations,
  reservationRecord,
  settledRecord,
  settleExpired,
  type Estimate,
} from '../domain/caps.js'
import {
  isObject,
  ModelGatewayError,
  type ClientConfig,
  type CostClass,
  type GatewayConfig,
  type MeteredPersonalCaps,
  type ModelEntry,
} from '../domain/config.js'
import { grantRefusal, type Grant } from '../domain/grants.js'
import type { LedgerEntry, ReservationRecord } from '../domain/ledger.js'
import { costClassFor } from '../domain/models.js'
import { openCodeProvider, type RunInput } from '../domain/run.js'
import { workflowEndsAt } from '../domain/workflows.js'
import type { ModelGatewayDependencies } from './ports.js'

/** What the caps read: the ledger, and the run directories' results. */
export type CapDependencies = Pick<ModelGatewayDependencies, 'files' | 'ledger' | 'workflows'>

/**
 * What a metered run holds while it runs: the estimate it reserved, and the
 * ledger line that holds it.
 */
export interface Reservation {
  readonly estimate: Estimate
  readonly record: ReservationRecord
  /** The time limit the run runs under: its own, clamped to what is left of its grant and workflow. */
  readonly time_limit_s: number
  /** For a run under a grant, the moment its grant or workflow lapses, whichever comes first; the run's check is held to it too. */
  readonly ends_at?: Date
}

/** A run under a grant needs at least this long before the grant or its workflow lapses. */
export const MIN_GRANT_TIME_LEFT_S = 60

/** One delegation of a batch plan, ready to have its estimate reserved up front. */
export interface PlannedReservation {
  /** The delegation's id in the plan, which the result is keyed by. */
  readonly id: string
  readonly cap: CapSource
  readonly costClass: CostClass
  readonly estimate: Estimate
  readonly input: RunInput
  readonly runDir: string
  readonly runId: string
  /** How long the reservation holds if its run dies, before the grace period: the plan's longest possible duration. */
  readonly window_s: number
}

/** A delegation whose estimate does not fit, and why. */
export interface PlanRefusal {
  readonly id: string
  readonly message: string
}

/** What reserving a plan came to: the reservation each delegation holds, or the ones that did not fit (and then nothing is held). */
export interface PlanReservation {
  readonly held: ReadonlyMap<string, Reservation>
  readonly refused: readonly PlanRefusal[]
}

/**
 * What a metered run's estimate is held against: the metered-personal caps, or
 * the grant that covers a metered-employer run. A subsidised run is held to
 * neither, and its plan limit watches it instead.
 */
export type CapSource = { readonly kind: 'metered-personal' } | { readonly kind: 'grant'; readonly grant: Grant }

/**
 * The caps a metered run is held to: what it is estimated to cost, the
 * reservation it holds before it starts, and the settlement that replaces it
 * with what the run really cost.
 *
 * A metered-personal run is held to the per-workflow and per-day caps; a
 * metered-employer run to the grant that covers its workflow. A reservation is
 * decided and written in one locked critical section that also settles the
 * reservations whose runs died, so two gateway processes cannot together
 * overshoot a cap.
 */
export class CapGuard {
  constructor(private readonly deps: CapDependencies) {}

  /**
   * What a metered run is estimated to cost, or a `no-estimate` refusal for a
   * client model the catalogue prices nowhere or gives no tier: a run the
   * gateway cannot estimate is never run uncapped. A subsidised run is held to
   * nothing, so it has no estimate.
   */
  estimate(input: RunInput, entry: ModelEntry | undefined, config: GatewayConfig): Estimate | undefined {
    if (heldCostClass(input, config) === undefined) return undefined
    const estimate = estimateRun(input.prompt, entry?.tier, entry?.prices, config.settings.output_allowance_tokens)
    if (estimate === undefined) {
      throw new ModelGatewayError(
        'no-estimate',
        `${input.client}:${input.model} cannot be estimated: the catalogue gives it no price or no tier, so a metered run on it cannot be held to the caps`,
      )
    }
    return estimate
  }

  /**
   * Reserves the run's estimate against the caps or the grant that holds it, or
   * refuses it with the cap it does not fit. The decision and the reservation
   * are one locked critical section over the ledger, which also settles every
   * reservation whose run left no settlement and whose expiry has passed.
   */
  async reserve(args: {
    readonly cap: CapSource
    readonly config: GatewayConfig
    readonly costClass: CostClass
    readonly estimate: Estimate | undefined
    readonly input: RunInput
    readonly now: Date
    readonly runDir: string
    readonly runId: string
  }): Promise<Reservation | undefined> {
    const { cap, config, costClass, estimate, input, now, runDir, runId } = args
    if (estimate === undefined) return undefined
    const held = cap.kind === 'grant' ? await this.grantTimeLimit(cap.grant, input, now) : undefined
    const timeLimitS = held?.time_limit_s ?? input.time_limit_s
    const costs = await this.finishedCosts()
    const record = reservationRecord({
      client: input.client,
      cost_class: costClass,
      estimate_usd: estimate.usd,
      model: input.model,
      reserved_at: now,
      run_dir: runDir,
      run_id: runId,
      time_limit_s: timeLimitS,
      workflow: input.workflow,
    })
    const refusal = await this.deps.ledger.update((entries) => {
      const settled = settleExpired(entries, now, costs)
      const all = [...entries, ...settled]
      const message =
        cap.kind === 'grant'
          ? grantRefusal(all, cap.grant, estimate.usd, input)
          : capRefusal(all, input, estimate.usd, config.settings.metered_personal_caps, now)
      return { append: message === undefined ? [...settled, record] : settled, result: message }
    })
    if (refusal !== undefined) throw new ModelGatewayError('cap-exceeded', refusal)
    return { estimate, record, time_limit_s: timeLimitS, ...(held === undefined ? {} : { ends_at: held.ends_at }) }
  }

  /**
   * The time limit a run under a grant may have: its own, clamped to the time
   * left until the grant lapses and until its workflow's 24 hours (or its end)
   * run out, so a run launched just before either keeps no spend running past
   * it. With under a minute left the run is refused (`grant-expiring`).
   */
  private async grantTimeLimit(grant: Grant, input: RunInput, now: Date): Promise<{ time_limit_s: number; ends_at: Date }> {
    const record = (await this.deps.workflows.read()).find((candidate) => candidate.name === input.workflow)
    const ends = [Date.parse(grant.expires_at), ...(record === undefined ? [] : [workflowEndsAt(record).getTime()])]
    const endsAt = new Date(Math.min(...ends))
    const leftS = Math.floor((endsAt.getTime() - now.getTime()) / 1000)
    if (leftS < MIN_GRANT_TIME_LEFT_S) {
      throw new ModelGatewayError(
        'grant-expiring',
        `the grant covering workflow ${input.workflow} has ${String(Math.max(leftS, 0))} seconds left: wait for it to lapse, then ask the owner for a new grant, or start a new workflow`,
      )
    }
    return { ends_at: endsAt, time_limit_s: Math.min(input.time_limit_s, leftS) }
  }

  /** Releases a reservation with what the run really cost, so the caps count spend rather than the estimate. */
  async settle(reservation: Reservation | undefined, costUsd: number, now: Date): Promise<void> {
    if (reservation === undefined) return
    await this.deps.ledger.append(settledRecord(reservation.record, costUsd, now))
  }

  /**
   * Releases a reservation the run never used, at no cost: the workflow could
   * not be started after the estimate was held, so the caps must not keep
   * holding it.
   */
  async release(reservation: Reservation | undefined, now: Date): Promise<void> {
    if (reservation === undefined) return
    await this.deps.ledger.append({ ...settledRecord(reservation.record, 0, now), reason: 'not-started' as const })
  }

  /**
   * Reserves a whole batch plan's estimates at once: each is checked, in plan
   * order, against the caps or its grant with the ones before it already held,
   * and every record is written in the same locked ledger update that decided
   * them, so no other process can take room in between. A plan any of whose
   * estimates does not fit holds nothing: the refusals come back, and no
   * record is written.
   */
  async reservePlan(plan: readonly PlannedReservation[], config: GatewayConfig, now: Date): Promise<PlanReservation> {
    const costs = await this.finishedCosts()
    return this.deps.ledger.update((entries) => {
      const settled = settleExpired(entries, now, costs)
      let all: readonly LedgerEntry[] = [...entries, ...settled]
      const held = new Map<string, Reservation>()
      const records: ReservationRecord[] = []
      const refused: PlanRefusal[] = []
      for (const item of plan) {
        const message =
          item.cap.kind === 'grant'
            ? grantRefusal(all, item.cap.grant, item.estimate.usd, item.input)
            : capRefusal(all, item.input, item.estimate.usd, config.settings.metered_personal_caps, now)
        if (message !== undefined) {
          refused.push({ id: item.id, message })
          continue
        }
        const record = reservationRecord({
          client: item.input.client,
          cost_class: item.costClass,
          estimate_usd: item.estimate.usd,
          model: item.input.model,
          reserved_at: now,
          run_dir: item.runDir,
          run_id: item.runId,
          time_limit_s: item.window_s,
          workflow: item.input.workflow,
        })
        held.set(item.id, { estimate: item.estimate, record, time_limit_s: item.input.time_limit_s })
        records.push(record)
        all = [...all, record]
      }
      return { append: refused.length === 0 ? [...settled, ...records] : settled, result: { held, refused } }
    })
  }

  /**
   * Gives back the reservations of a batch's delegations, whatever of them is
   * still held. A run that left a result before it was lost counts what that
   * result says it cost (settled as a lapsed reservation is, which is what
   * makes the caps and grants count it); only one that left
   * none never ran, and is settled at no cost. One a run already settled, or
   * one that has its own line, is left alone, so it can be called again for
   * the same plan.
   */
  async releaseUnrun(records: readonly ReservationRecord[], now: Date): Promise<void> {
    const costs = await this.finishedCosts()
    await this.deps.ledger.update((entries) => {
      const live = new Set(liveReservations(entries).map((reservation) => reservation.run_id))
      const lost = records.filter((record) => live.has(record.run_id))
      return {
        append: lost.map((record) => {
          const cost = costs.get(record.run_id)
          return cost === undefined ? { ...settledRecord(record, 0, now), reason: 'not-run' as const } : { ...settledRecord(record, cost, now), reason: 'expired' as const }
        }),
        result: undefined,
      }
    })
  }

  /**
   * Settles every reservation whose run left a result behind, at once and
   * whatever its expiry, and every one whose expiry has passed, from the run
   * directory's result where it left one and at its full estimate otherwise.
   * Every gateway call that reads the ledger for the caps runs this first, so a
   * run whose process died still counts, and one that finished counts what it
   * really cost rather than its estimate.
   */
  async settleStale(now: Date): Promise<void> {
    const costs = await this.finishedCosts()
    await this.deps.ledger.update((entries) => ({ append: settleExpired(entries, now, costs), result: undefined }))
  }

  /**
   * What each live reservation's run directory says the run cost, by run id: a
   * run that left a result is finished, whatever its expiry says. It is read
   * before the ledger's lock is taken, so the lock is held for the decision
   * alone; a reservation that settles meanwhile is skipped there.
   */
  private async finishedCosts(): Promise<ReadonlyMap<string, number>> {
    const costs = new Map<string, number>()
    for (const reservation of liveReservations(await this.deps.ledger.read())) {
      const cost = await this.resultCost(reservation.run_dir)
      if (cost !== undefined) costs.set(reservation.run_id, cost)
    }
    return costs
  }

  /**
   * What a run directory's result says the run cost, or undefined when it left
   * none the gateway can read: a directory it cannot look at, a file it cannot
   * read or parse, or a result without a cost all leave the reservation to its
   * expiry, which counts its full estimate.
   */
  private async resultCost(runDir: string): Promise<number | undefined> {
    const path = join(runDir, 'result.json')
    try {
      if (!(await this.deps.files.exists(path))) return undefined
      const parsed: unknown = JSON.parse(await this.deps.files.readText(path))
      const cost = isObject(parsed) ? parsed.cost_usd : undefined
      return typeof cost === 'number' && Number.isFinite(cost) && cost >= 0 ? cost : undefined
    } catch {
      return undefined
    }
  }
}

/**
 * The cost class a run is held to a cap by, or undefined for a subsidised run,
 * which its plan limit watches instead: metered-personal against the caps, and
 * metered-employer against its grant. A client the configuration does not
 * declare is refused (`unknown-client`), never treated as subsidised.
 */
export function heldCostClass(input: RunInput, config: GatewayConfig): CostClass | undefined {
  const client = config.clients[input.client]
  if (client === undefined) {
    throw new ModelGatewayError('unknown-client', `the gateway knows no client ${input.client}, so it may not run: a client the configuration does not declare is never held to nothing`)
  }
  const costClass = runCostClass(client, input)
  return costClass === 'subsidised' ? undefined : costClass
}

/** Why a run does not fit the metered-personal caps, naming the cap, its limit, what is spent and reserved, and the estimate. */
function capRefusal(
  entries: readonly LedgerEntry[],
  input: RunInput,
  estimateUsd: number,
  caps: MeteredPersonalCaps,
  now: Date,
): string | undefined {
  const decision = checkCaps(entries, input.workflow, estimateUsd, caps, now)
  if (decision.refused === undefined) return undefined
  return `${input.client}:${input.model} ${capRefusalMessage(decision.refused, estimateUsd, input.workflow, now)}`
}

/** The cost class of one run: the class of the provider it runs on, else its client's own. */
export function runCostClass(client: ClientConfig, input: RunInput): CostClass {
  return costClassFor(client, input.client === 'opencode' ? openCodeProvider(input.model) : input.provider)
}
