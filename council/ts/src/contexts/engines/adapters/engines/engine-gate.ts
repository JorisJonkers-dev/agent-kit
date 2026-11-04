import type { EngineAuthorizeRequest, EngineGuard, EngineReservation, RunTokens } from '../../../model-gateway/index.js'
import { engineWorkflowName, isGuardedEngine } from '../../domain/guard.js'

/** What council asks of the gateway: to allow a run before its process starts, and to write down what it came to. */
export type EngineRunGuard = Pick<EngineGuard, 'authorize' | 'reauthorize' | 'record'>

/** The clock the gate times a run by: wall time, so a lease that outlives its supervisor can still be timed. */
export interface EngineGateClock {
  now(): Date
}

/** One engine process council is about to start. */
export interface EngineLaunch {
  readonly engine: string
  readonly model: string
  /** The council run's id, which the gateway names its workflow by once the name rule has been applied. */
  readonly workflow: string
  /** How long the prompt is, in characters. */
  readonly promptChars: number
  /** The repository the engine works in, which the data policy is decided for. */
  readonly repo?: string
  /** How long the run may take, in seconds. */
  readonly timeLimitS?: number
  /** Whether the prompt may carry history (ticket and issue text); true unless the caller says it carries none. */
  readonly carriesHistory?: boolean
  /** The worker's task, which its lease is kept for. */
  readonly taskId?: string
}

/** What one engine process came to, as council knows it. Tokens and cost are what the engine reported; absent, the gateway estimates from the catalogue price. */
export interface EngineEnd {
  readonly outcome: 'ok' | 'failed' | 'timeout'
  readonly reason?: string
  readonly tokens?: RunTokens
  readonly costUsd?: number
}

/** What a lease keeps so another process can end it: the gateway's reservation and when the run started. */
export interface PersistedEngineLease {
  /** The council run, the task, the engine and the model the lease was opened for, which a resume must find unchanged. */
  readonly council_run: string
  readonly task_id: string
  readonly engine: string
  readonly model: string
  readonly reservation: EngineReservation
  readonly started_at: string
  /** The lease was opened for a run whose supervisor died, so its real start was not seen. */
  readonly reattached?: true
}

/** An authorised run, until its end is written down. */
export interface EngineLease {
  /** The id the gateway's ledger line for this run carries. */
  readonly runId: string
  /** What another process needs to end this lease if this one dies first. */
  persisted(): PersistedEngineLease
  /** Records the run in the gateway's ledger and settles its reservation; a second call records nothing. */
  end(end: EngineEnd): Promise<void>
}

/**
 * Council's door to the gateway for the engines it launches itself. Before a
 * `claude` or `codex` process starts the gateway authorises it (the live grant,
 * the caps, the data policy, the plan limit); after it ends the gateway records
 * it. Any other engine is council's own and passes through unguarded.
 *
 * Council keeps its own launch, streaming and supervision: the gateway stands
 * either side of them and never sees the process.
 */
export class EngineGate {
  constructor(
    private readonly guard: EngineRunGuard,
    private readonly clock: EngineGateClock,
  ) {}

  /**
   * The lease for a guarded engine's run, or undefined for an engine the
   * gateway does not guard. A refusal is the gateway's own error, raised before
   * anything is started. `reattached` marks a lease opened for a run whose
   * supervisor died and whose own lease was lost.
   */
  async open(launch: EngineLaunch, options: { readonly reattached?: boolean } = {}): Promise<EngineLease | undefined> {
    return isGuardedEngine(launch.engine) ? this.begin(launch, options) : undefined
  }

  /** `open` for an engine already known to be guarded: always a lease, and an error for an engine that is not guarded. */
  async begin(launch: EngineLaunch, options: { readonly reattached?: boolean } = {}): Promise<EngineLease> {
    if (!isGuardedEngine(launch.engine)) throw new Error(`${launch.engine} is not an engine the model gateway guards`)
    const request: EngineAuthorizeRequest = {
      carries_history: launch.carriesHistory ?? true,
      client: launch.engine,
      model: launch.model,
      prompt_chars: launch.promptChars,
      workflow: engineWorkflowName(launch.workflow),
      ...(launch.repo === undefined ? {} : { repo: launch.repo }),
      ...(launch.timeLimitS === undefined ? {} : { time_limit_s: launch.timeLimitS }),
    }
    const { reservation } = await this.guard.authorize(request)
    return this.lease({
      council_run: launch.workflow,
      engine: launch.engine,
      model: launch.model,
      reservation,
      task_id: launch.taskId ?? '',
      started_at: this.clock.now().toISOString(),
      ...(options.reattached === true ? { reattached: true as const } : {}),
    })
  }

  /**
   * The lease a dead supervisor left, picked up again once the gateway agrees
   * that what authorised the run still holds (its grant, its workflow, the
   * repository's policy, the plan limit). If it does not, the lease is ended as
   * a failed run with the gateway's reason and the gateway's refusal is raised,
   * so a restart never spends on an authorisation that lapsed.
   */
  async resume(persisted: PersistedEngineLease, context: { readonly repo?: string; readonly carriesHistory?: boolean } = {}): Promise<EngineLease> {
    const lease = this.lease(persisted)
    try {
      await this.guard.reauthorize({
        carries_history: context.carriesHistory ?? true,
        reservation: persisted.reservation,
        ...(context.repo === undefined ? {} : { repo: context.repo }),
      })
    } catch (error) {
      const code = typeof error === 'object' && error !== null && 'code' in error && typeof error.code === 'string' ? error.code : 'reauthorize-failed'
      await lease.end({ outcome: 'failed', reason: code })
      throw error
    }
    return lease
  }

  private lease(persisted: PersistedEngineLease): EngineLease {
    const { reservation } = persisted
    let ended = false
    return {
      end: async (end) => {
        if (ended) return
        ended = true
        await this.guard.record({
          duration_s: Math.max(0, Math.round(this.clock.now().getTime() - Date.parse(persisted.started_at))) / 1000,
          outcome: end.outcome,
          reservation,
          run_id: reservation.run_id,
          ...(end.reason === undefined ? {} : { reason: end.reason }),
          ...(end.tokens === undefined ? {} : { tokens: end.tokens }),
          ...(end.costUsd === undefined ? {} : { cost_usd: end.costUsd }),
          ...(persisted.reattached === true ? { reattached: true } : {}),
        })
      },
      persisted: () => persisted,
      runId: reservation.run_id,
    }
  }
}

/** What the lease file under a worker's directory holds: an open lease, or the mark that its run was recorded. */
export type EngineLeaseFile = { readonly state: 'open'; readonly lease: PersistedEngineLease } | { readonly state: 'ended' }

/** The lease file's text. */
export function engineLeaseFileText(file: EngineLeaseFile): string {
  return `${JSON.stringify(file, null, 2)}\n`
}

/** Reads a lease file; a file that is not one is an error, never an absent lease, so a worker is not run past the gateway. */
export function parseEngineLeaseFile(text: string): EngineLeaseFile {
  let raw: unknown
  try {
    raw = JSON.parse(text)
  } catch {
    throw new Error('the engine lease file is not JSON')
  }
  if (typeof raw === 'object' && raw !== null && 'state' in raw) {
    if (raw.state === 'ended') return { state: 'ended' }
    if (raw.state === 'open' && 'lease' in raw && isPersistedLease(raw.lease)) return { lease: raw.lease, state: 'open' }
  }
  throw new Error('the engine lease file is not a lease the gateway wrote')
}

function isPersistedLease(value: unknown): value is PersistedEngineLease {
  if (typeof value !== 'object' || value === null || !('reservation' in value) || !('started_at' in value)) return false
  const { reservation, started_at: startedAt } = value
  const text = (key: string): boolean => key in value && typeof (value as Record<string, unknown>)[key] === 'string'
  return text('council_run') && text('task_id') && text('engine') && text('model') && typeof startedAt === 'string' && typeof reservation === 'object' && reservation !== null && 'run_id' in reservation && typeof reservation.run_id === 'string'
}
