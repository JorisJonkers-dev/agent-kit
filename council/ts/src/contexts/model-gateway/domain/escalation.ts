import { TIERS, type CostClass, type GatewayConfig, type ModelEntry, type Tier } from './config.js'
import { costClassFor, modelKey } from './models.js'
import type { ModelPermission } from './policy.js'
import { openCodeProvider, PROVIDER_CLIENTS, requireAllowedProvider, RUN_CLIENTS, type RunInput } from './run.js'

/**
 * What happens to a delegation that does not come back clean: it is retried
 * once on the same client model with the failure attached, then climbs to the
 * next stronger permitted subsidised model, and a client that cannot start is
 * swapped for another of the same cost class. The gateway never moves into a
 * costlier cost class on its own.
 */

/** A delegation gets this many attempts in all: the first, the retry, and one climb. */
export const DEFAULT_MAX_ATTEMPTS = 3

/** How many characters of a failed attempt's output the next attempt is shown. */
export const FAILURE_TAIL_CHARS = 1000

/**
 * The reasons that are refusals, never failures of a run: a delegation the
 * gateway refuses is refused again on the same terms, so none is retried.
 * Refusals are errors that stop the call before anything is launched; the
 * codes are listed here as well so one that surfaces as a result's reason
 * (the caps and the grants) is still never retried.
 */
const REFUSAL_REASONS: readonly string[] = [
  'cap-exceeded',
  'no-grant',
  'invalid-config',
  'invalid-input',
  'nested-delegation',
  'no-estimate',
  'not-supported-yet',
  'policy-refused',
  'provider-not-allowed',
  'repository-unreadable',
  'unknown-client',
  'unknown-model',
  'unknown-workflow',
  'workflow-ended',
  'write-needs-repository',
  'worktree-failed',
]

/** What a client's own error channel says when it will not start: it is signed out or out of allowance, or its launch failed. */
const CANNOT_START_REASONS: readonly string[] = ['auth', 'launch-failed', 'rate-limited']

/** Whether a failed result's reason is a refusal, which never retries. */
export function isRefusal(reason: string | undefined): boolean {
  return reason !== undefined && REFUSAL_REASONS.includes(reason)
}

/** Whether a failed result's reason says the client could not start, which swaps the client rather than retrying it. */
export function isCannotStart(reason: string | undefined): boolean {
  return reason !== undefined && CANNOT_START_REASONS.includes(reason)
}

/** Raised from inside a launch when its client cannot start, so the delegation swaps the client rather than failing. */
export class CannotStartError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'CannotStartError'
  }
}

/** The link a retried or climbed attempt carries to the first attempt of its delegation. */
export interface AttemptLink {
  /** This attempt's place in the delegation, 1 for the first. */
  readonly attempt: number
  /** The run id of the delegation's first attempt; absent on the first. */
  readonly attempt_of?: string
}

/** One attempt, as the result lists it. */
export interface AttemptSummary {
  readonly run_id: string
  readonly client: string
  readonly model: string
  readonly status: 'ok' | 'failed'
  readonly reason?: string
}

/** A client model the delegation could not start, and why. */
export interface Unstartable {
  readonly client: string
  readonly model: string
  readonly reason: string
}

/** What a delegation adds to its last run's result once it is escalated. */
export interface EscalationReport {
  readonly attempts: readonly AttemptSummary[]
  /**
   * `exhausted` when the delegation ends on a failed result with nothing left
   * to try, `stopped` when a later attempt could not start and no client model
   * could stand in for it.
   */
  readonly escalation?: 'exhausted' | 'stopped'
  /** The client models passed over because they could not start. */
  readonly skipped?: readonly Unstartable[]
  /** Why the escalation stopped early: what a later attempt could not start with, or the refusal it ended on. */
  readonly stopped_reason?: string
}

/** A delegation no permitted client model of its cost class can start. */
export interface UnavailableResult {
  readonly status: 'unavailable'
  readonly workflow: string
  readonly client: string
  readonly model: string
  /** One line saying why nothing was started. */
  readonly message: string
  /** The reason each candidate could not start. */
  readonly reasons: readonly Unstartable[]
  readonly attempts: readonly AttemptSummary[]
}

/** The prompt a retried or climbed attempt gets: the caller's, and what the last attempt came to. */
export function retryPrompt(original: string, reason: string, detail: string): string {
  const tail = detail.trim().slice(-FAILURE_TAIL_CHARS)
  return `${original}\n\nA previous attempt failed: ${reason}${tail === '' ? '' : `; ${tail}`}`
}

/** One client model a delegation may run on, with what places it among the others. */
export interface Candidate {
  readonly client: string
  readonly model: string
  readonly family: string
  /** The highest tier the catalogue says it handles; null when it names none, which keeps it out of every climb. */
  readonly tier: Tier | null
  readonly costClass: CostClass
  /** The provider a client that runs on one is named with. */
  readonly provider?: string
}

/** The key two candidates are the same model by. */
export function candidateKey(candidate: Pick<Candidate, 'client' | 'model'>): string {
  return modelKey(candidate.client, candidate.model)
}

function tierRank(tier: Tier): number {
  return TIERS.indexOf(tier)
}

/**
 * The model the caller asked for, as a candidate: its tier comes from the
 * catalogue entry, and is null for a model the catalogue does not know.
 */
export function requestedCandidate(config: GatewayConfig, input: RunInput, entry: ModelEntry | undefined, family: string): Candidate {
  const client = config.clients[input.client]
  const provider = providerOf(input)
  return {
    client: input.client,
    costClass: client === undefined ? 'subsidised' : costClassFor(client, provider),
    family,
    model: input.model,
    tier: entry?.tier ?? null,
    ...(input.provider === undefined ? {} : { provider: input.provider }),
  }
}

/** The provider a run reaches its client model on: the one it names, else the one an OpenCode model names. */
function providerOf(input: RunInput): string | undefined {
  return input.provider ?? (input.client === 'opencode' ? openCodeProvider(input.model) : undefined)
}

/**
 * Every catalogue model the delegation may be moved to: one a client that can
 * run a delegation offers, that the data policy permits for what the run
 * carries, that this machine has not disabled, and whose provider the machine
 * allows. A metered-employer model joins the pool only while a live grant
 * covers the workflow, so a client that cannot start is swapped for one of
 * them only when the owner allowed that workflow to spend on it. A client that
 * runs on a provider the caller names is open only to the provider the caller
 * named.
 */
export function candidatePool(
  config: GatewayConfig,
  input: RunInput,
  permissions: readonly ModelPermission[],
  granted: ReadonlySet<string>,
): readonly Candidate[] {
  const permitted = new Set(permissions.filter((permission) => carries(permission, input)).map((permission) => modelKey(permission.client, permission.model)))
  const pool: Candidate[] = []
  for (const entry of config.models) {
    const client = config.clients[entry.client]
    if (client === undefined || !(RUN_CLIENTS as readonly string[]).includes(entry.client)) continue
    if (!permitted.has(modelKey(entry.client, entry.model))) continue
    if ([entry.model, ...(entry.names ?? [])].some((name) => config.disabled.includes(modelKey(entry.client, name)))) continue
    const named = (PROVIDER_CLIENTS as readonly string[]).includes(entry.client)
    if (named && (entry.client !== input.client || input.provider === undefined)) continue
    if (entry.client === 'opencode' && !providerAllowed(entry.model, client.providers)) continue
    const provider = named ? input.provider : entry.client === 'opencode' ? openCodeProvider(entry.model) : undefined
    const costClass = costClassFor(client, provider)
    if (costClass === 'metered-employer' && !granted.has(entry.client)) continue
    pool.push({
      client: entry.client,
      costClass,
      family: entry.family,
      model: entry.model,
      tier: entry.tier,
      ...(named && provider !== undefined ? { provider } : {}),
    })
  }
  return pool
}

function providerAllowed(model: string, allowed: readonly string[] | undefined): boolean {
  try {
    requireAllowedProvider(model, allowed)
    return true
  } catch {
    return false
  }
}

/** Whether the policy lets a delegation with this run's contents reach the model: what the run reads and what the caller marks. */
function carries(permission: ModelPermission, input: RunInput): boolean {
  if (!permission.permitted) return false
  const marks = input.carries ?? { code: false, history: false }
  const code = input.repo === undefined ? marks.code : true
  return !(code && !permission.may_send.code) && !(marks.history && !permission.may_send.history)
}

/**
 * The client models a climb may move to from the one that failed twice: the
 * next stronger subsidised ones, whatever cost class the failed model is in, so
 * a climb never spends money; a tier strictly higher than the failed model's,
 * by tier and then by the catalogue's order. A model with no tier has nowhere
 * to climb from.
 */
export function climbOrder(from: Candidate, pool: readonly Candidate[]): readonly Candidate[] {
  if (from.tier === null) return []
  const above = tierRank(from.tier)
  return pool
    .filter(
      (candidate) =>
        candidateKey(candidate) !== candidateKey(from) &&
        candidate.costClass === 'subsidised' &&
        candidate.tier !== null &&
        tierRank(candidate.tier) > above,
    )
    .map((candidate, index) => ({ candidate, index }))
    .sort((left, right) => tierRank(left.candidate.tier ?? 'trivial') - tierRank(right.candidate.tier ?? 'trivial') || left.index - right.index)
    .map((entry) => entry.candidate)
}

/**
 * The client models a client that cannot start may be swapped for: the same
 * cost class, and a tier at least the wanted model's, the nearest tier first
 * and then the catalogue's order. A model with no tier covers no tier.
 */
export function fallThroughOrder(wanted: Candidate, pool: readonly Candidate[]): readonly Candidate[] {
  if (wanted.tier === null) return []
  const floor = tierRank(wanted.tier)
  return pool
    .filter(
      (candidate) =>
        candidateKey(candidate) !== candidateKey(wanted) &&
        candidate.costClass === wanted.costClass &&
        candidate.tier !== null &&
        tierRank(candidate.tier) >= floor,
    )
    .map((candidate, index) => ({ candidate, index }))
    .sort((left, right) => tierRank(left.candidate.tier ?? 'trivial') - tierRank(right.candidate.tier ?? 'trivial') || left.index - right.index)
    .map((entry) => entry.candidate)
}

/** The run input of an attempt on a candidate: the caller's, on that client model, with the prompt of this attempt. */
export function attemptInput(input: RunInput, candidate: Candidate, prompt: string): RunInput {
  const rest = Object.fromEntries(Object.entries(input).filter(([key]) => key !== 'provider')) as Omit<RunInput, 'provider'>
  return {
    ...rest,
    client: candidate.client,
    model: candidate.model,
    prompt,
    ...(candidate.provider === undefined ? {} : { provider: candidate.provider }),
  }
}
