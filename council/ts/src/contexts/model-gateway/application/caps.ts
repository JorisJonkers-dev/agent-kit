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
  type ModelEntry,
} from '../domain/config.js'
import type { ReservationRecord } from '../domain/ledger.js'
import { costClassFor } from '../domain/models.js'
import { openCodeProvider, type RunInput } from '../domain/run.js'
import type { ModelGatewayDependencies } from './ports.js'

/**
 * What a metered-personal run holds against the caps while it runs: the
 * estimate it reserved, and the ledger line that holds it.
 */
export interface Reservation {
  readonly estimate: Estimate
  readonly record: ReservationRecord
}

/**
 * The metered-personal caps: what a run is estimated to cost, the reservation
 * it holds before it starts, and the settlement that replaces it with what the
 * run really cost.
 *
 * Only a run whose cost class is metered-personal is held to the caps: a
 * subsidised run is watched by its plan limit, and a metered-employer run by
 * its grant (#298). A reservation is decided and written in one locked
 * critical section that also settles the reservations whose runs died, so two
 * gateway processes cannot together overshoot a cap.
 */
export class CapGuard {
  constructor(private readonly deps: ModelGatewayDependencies) {}

  /**
   * What a metered run is estimated to cost, or a `no-estimate` refusal for a
   * client model the catalogue prices nowhere or gives no tier: a run the
   * gateway cannot estimate is never run uncapped.
   */
  estimate(input: RunInput, entry: ModelEntry | undefined, config: GatewayConfig): Estimate | undefined {
    if (!isMetered(input, config)) return undefined
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
   * Reserves the run's estimate against both caps, or refuses it with the cap
   * it does not fit. The decision and the reservation are one locked critical
   * section over the ledger, which also settles every reservation whose run
   * left no settlement and whose expiry has passed.
   */
  async reserve(args: {
    readonly config: GatewayConfig
    readonly costClass: CostClass
    readonly estimate: Estimate | undefined
    readonly input: RunInput
    readonly now: Date
    readonly runDir: string
    readonly runId: string
  }): Promise<Reservation | undefined> {
    const { config, costClass, estimate, input, now, runDir, runId } = args
    if (estimate === undefined) return undefined
    const costs = await this.finishedCosts()
    const record = reservationRecord({
      client: input.client,
      cost_class: costClass,
      estimate_usd: estimate.usd,
      model: input.model,
      reserved_at: now,
      run_dir: runDir,
      run_id: runId,
      time_limit_s: input.time_limit_s,
      workflow: input.workflow,
    })
    const decision = await this.deps.ledger.update((entries) => {
      const settled = settleExpired(entries, now, costs)
      const check = checkCaps([...entries, ...settled], input.workflow, estimate.usd, config.settings.metered_personal_caps, now)
      return { append: check.refused === undefined ? [...settled, record] : settled, result: check }
    })
    if (decision.refused !== undefined) {
      throw new ModelGatewayError(
        'cap-exceeded',
        `${input.client}:${input.model} ${capRefusalMessage(decision.refused, estimate.usd, input.workflow, now)}`,
      )
    }
    return { estimate, record }
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

/** Whether a run is held to the metered-personal caps: its cost class, which is its provider's where it names one. */
export function isMetered(input: RunInput, config: GatewayConfig): boolean {
  const client = config.clients[input.client]
  return client !== undefined && runCostClass(client, input) === 'metered-personal'
}

/** The cost class of one run: the class of the provider it runs on, else its client's own. */
export function runCostClass(client: ClientConfig, input: RunInput): CostClass {
  return costClassFor(client, input.client === 'opencode' ? openCodeProvider(input.model) : input.provider)
}
