import { dirname, join } from 'node:path'

import { ModelGatewayError } from '../domain/config.js'
import {
  attemptInput,
  bindFamily,
  candidateKey,
  candidatePool,
  CannotStartError,
  climbOrder,
  DEFAULT_MAX_ATTEMPTS,
  escalationNotice,
  fallThroughOrder,
  isCannotStart,
  isRefusal,
  requestedCandidate,
  retryPrompt,
  type AttemptLink,
  type AttemptSummary,
  type Candidate,
  type EscalationReport,
  type Unstartable,
  type UnavailableResult,
} from '../domain/escalation.js'
import { familyOfModelId } from '../domain/models.js'
import { decidePolicy } from '../domain/policy.js'
import type { RunInput } from '../domain/run.js'
import { loadConfig } from './config.js'
import { GrantGuard } from './grants.js'
import { messageOf } from './json-file.js'
import { overridePath } from './locations.js'
import { resolveScope } from './policy.js'
import type { ModelGatewayDependencies } from './ports.js'
import { clientAvailability } from './usage.js'

/** What the escalation needs to know of one finished run, whatever else its result carries. */
export interface AttemptedRun {
  readonly status: 'ok' | 'failed'
  readonly run_id: string
  readonly client: string
  readonly model: string
  readonly family?: string
  readonly summary?: string
  readonly reason?: string
  readonly output_path: string
  readonly check?: { readonly output_path: string }
}

/** What one attempt is launched with: how it links to the first, and the check that its client can start. */
export interface AttemptOptions {
  /** Absent on the first attempt, which nothing links to. */
  readonly link?: AttemptLink
  /**
   * Called once everything that can refuse the run has passed and before
   * anything is launched; a reason says the client cannot start, and the
   * attempt then ends with a `CannotStartError` instead of a run.
   */
  readonly startCheck?: (input: RunInput) => Promise<string | undefined>
}

/** One attempt: the single-run path, which the escalation wraps. */
export type AttemptRunner<R extends AttemptedRun> = (input: RunInput, options: AttemptOptions) => Promise<R>

/** A delegation's last run, with what the escalation did to get there. */
export type Escalated<R> = R & EscalationReport

/** Where the delegation's catalogue stands: the model asked for and every model it may move to. */
interface Plan {
  readonly requested: Candidate
  readonly pool: readonly Candidate[]
}

/**
 * A delegation that does not come back clean. A failed result is retried once
 * on the same client model with the failure attached; a second failure climbs
 * to the next stronger permitted model that is no costlier; a client that
 * cannot start is swapped for another client model of the same cost class
 * that covers the tier. When nothing is left the last result goes back with
 * `escalation: "exhausted"`, or the delegation is `unavailable` when no
 * client model of its cost class can start. Each attempt is a run of its own.
 *
 * Batch delegations call `run` once each: it takes the single-run path as a
 * function, so it knows nothing of how a run is launched.
 */
export class EscalatedRun {
  private readonly grants: GrantGuard

  constructor(private readonly deps: ModelGatewayDependencies) {
    this.grants = new GrantGuard(deps)
  }

  async run<R extends AttemptedRun>(input: RunInput, attempt: AttemptRunner<R>): Promise<Escalated<R> | UnavailableResult> {
    if (input.escalate === false) {
      const only = await attempt(input, {})
      return { ...only, attempts: [summaryOf(only)] }
    }
    const maxAttempts = input.max_attempts ?? DEFAULT_MAX_ATTEMPTS
    const attempts: AttemptSummary[] = []
    const skipped: Unstartable[] = []
    const dead = new Set<string>()
    let plan: Plan | undefined
    const planned = async (): Promise<Plan> => (plan ??= await this.plan(input))
    /** The model to launch next; undefined is the model the caller asked for, launched as the caller sent it. */
    let current: Candidate | undefined
    /** The model the next swap is measured from: the one the slot wanted, or the one that failed twice when climbing. */
    let ref: Candidate | undefined
    let mode: 'same' | 'climb' = 'same'
    let failures = 0
    /** The attempts that count toward `max_attempts`: a run whose client could not start does not. */
    let counted = 0
    let prompt = input.prompt
    let last: R | undefined
    /** The last attempt that really ran: a run with an output path and a cost, which a later attempt that cannot start never replaces. */
    let real: R | undefined

    /** The summary of a run answered by another family: it opens with the escalation, both families and the cause. */
    const stated = async (run: R): Promise<Pick<AttemptedRun, 'summary'>> => {
      if (run.status !== 'ok' || run.family === undefined || run.summary === undefined) return {}
      const { requested } = await planned()
      if (run.family === requested.family) return {}
      const cause = skipped.find((who) => who.client === requested.client && who.model === requested.model)?.reason ?? attempts[0]?.reason ?? 'failed'
      return { summary: `${escalationNotice(requested, cause, { family: run.family })}\n${run.summary}` }
    }
    const done = async (run: R, escalation?: 'exhausted' | 'stopped', stoppedReason?: string): Promise<Escalated<R>> => ({
      ...run,
      ...(await stated(run)),
      attempts,
      ...(escalation === undefined ? {} : { escalation }),
      ...(skipped.length === 0 ? {} : { skipped }),
      ...(stoppedReason === undefined ? {} : { stopped_reason: stoppedReason }),
    })
    const unavailable = async (): Promise<UnavailableResult> => {
      const from = ref ?? (await planned()).requested
      const binding = input.family_binding === false ? '' : `; the family ${from.family} is binding, so no other family answers`
      return {
        attempts,
        client: input.client,
        message: `${from.client}:${from.model} cannot start, and no other permitted ${from.costClass} client model covers its tier; the gateway never moves into a costlier cost class by itself${binding}`,
        model: input.model,
        reasons: skipped,
        status: 'unavailable',
        workflow: input.workflow,
      }
    }
    const passOver = (who: Pick<Candidate, 'client' | 'model'>, reason: string): void => {
      dead.add(candidateKey(who))
      skipped.push({ client: who.client, model: who.model, reason })
    }
    const replacement = async (): Promise<Candidate | undefined> => {
      const { pool, requested } = await planned()
      const from = ref ?? requested
      const order = mode === 'climb' ? climbOrder(from, pool) : fallThroughOrder(from, pool)
      return order.find((candidate) => !dead.has(candidateKey(candidate)))
    }

    for (;;) {
      if (last !== undefined && counted >= maxAttempts) return await done(last, 'exhausted')
      const link: AttemptLink | undefined =
        attempts.length === 0 ? undefined : { attempt: attempts.length + 1, attempt_of: String(attempts[0]?.run_id) }
      let run: R
      try {
        const next = current === undefined ? input : attemptInput(input, current, prompt)
        run = await attempt(next, { ...(link === undefined ? {} : { link }), startCheck: (checked) => this.startCheck(checked) })
      } catch (error) {
        if (!(error instanceof CannotStartError)) {
          if (last === undefined) throw error
          return await done(last, 'stopped', error instanceof ModelGatewayError ? `${error.code}: ${error.message}` : messageOf(error))
        }
        passOver(current ?? input, error.message)
        const swapped = await replacement()
        if (swapped === undefined) {
          if (real === undefined) return await unavailable()
          return mode === 'climb' ? await done(real, 'exhausted') : await done(real, 'stopped', error.message)
        }
        current = swapped
        continue
      }
      attempts.push(summaryOf(run))
      last = run
      if (run.status === 'ok') return await done(run)
      if (isCannotStart(run.reason)) {
        passOver(current ?? input, run.reason ?? 'launch-failed')
        const swapped = await replacement()
        if (swapped === undefined) {
          if (real === undefined) return await unavailable()
          return mode === 'climb' ? await done(real, 'exhausted') : await done(real, 'stopped', run.reason ?? 'launch-failed')
        }
        current = swapped
        continue
      }
      real = run
      counted += 1
      if (isRefusal(run.reason)) return await done(run)
      failures += 1
      ref = current ?? (await planned()).requested
      current = ref
      mode = 'same'
      prompt = retryPrompt(input.prompt, run.reason ?? 'failed', await this.failureDetail(run))
      if (failures === 1) continue
      mode = 'climb'
      const stronger = await replacement()
      if (stronger === undefined) return await done(run, 'exhausted')
      current = stronger
      failures = 0
    }
  }

  /**
   * The catalogue the delegation may move through: the model asked for, and
   * every permitted model it may be moved to. A metered-employer model is in
   * that pool only while a live grant covers the workflow, so a client that
   * cannot start is swapped for one of them only when the owner allowed it.
   */
  private async plan(input: RunInput): Promise<Plan> {
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const entry = config.models.find((candidate) => candidate.client === input.client && candidate.model === input.model)
    const family = entry?.family ?? familyOfModelId(input.model) ?? ''
    const requested = requestedCandidate(config, input, entry, family)
    const scope = await resolveScope(this.deps, input.repo)
    const granted = await this.grants.liveClients(input.workflow, this.deps.clock.now())
    const pool = bindFamily(input, requested, candidatePool(config, input, decidePolicy(config, scope, config.models).permissions, granted))
    return { pool, requested }
  }

  /** Why a client cannot start, or undefined when it can: what `usage` reports as its availability. */
  private async startCheck(input: RunInput): Promise<string | undefined> {
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const availability = await clientAvailability(this.deps, config, input.client)
    return availability === undefined || availability.status === 'available' ? undefined : `${availability.status}: ${availability.reason}`
  }

  /**
   * What a failed attempt tells the next one: the check's output for a writing
   * run whose check failed, else what the client printed on stderr, else its
   * output. Whatever cannot be read says nothing.
   */
  private async failureDetail(run: AttemptedRun): Promise<string> {
    const check = run.reason?.startsWith('check-') === true ? run.check?.output_path : undefined
    for (const path of [check, join(dirname(run.output_path), 'stderr.txt'), run.output_path]) {
      const text = path === undefined ? '' : await this.readQuietly(path)
      if (text.trim() !== '') return text
    }
    return ''
  }

  /** A file's text, or nothing when it is not there or cannot be read: a missing stderr file is a client that printed none. */
  private async readQuietly(path: string): Promise<string> {
    try {
      return await this.deps.files.readText(path)
    } catch {
      return ''
    }
  }
}

function summaryOf(run: AttemptedRun): AttemptSummary {
  return {
    client: run.client,
    model: run.model,
    run_id: run.run_id,
    status: run.status,
    ...(run.reason === undefined ? {} : { reason: run.reason }),
  }
}
