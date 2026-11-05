import { randomBytes } from 'node:crypto'
import { join } from 'node:path'

import { ModelGatewayError, type ClientConfig, type CostClass, type GatewayConfig, type ModelEntry, type Tier } from '../domain/config.js'
import { runRecords, type LedgerRecord } from '../domain/ledger.js'
import { decidePolicy } from '../domain/policy.js'
import { priceRun, runId, type RunInput, type RunTokens } from '../domain/run.js'
import { describeWindow, planWindows } from '../domain/usage.js'
import { requireWorkflowName, requireWorkflowOpen, startWorkflow } from '../domain/workflows.js'
import { CapGuard, runCostClass, type CapDependencies, type CapSource, type Reservation } from './caps.js'
import { loadConfig } from './config.js'
import { GrantGuard, type GrantGuardDependencies } from './grants.js'
import { overridePath, runsDir } from './locations.js'
import { resolveScope, type ScopeDependencies } from './policy.js'
import type { ModelGatewayDependencies } from './ports.js'

/**
 * How long an engine run's reservation holds before the gateway settles it at
 * its full estimate: four hours, because a supervised worker runs far longer
 * than a delegation does, and a reservation that lapses under a live worker
 * would count it twice. The caller may name a limit of its own.
 */
export const ENGINE_TIME_LIMIT_S = 4 * 60 * 60

/** What the guard reads: the same ports the gateway's own runs use, without the clients, proofs or worktrees. */
export type EngineGuardDependencies = CapDependencies &
  GrantGuardDependencies &
  ScopeDependencies &
  Pick<ModelGatewayDependencies, 'clock' | 'defaultsPath'>

/** One engine process a caller is about to start itself, and what the gateway needs to know to allow it. */
export interface EngineAuthorizeRequest {
  /** The workflow the run belongs to; held to the same name rule as every gateway workflow. */
  readonly workflow: string
  /** The client that runs it, as the gateway names it (`codex`, `claude`). */
  readonly client: string
  /** The model, as the catalogue or a client alias (`gpt-6.1-sol`, `haiku`) names it. */
  readonly model: string
  /** How long the prompt is, in characters, which the estimate counts. */
  readonly prompt_chars: number
  /** The run's difficulty tier, where the catalogue's own would not do. */
  readonly tier?: Tier
  /** The repository the engine works in; the data policy is decided for it. A run in none carries only inline content. */
  readonly repo?: string
  /** How long the run may take, in seconds. */
  readonly time_limit_s?: number
  /**
   * Whether the prompt may carry history (commit messages, issue and ticket
   * text), which the repository's allowlist entry must then allow. It does
   * unless the caller says it does not: an unsure caller fails closed.
   */
  readonly carries_history?: boolean
}

/** What an authorised engine run holds until `record` settles it. */
export interface EngineReservation {
  /** The id the run's ledger line and its settlement carry. */
  readonly run_id: string
  readonly workflow: string
  readonly client: string
  /** The catalogue's model the request named, which is the one the ledger line carries. */
  readonly model: string
  readonly family: string
  readonly cost_class: CostClass
  /** The repository the run worked in, keyed `<host>/<owner>/<repo>`; null for an inline run. */
  readonly repository: string | null
  /** The path of the repository the run was authorised in, which a re-attach is held to again; absent for an inline run. */
  readonly repo?: string
  /** What the run was estimated to cost, which is what it is recorded at when nothing better is reported; 0 for a subsidised run. */
  readonly estimate_usd: number
  /** The hold against the caps or the grant; undefined for a subsidised run, which holds nothing. */
  readonly hold: Reservation | undefined
}

/** The gateway's answer to an engine run it allows. A refusal is a `ModelGatewayError` with the gateway's own code and message. */
export interface EngineAuthorization {
  readonly reservation: EngineReservation
}

/** What one engine run came to, as its caller knows it. */
export interface EngineRecordInput {
  readonly reservation: EngineReservation
  readonly outcome: 'ok' | 'failed' | 'timeout'
  /** What the engine reported it used; absent when it reported none. */
  readonly tokens?: RunTokens
  /** The dollars the engine reported; absent when it priced nothing. */
  readonly cost_usd?: number
  readonly duration_s: number
  /** The reservation's own run id, which the ledger line carries; any other is refused. */
  readonly run_id: string
  /** Why a failed run failed, when its caller knows. */
  readonly reason?: string
  /** The caller re-attached to the run after its first supervisor died, so its start was not seen. */
  readonly reattached?: boolean
}

/** A run that is already going, asked about again: whether what authorised it still holds. */
export interface EngineReauthorizeRequest {
  readonly reservation: EngineReservation
  /** The repository the engine works in, which the data policy is decided for again. */
  readonly repo?: string
  /** Whether the prompt may carry history; true unless the caller says it carries none. */
  readonly carries_history?: boolean
}

/**
 * The gateway's guard for an engine process another program launches itself:
 * it decides before the process starts, and writes down what the process came
 * to once it ends. Council's `claude` and `codex` engines stream and supervise
 * long-running workers, which the gateway's `run` (one result, captured at the
 * end) cannot do, so council keeps its own launch and the gateway stands
 * either side of it, with the checks and the ledger a delegation gets.
 *
 * `authorize` reuses what a run is held to: the workflow's name and life, the
 * data policy for the repository, the plan limit of a subsidised client, and
 * either the live grant a metered-employer client needs or the caps a
 * metered-personal one is held to, reserving the estimate in the same locked
 * critical section. Nothing is reserved or started for a run it refuses.
 */
export class EngineGuard {
  private readonly caps: CapGuard
  private readonly grants: GrantGuard

  constructor(private readonly deps: EngineGuardDependencies) {
    this.caps = new CapGuard(deps)
    this.grants = new GrantGuard(deps)
  }

  /** Allows the run and holds what it may cost, or refuses it with the gateway's own code and message. */
  async authorize(request: EngineAuthorizeRequest): Promise<EngineAuthorization> {
    const workflow = requireWorkflowName(request.workflow, 'an engine run must name its workflow')
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const client = config.clients[request.client]
    if (client === undefined) {
      throw new ModelGatewayError('unknown-client', `the gateway knows no client ${request.client}, so it may not run: a client the configuration does not declare is never held to nothing`)
    }
    const entry = this.requireEntry(config, request.client, request.model)
    const { family, model } = entry
    const input = engineInput(request, workflow, model)
    const repository = await this.requirePermission(input, config, family, request.carries_history ?? true)
    const costClass = runCostClass(client, input)
    const now = this.deps.clock.now()
    await this.requireNotExhausted(request.client, client, config, model, now)
    requireWorkflowOpen(await this.deps.workflows.read(), workflow, now)
    const priced = request.tier === undefined ? entry : { ...entry, tier: request.tier }
    const estimate = costClass === 'subsidised' ? undefined : this.caps.estimate(input, priced, config)
    const cap = await this.capSource(costClass, workflow, request.client)
    const id = runId(now, randomBytes(3).toString('hex'))
    const hold = await this.caps.reserve({
      cap,
      config,
      costClass,
      estimate,
      input,
      now: this.deps.clock.now(),
      runDir: join(runsDir(this.deps.env), id),
      runId: id,
    })
    await this.joinWorkflow(workflow, hold)
    return {
      reservation: {
        client: request.client,
        cost_class: costClass,
        estimate_usd: estimate?.usd ?? 0,
        family,
        hold,
        model,
        repository,
        run_id: id,
        ...(request.repo === undefined ? {} : { repo: request.repo }),
        workflow,
      },
    }
  }

  /**
   * Whether what authorised a run that is already going still holds, so a
   * restart or a re-attach never spends on an authorisation that lapsed: its
   * workflow is open, its repository's policy still allows it, a subsidised
   * client is below its plan limit, and a metered-employer run's grant is still
   * live with room. A refusal is the gateway's own error.
   */
  async reauthorize(request: EngineReauthorizeRequest): Promise<void> {
    const { reservation } = request
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const client = config.clients[reservation.client]
    if (client === undefined) {
      throw new ModelGatewayError('unknown-client', `the gateway knows no client ${reservation.client}, so it may not run: a client the configuration does not declare is never held to nothing`)
    }
    this.requireEntry(config, reservation.client, reservation.model)
    // The repository the lease was taken for: a re-attach that names none is
    // held to the same one, and one that names another is refused.
    const repo = request.repo ?? reservation.repo
    if (repo === undefined && reservation.repository !== null) {
      throw new ModelGatewayError('repository-unreadable', `the run was authorised in ${reservation.repository}, and no repository is named to re-attach in: it is not resumed`)
    }
    const input = engineInput(
      { client: reservation.client, model: reservation.model, prompt_chars: 0, workflow: reservation.workflow, ...(repo === undefined ? {} : { repo }) },
      reservation.workflow,
      reservation.model,
    )
    const repository = await this.requirePermission(input, config, reservation.family, request.carries_history ?? true)
    if (repository !== reservation.repository) {
      throw new ModelGatewayError('policy-refused', `the run was authorised in ${reservation.repository ?? 'no repository'}, not ${repository ?? 'no repository'}: it is not resumed there`)
    }
    const now = this.deps.clock.now()
    await this.requireNotExhausted(reservation.client, client, config, reservation.model, now)
    requireWorkflowOpen(await this.deps.workflows.read(), reservation.workflow, now)
    if (runCostClass(client, input) === 'metered-employer') await this.grants.require(reservation.workflow, reservation.client, now)
  }

  /**
   * Writes the run's ledger line (a write run, from council) and releases its
   * reservation at what the run cost: the engine's own figure, else its tokens
   * at the catalogue price, else the estimate the run was held at. A timeout is
   * a failed run that timed out.
   */
  async record(input: EngineRecordInput): Promise<void> {
    const { reservation } = input
    if (input.run_id !== reservation.run_id) {
      throw new ModelGatewayError('invalid-input', `run ${input.run_id} is not the run its reservation holds (${reservation.run_id}): a record settles only its own reservation`)
    }
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const entry = resolveEntry(config, reservation.client, reservation.model)
    const cost = costOf(input, entry)
    const reason = input.outcome === 'timeout' ? 'timeout' : input.reason
    const line: LedgerRecord = {
      at: this.deps.clock.now().toISOString(),
      client: reservation.client,
      cost_class: reservation.cost_class,
      cost_usd: cost,
      duration_s: input.duration_s,
      family: reservation.family,
      mode: 'write',
      model: reservation.model,
      outcome: input.outcome === 'ok' ? 'ok' : 'failed',
      repository: reservation.repository,
      run_id: reservation.run_id,
      source: 'council',
      workflow: reservation.workflow,
      ...(input.tokens === undefined ? {} : { tokens: input.tokens }),
      ...(reason === undefined ? {} : { reason }),
      ...(input.reattached === true ? { reattached: true } : {}),
    }
    await this.deps.ledger.append(line)
    await this.caps.settle(reservation.hold, cost, this.deps.clock.now())
  }

  /** The data policy's refusal for this client model and repository, or the repository's key; a worker works in a repository, so it carries code, and history unless its caller says it carries none. */
  private async requirePermission(input: RunInput, config: GatewayConfig, family: string, carriesHistory: boolean): Promise<string | null> {
    const scope = await resolveScope(this.deps, input.repo)
    const decision = decidePolicy(config, scope, [{ client: input.client, family, model: input.model }])
    const permission = decision.permissions[0]
    if (!permission?.permitted) {
      throw new ModelGatewayError('policy-refused', `${input.client}:${input.model} is not permitted: ${decision.reason}`)
    }
    if (!permission.may_send.code && input.repo !== undefined) {
      throw new ModelGatewayError('policy-refused', `${input.client}:${input.model} may not carry code: ${decision.reason}`)
    }
    if (carriesHistory && !permission.may_send.history) {
      throw new ModelGatewayError('policy-refused', `${input.client}:${input.model} may not carry history: ${decision.reason}`)
    }
    return scope.kind === 'repository' ? scope.key : null
  }

  /**
   * The catalogue's entry for the model a run names, or a refusal: a model the
   * catalogue lacks cannot be priced or held to a cap, so it does not run at
   * all, and a model the owner disabled (by its id, by any name it is recorded
   * under, or by the name the caller used) is refused however it is named. The
   * request is resolved to the entry first and `disabled` is read against the
   * entry, so an alias cannot get past what the owner switched off.
   */
  private requireEntry(config: GatewayConfig, client: string, model: string): ModelEntry {
    const entry = resolveEntry(config, client, model)
    if (entry === undefined) {
      throw new ModelGatewayError(
        'no-estimate',
        `${client}:${model} is not in the catalogue, so a run on it cannot be priced or held to a cap: add it to models in defaults.json or in the machine file ${overridePath(this.deps.env)}`,
      )
    }
    const names = [entry.model, ...(entry.names ?? []), model]
    if (names.some((name) => config.disabled.includes(`${client}:${name}`))) {
      throw new ModelGatewayError('unknown-model', `${client}:${model} is disabled in this machine's configuration`)
    }
    return entry
  }

  /** A subsidised client at 100% of a plan window cannot start: the gateway says so rather than let the engine fail. */
  private async requireNotExhausted(
    clientId: string,
    client: ClientConfig,
    config: GatewayConfig,
    model: string,
    now: Date,
  ): Promise<void> {
    if (client.cost_class !== 'subsidised') return
    const records = runRecords(await this.deps.ledger.read())
    const windows = planWindows(clientId, client.plan_limits ?? [], records, now, config.settings.plan_warning_ratio)
    const exhausted = windows.find((window) => window.state === 'exhausted')
    if (exhausted !== undefined) {
      throw new ModelGatewayError('unable-to-start', `${clientId}:${model} cannot start: at 100% of ${describeWindow(exhausted)}`)
    }
  }

  /** What the run's estimate is held against: the grant a metered-employer run needs, else the metered-personal caps. */
  private async capSource(costClass: CostClass, workflow: string, client: string): Promise<CapSource> {
    if (costClass !== 'metered-employer') return { kind: 'metered-personal' }
    return { grant: await this.grants.require(workflow, client, this.deps.clock.now()), kind: 'grant' }
  }

  /** The workflow starts with its first run, and a run that cannot start it gives its hold back. */
  private async joinWorkflow(workflow: string, hold: Reservation | undefined): Promise<void> {
    const now = this.deps.clock.now()
    try {
      await this.deps.workflows.update((workflows) => startWorkflow(workflows, workflow, now))
    } catch (error) {
      await this.caps.release(hold, now)
      throw error
    }
  }
}

/**
 * The run as the gateway's own checks read it: one writing run of the named
 * client model, whose prompt is as long as the caller says. Only the prompt's
 * length is ever counted, so the text is blank.
 */
function engineInput(request: EngineAuthorizeRequest, workflow: string, model: string): RunInput {
  return {
    client: request.client,
    mode: 'write',
    model,
    prompt: ' '.repeat(Math.max(0, Math.trunc(request.prompt_chars))),
    time_limit_s: request.time_limit_s ?? ENGINE_TIME_LIMIT_S,
    workflow,
    ...(request.repo === undefined ? {} : { repo: request.repo }),
  }
}

/** The names council documents for Claude's models, mapped to the catalogue's ids; nothing else is matched loosely. */
const COUNCIL_ALIASES: Readonly<Record<string, Readonly<Record<string, string>>>> = {
  claude: { haiku: 'claude-haiku-5-5', opus: 'claude-opus-5-5', sonnet: 'claude-sonnet-5-5' },
}

/**
 * The catalogue's entry a model name resolves to: its exact id, else an exact
 * name it is recorded under, else the catalogue id a council alias maps to. No
 * name is matched by a word or a prefix.
 */
function resolveEntry(config: GatewayConfig, client: string, model: string): ModelEntry | undefined {
  const own = config.models.filter((candidate) => candidate.client === client)
  const byId = (id: string | undefined): ModelEntry | undefined => own.find((candidate) => candidate.model === id)
  return byId(model) ?? own.find((candidate) => candidate.names?.includes(model) === true) ?? byId(COUNCIL_ALIASES[client]?.[model])
}

/** What the run cost: the engine's own figure, else its tokens at the catalogue price, else what it was held at. */
function costOf(input: EngineRecordInput, entry: ModelEntry | undefined): number {
  if (input.cost_usd !== undefined) return input.cost_usd
  if (input.tokens !== undefined) return priceRun(undefined, input.tokens, entry?.prices)
  return input.reservation.estimate_usd
}
