/**
 * The gateway's configuration: the clients on this machine, the models each
 * one offers, and the settings a delegation is held to.
 *
 * agent-kit ships a defaults file with all of it; this machine's override file
 * carries the same top-level keys, every one optional, and is merged over the
 * defaults field by field. Both are read as JSON and validated here, so a
 * broken file is one error that names what is wrong rather than a surprise
 * later.
 */

export const CONFIG_VERSION = 1

/** What a run on a client costs the owner. */
export const COST_CLASSES = ['subsidised', 'metered-personal', 'metered-employer'] as const
export type CostClass = (typeof COST_CLASSES)[number]

/** How hard a task a model covers. */
export const TIERS = ['trivial', 'routine', 'hard', 'frontier'] as const
export type Tier = (typeof TIERS)[number]

/** Where a client's model list comes from: the client itself, its config, or the declared list. */
export const MODEL_SOURCES = ['live', 'config', 'declared'] as const
export type ModelSource = (typeof MODEL_SOURCES)[number]

/** The spans a plan limit may name; each reaches back from now. */
export const PLAN_WINDOWS = ['5h', 'day', 'week', 'month'] as const
export type PlanWindow = (typeof PLAN_WINDOWS)[number]

/**
 * One window of a client's plan: the dollars it allows in that window, or the
 * runs it allows, never both. A plan that is not billed in dollars names runs.
 */
export type PlanLimit =
  | { readonly window: PlanWindow; readonly limit_usd: number }
  | { readonly window: PlanWindow; readonly limit_runs: number }

/** One client on this machine. */
export interface ClientConfig {
  /** The program that runs it, as it is named on PATH. */
  readonly binary: string
  readonly cost_class: CostClass
  readonly model_source: ModelSource
  /** The model families this client may reach; undefined when it is not restricted. */
  readonly families: readonly string[] | undefined
  /**
   * The cost class of each provider a client's models run on, by provider name;
   * undefined when the client's own cost class covers every model it lists.
   */
  readonly provider_cost_classes: Readonly<Record<string, CostClass>> | undefined
  /**
   * The providers a run may reach through the client, by name: the ones an
   * OpenCode model may name, and the ones a Hermes run may name. Undefined
   * when the client has no such allowlist, which lets a Hermes run on none.
   */
  readonly providers: readonly string[] | undefined
  /**
   * What the client's plan allows, window by window; undefined when the owner
   * has set none, which leaves the client unwatched.
   */
  readonly plan_limits?: readonly PlanLimit[]
}

/** The fields a client has; a machine file that names another is refused, not quietly ignored. */
const CLIENT_FIELDS = [
  'binary',
  'cost_class',
  'model_source',
  'families',
  'provider_cost_classes',
  'providers',
  'plan_limits',
] as const

/** The top-level keys a machine file may name; one it does not is refused, not quietly dropped. */
const OVERRIDE_KEYS = ['version', 'clients', 'disabled', 'models', 'settings'] as const

/** One model a client offers, as the declared list describes it. */
export interface ModelEntry {
  readonly client: string
  readonly model: string
  /**
   * The names the client's own configuration gives this model, where `model`
   * is the alias the client accepts; undefined when the model has no other name.
   */
  readonly names?: readonly string[]
  readonly family: string
  readonly tier: Tier
  readonly notes: string
  /**
   * What the model costs, in USD per million tokens, when the catalogue knows
   * it: the copy of the client's own prices (critique's routing), from which
   * a run's dollars are counted when the client's export prices nothing.
   */
  readonly prices?: ModelPrices
}

/** What one model costs, in USD per million tokens; cached input is prompt the provider served from its cache. */
export interface ModelPrices {
  readonly input: number
  readonly cached_input: number
  readonly output: number
}

/** The dollar caps a metered-personal client is held to. */
export interface MeteredPersonalCaps {
  readonly per_workflow_usd: number
  readonly per_day_usd: number
}

export interface Settings {
  readonly plan_warning_ratio: number
  readonly summary_max_chars: number
  readonly max_parallel_per_client: number
  /** The most delegations one batch plan may hold; a bigger plan is refused before any work. */
  readonly max_batch_delegations: number
  readonly availability_window_minutes: number
  readonly metered_personal_caps: MeteredPersonalCaps
  /**
   * The output tokens a run of each difficulty tier is allowed, which is what
   * a metered run's estimate counts at the model's output price.
   */
  readonly output_allowance_tokens: Readonly<Record<Tier, number>>
}

export interface GatewayConfig {
  readonly version: number
  readonly clients: Readonly<Record<string, ClientConfig>>
  readonly models: readonly ModelEntry[]
  readonly settings: Settings
  /** The `<client>:<model>` keys the owner left out. */
  readonly disabled: readonly string[]
}

export type ModelGatewayErrorCode =
  /** The run named an extra its client cannot apply, such as an MCP server for a client that attaches none. */
  | 'extras-not-supported'
  /**
   * The run asked to switch the client's home servers off, and the home's
   * configuration cannot be read or parsed, so the gateway cannot name them.
   */
  | 'home-config-unreadable'
  | 'invalid-config'
  | 'invalid-input'
  /**
   * The call came from inside a delegation (the delegate's environment holds
   * the gateway's mark), and a delegate may never delegate on.
   */
  | 'nested-delegation'
  /**
   * A batch plan has a problem (a cycle, a dependency on nothing, a model the
   * gateway cannot place or the policy forbids, no grant, or estimates that
   * together do not fit the caps and grants), so nothing was launched.
   */
  | 'plan-refused'
  /**
   * A metered-employer delegation that starts later than its plan was
   * checked has under a minute left on its grant, so it may not start.
   */
  | 'grant-expiring'
  /** The run asked for a client or a mode a later ticket builds (#297). */
  | 'not-supported-yet'
  /** The data policy refuses the run's client model, or what it claims to carry. */
  | 'policy-refused'
  /**
   * The run names a provider this machine does not let the client run on, or
   * one the owner forbade outright (OpenRouter), or an OpenCode model whose
   * provider `clients.opencode.providers` does not allow, as every
   * pay-per-token one is not.
   */
  | 'provider-not-allowed'
  /** git could not read the path the caller named, so the data policy cannot be decided. */
  | 'repository-unreadable'
  | 'unknown-command'
  /** The run names a client the configuration does not declare, so no cap or grant could hold it. */
  | 'unknown-client'
  /** The run named a client model the gateway cannot list now. */
  | 'unknown-model'
  /** A writing run names no repository, and a change needs one. */
  | 'write-needs-repository'
  /** git could not add the throwaway worktree a run without a read-only proof works in, or create a writing run's worktree off its base. */
  | 'worktree-failed'
  /** `workflow end` named a workflow this machine never started. */
  | 'unknown-workflow'
  /** The run named a workflow that ended, so its caps and grants lapsed with it. */
  | 'workflow-ended'
  /**
   * A metered run's client model has no catalogue price or no difficulty tier,
   * so the gateway cannot estimate what it would cost and will not run it
   * uncapped.
   */
  | 'no-estimate'
  /** A metered run's estimate does not fit the per-workflow or the per-day cap, or its grant's cap. */
  | 'cap-exceeded'
  /**
   * A metered-employer run names a workflow no live grant covers: the owner
   * has not allowed that workflow to spend nedap money, so the run is refused
   * rather than run uncapped.
   */
  | 'no-grant'
  /**
   * `grant write` names a workflow and client a live grant already covers: the
   * owner re-approves only after that grant lapsed, so a second grant cannot
   * quietly double the cap.
   */
  | 'grant-exists'
  /** The grants file lies inside a git repository, and a grant is never written into one. */
  | 'grants-in-repository'
  /** `grant write` could not read the ledger, so a standing grant cannot be ruled out: it fails closed. */
  | 'ledger-unreadable'
  /**
   * A subsidised client is at 100% of one of its plan windows, so an engine run
   * that asks the gateway first is refused rather than started to fail.
   */
  | 'unable-to-start'

export class ModelGatewayError extends Error {
  constructor(
    readonly code: ModelGatewayErrorCode,
    message: string,
  ) {
    super(message)
    this.name = 'ModelGatewayError'
  }
}

export type JsonObject = Record<string, unknown>

export function isObject(value: unknown): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Reads parsed JSON as the shipped defaults, or says what is wrong with it. */
export function parseDefaults(raw: unknown): GatewayConfig {
  if (!isObject(raw)) throw invalid('the defaults must be a JSON object')
  if (raw.version !== CONFIG_VERSION) throw invalid(`version must be ${String(CONFIG_VERSION)}`)
  if (!isObject(raw.clients)) throw invalid('clients must be an object keyed by client id')
  const clients = Object.fromEntries(
    Object.entries(raw.clients).map(([id, value]) => {
      if (!isObject(value)) throw invalid(`clients.${id} must be an object`)
      return [id, parseClient(id, value, undefined)]
    }),
  )
  if (!Array.isArray(raw.models)) throw invalid('models must be a list')
  const models = raw.models.map((value, index) => parseModelEntry(value, index, undefined))
  if (!isObject(raw.settings)) throw invalid('settings must be an object')
  return {
    clients,
    disabled: parseDisabled(raw.disabled),
    models,
    settings: parseSettings(raw.settings),
    version: CONFIG_VERSION,
  }
}

/**
 * This machine's override merged over the defaults: a client field replaces
 * that field, a model entry matching `client` and `model` (or one of its
 * `names`) replaces its tier, notes or family, a new model entry adds a declared model, a setting replaces
 * that setting (deeply for the caps), and a `disabled` list replaces the list.
 */
export function applyOverride(config: GatewayConfig, raw: unknown): GatewayConfig {
  if (!isObject(raw)) throw invalid('the override must be a JSON object')
  refuseUnknownFields(raw, 'the override', OVERRIDE_KEYS, 'a key the override has')
  if (raw.version !== undefined && raw.version !== CONFIG_VERSION) throw invalid(`version must be ${String(CONFIG_VERSION)}`)
  return {
    clients: mergeClients(config.clients, raw.clients),
    disabled: raw.disabled === undefined ? config.disabled : parseDisabled(raw.disabled),
    models: mergeModels(config.models, raw.models),
    settings: mergeSettings(config.settings, raw.settings),
    version: CONFIG_VERSION,
  }
}

function mergeClients(base: Readonly<Record<string, ClientConfig>>, raw: unknown): Readonly<Record<string, ClientConfig>> {
  if (raw === undefined) return base
  if (!isObject(raw)) throw invalid('clients must be an object keyed by client id')
  const merged: Record<string, ClientConfig> = { ...base }
  for (const [id, value] of Object.entries(raw)) {
    if (!isObject(value)) throw invalid(`clients.${id} must be an object`)
    merged[id] = parseClient(id, value, merged[id])
  }
  return merged
}

/**
 * One client, every field it does not name taken from `base`; a new client
 * must name all three. `provider_cost_classes` merges key by key, so an
 * override that names one provider keeps the cost class of the others;
 * `plan_limits` replaces the list, so an override that names one window
 * replaces the client's whole plan.
 */
function parseClient(id: string, raw: JsonObject, base: ClientConfig | undefined): ClientConfig {
  const where = `clients.${id}`
  refuseUnknownFields(raw, where, CLIENT_FIELDS, 'a field a client has')
  const binary = raw.binary === undefined ? base?.binary : requireString(raw.binary, `${where}.binary`)
  const costClass = raw.cost_class === undefined ? base?.cost_class : requireOneOf(raw.cost_class, COST_CLASSES, `${where}.cost_class`)
  const modelSource =
    raw.model_source === undefined ? base?.model_source : requireOneOf(raw.model_source, MODEL_SOURCES, `${where}.model_source`)
  const families = raw.families === undefined ? base?.families : requireStrings(raw.families, `${where}.families`)
  const providerCostClasses =
    raw.provider_cost_classes === undefined
      ? base?.provider_cost_classes
      : {
          ...base?.provider_cost_classes,
          ...parseProviderCostClasses(raw.provider_cost_classes, `${where}.provider_cost_classes`),
        }
  const providers = raw.providers === undefined ? base?.providers : requireStrings(raw.providers, `${where}.providers`)
  const planLimits = raw.plan_limits === undefined ? base?.plan_limits : parsePlanLimits(raw.plan_limits, `${where}.plan_limits`)
  if (binary === undefined || costClass === undefined || modelSource === undefined) {
    throw invalid(`${where} is new, so it needs binary, cost_class and model_source`)
  }
  return {
    binary,
    cost_class: costClass,
    families,
    model_source: modelSource,
    provider_cost_classes: providerCostClasses,
    providers,
    ...(planLimits === undefined ? {} : { plan_limits: planLimits }),
  }
}

/** The windows of a client's plan, each naming its span and either its dollars or its runs. */
function parsePlanLimits(raw: unknown, where: string): readonly PlanLimit[] {
  if (!Array.isArray(raw)) throw invalid(`${where} must be a list of windows`)
  return raw.map((value, index) => {
    const at = `${where}[${String(index)}]`
    if (!isObject(value)) throw invalid(`${at} must be an object`)
    const window = requireOneOf(value.window, PLAN_WINDOWS, `${at}.window`)
    const dollars = value.limit_usd
    const runs = value.limit_runs
    if ((dollars === undefined) === (runs === undefined)) {
      throw invalid(`${at} must name exactly one of limit_usd and limit_runs`)
    }
    if (dollars !== undefined) {
      const limit = requireNumber(dollars, `${at}.limit_usd`)
      if (limit <= 0) throw invalid(`${at}.limit_usd must be above 0`)
      return { limit_usd: limit, window }
    }
    const limit = requireNumber(runs, `${at}.limit_runs`)
    if (!Number.isInteger(limit) || limit <= 0) throw invalid(`${at}.limit_runs must be a whole number above 0`)
    return { limit_runs: limit, window }
  })
}

/** Refuses a key a file may not name, so a machine file that misspells one learns it, rather than losing it. */
function refuseUnknownFields(raw: JsonObject, where: string, allowed: readonly string[], what: string): void {
  const unknown = Object.keys(raw).find((key) => !allowed.includes(key))
  if (unknown !== undefined) {
    throw invalid(`${where}.${unknown} is not ${what}; choose from ${allowed.join(', ')}`)
  }
}

/** The cost class of each provider a client's models run on, by provider name. */
function parseProviderCostClasses(raw: unknown, where: string): Readonly<Record<string, CostClass>> {
  if (!isObject(raw)) throw invalid(`${where} must be an object keyed by provider name`)
  return Object.fromEntries(
    Object.entries(raw).map(([provider, value]) => [provider, requireOneOf(value, COST_CLASSES, `${where}.${provider}`)]),
  )
}

function mergeModels(base: readonly ModelEntry[], raw: unknown): readonly ModelEntry[] {
  if (raw === undefined) return base
  if (!Array.isArray(raw)) throw invalid('models must be a list')
  const merged = [...base]
  for (const [index, value] of raw.entries()) {
    if (!isObject(value)) throw invalid(`models[${String(index)}] must be an object`)
    const client = requireString(value.client, `models[${String(index)}].client`)
    const model = requireString(value.model, `models[${String(index)}].model`)
    const at = merged.findIndex((entry) => entry.client === client && (entry.model === model || (entry.names ?? []).includes(model)))
    const target = at === -1 ? undefined : merged[at]
    const entry = parseModelEntry(target === undefined ? value : { ...value, model: target.model }, index, target)
    if (at === -1) merged.push(entry)
    else merged[at] = entry
  }
  return merged
}

/** One model entry, every field it does not name taken from `base`; a new entry must name all five. */
function parseModelEntry(raw: unknown, index: number, base: ModelEntry | undefined): ModelEntry {
  const where = `models[${String(index)}]`
  if (!isObject(raw)) throw invalid(`${where} must be an object`)
  const client = requireString(raw.client, `${where}.client`)
  const model = requireString(raw.model, `${where}.model`)
  const family = raw.family === undefined ? base?.family : requireString(raw.family, `${where}.family`)
  const tier = raw.tier === undefined ? base?.tier : requireOneOf(raw.tier, TIERS, `${where}.tier`)
  const notes = raw.notes === undefined ? base?.notes : requireString(raw.notes, `${where}.notes`)
  const prices = raw.prices === undefined ? base?.prices : parseModelPrices(raw.prices, `${where}.prices`)
  if (family === undefined || tier === undefined || notes === undefined) {
    throw invalid(`${where} (${client}:${model}) is new, so it needs family, tier and notes`)
  }
  const names = raw.names === undefined ? base?.names : requireStrings(raw.names, `${where}.names`)
  return {
    client,
    family,
    model,
    ...(names === undefined ? {} : { names }),
    notes,
    ...(prices === undefined ? {} : { prices }),
    tier,
  }
}

/** The catalogue's copy of a model's prices: input and output above 0, cached input at least 0. */
function parseModelPrices(raw: unknown, where: string): ModelPrices {
  if (!isObject(raw)) throw invalid(`${where} must be an object`)
  const input = requireNumber(raw.input, `${where}.input`)
  const output = requireNumber(raw.output, `${where}.output`)
  const cachedInput = requireNumber(raw.cached_input, `${where}.cached_input`)
  if (input <= 0 || output <= 0) throw invalid(`${where}.input and ${where}.output must be above 0`)
  if (cachedInput < 0) throw invalid(`${where}.cached_input must be at least 0`)
  return { cached_input: cachedInput, input, output }
}

function mergeSettings(base: Settings, raw: unknown): Settings {
  if (raw === undefined) return base
  if (!isObject(raw)) throw invalid('settings must be an object')
  return {
    availability_window_minutes: numberField(raw, 'availability_window_minutes', base.availability_window_minutes),
    max_batch_delegations: countField(raw, 'max_batch_delegations', base.max_batch_delegations),
    max_parallel_per_client: countField(raw, 'max_parallel_per_client', base.max_parallel_per_client),
    metered_personal_caps: mergeCaps(base.metered_personal_caps, raw.metered_personal_caps),
    output_allowance_tokens: mergeAllowances(base.output_allowance_tokens, raw.output_allowance_tokens),
    plan_warning_ratio: ratioField(raw, 'plan_warning_ratio', base.plan_warning_ratio),
    summary_max_chars: numberField(raw, 'summary_max_chars', base.summary_max_chars),
  }
}

function parseSettings(raw: JsonObject): Settings {
  const caps = raw.metered_personal_caps
  if (!isObject(caps)) throw invalid('settings.metered_personal_caps must be an object')
  const allowances = raw.output_allowance_tokens
  if (!isObject(allowances)) throw invalid('settings.output_allowance_tokens must be an object keyed by tier')
  return {
    availability_window_minutes: requireNumber(raw.availability_window_minutes, 'settings.availability_window_minutes'),
    max_batch_delegations: requireCount(raw.max_batch_delegations, 'settings.max_batch_delegations'),
    max_parallel_per_client: requireCount(raw.max_parallel_per_client, 'settings.max_parallel_per_client'),
    metered_personal_caps: {
      per_day_usd: requireNumber(caps.per_day_usd, 'settings.metered_personal_caps.per_day_usd'),
      per_workflow_usd: requireNumber(caps.per_workflow_usd, 'settings.metered_personal_caps.per_workflow_usd'),
    },
    output_allowance_tokens: {
      frontier: requireAllowance(allowances, 'frontier'),
      hard: requireAllowance(allowances, 'hard'),
      routine: requireAllowance(allowances, 'routine'),
      trivial: requireAllowance(allowances, 'trivial'),
    },
    plan_warning_ratio: requireRatio(raw.plan_warning_ratio, 'settings.plan_warning_ratio'),
    summary_max_chars: requireNumber(raw.summary_max_chars, 'settings.summary_max_chars'),
  }
}

/** The output tokens one tier's run is allowed, which a machine file may override tier by tier. */
function requireAllowance(raw: JsonObject, tier: Tier): number {
  const where = `settings.output_allowance_tokens.${tier}`
  const value = requireNumber(raw[tier], where)
  if (!Number.isInteger(value) || value <= 0) throw invalid(`${where} must be a whole number of tokens above 0`)
  return value
}

function mergeAllowances(base: Readonly<Record<Tier, number>>, raw: unknown): Readonly<Record<Tier, number>> {
  if (raw === undefined) return base
  if (!isObject(raw)) throw invalid('settings.output_allowance_tokens must be an object keyed by tier')
  return {
    frontier: raw.frontier === undefined ? base.frontier : requireAllowance(raw, 'frontier'),
    hard: raw.hard === undefined ? base.hard : requireAllowance(raw, 'hard'),
    routine: raw.routine === undefined ? base.routine : requireAllowance(raw, 'routine'),
    trivial: raw.trivial === undefined ? base.trivial : requireAllowance(raw, 'trivial'),
  }
}

function mergeCaps(base: MeteredPersonalCaps, raw: unknown): MeteredPersonalCaps {
  if (raw === undefined) return base
  if (!isObject(raw)) throw invalid('settings.metered_personal_caps must be an object')
  return {
    per_day_usd: numberField(raw, 'per_day_usd', base.per_day_usd, 'settings.metered_personal_caps.'),
    per_workflow_usd: numberField(raw, 'per_workflow_usd', base.per_workflow_usd, 'settings.metered_personal_caps.'),
  }
}

function numberField(raw: JsonObject, name: string, base: number, prefix = 'settings.'): number {
  return raw[name] === undefined ? base : requireNumber(raw[name], `${prefix}${name}`)
}

/** A setting that counts things: a whole number of at least 1. */
function countField(raw: JsonObject, name: string, base: number): number {
  return raw[name] === undefined ? base : requireCount(raw[name], `settings.${name}`)
}

/** A setting that is a share of a whole: a ratio from 0 to 1. */
function ratioField(raw: JsonObject, name: string, base: number): number {
  return raw[name] === undefined ? base : requireRatio(raw[name], `settings.${name}`)
}

function requireCount(value: unknown, where: string): number {
  const count = requireNumber(value, where)
  if (!Number.isInteger(count) || count < 1) throw invalid(`${where} must be a whole number of at least 1`)
  return count
}

/** The share of a plan that may be used before the gateway warns: a ratio from 0 to 1, not a percentage. */
function requireRatio(value: unknown, where: string): number {
  const ratio = requireNumber(value, where)
  if (ratio < 0 || ratio > 1) throw invalid(`${where} must be between 0 and 1`)
  return ratio
}

function parseDisabled(raw: unknown): readonly string[] {
  if (raw === undefined) return []
  return requireStrings(raw, 'disabled')
}

function requireStrings(value: unknown, where: string): readonly string[] {
  if (!Array.isArray(value)) throw invalid(`${where} must be a list of non-empty strings`)
  const items: readonly unknown[] = value
  const strings: string[] = []
  for (const item of items) {
    if (typeof item !== 'string' || item === '') throw invalid(`${where} must be a list of non-empty strings`)
    strings.push(item)
  }
  return strings
}

function requireString(value: unknown, where: string): string {
  if (typeof value !== 'string' || value === '') throw invalid(`${where} must be a non-empty string`)
  return value
}

function requireNumber(value: unknown, where: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw invalid(`${where} must be a number`)
  return value
}

function requireOneOf<T extends string>(value: unknown, allowed: readonly T[], where: string): T {
  if (typeof value !== 'string' || !(allowed as readonly string[]).includes(value)) {
    throw invalid(`${where} must be one of ${allowed.join(', ')}`)
  }
  return value as T
}

function invalid(message: string): ModelGatewayError {
  return new ModelGatewayError('invalid-config', message)
}
