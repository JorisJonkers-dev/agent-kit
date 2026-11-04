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
}

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
  readonly availability_window_minutes: number
  readonly metered_personal_caps: MeteredPersonalCaps
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
  | 'invalid-config'
  | 'invalid-input'
  /**
   * The call came from inside a delegation (the delegate's environment holds
   * the gateway's mark), and a delegate may never delegate on.
   */
  | 'nested-delegation'
  /** The run asked for a client or a mode a later ticket builds (#299, #297). */
  | 'not-supported-yet'
  /** The data policy refuses the run's client model, or what it claims to carry. */
  | 'policy-refused'
  /** git could not read the path the caller named, so the data policy cannot be decided. */
  | 'repository-unreadable'
  /** The client model has no read-only proof that passed on this machine (#299 runs such clients in a worktree). */
  | 'unproven-read-only'
  | 'unknown-command'
  /** The run named a client model the gateway cannot list now. */
  | 'unknown-model'

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
 * override that names one provider keeps the cost class of the others.
 */
function parseClient(id: string, raw: JsonObject, base: ClientConfig | undefined): ClientConfig {
  const where = `clients.${id}`
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
  if (binary === undefined || costClass === undefined || modelSource === undefined) {
    throw invalid(`${where} is new, so it needs binary, cost_class and model_source`)
  }
  return { binary, cost_class: costClass, families, model_source: modelSource, provider_cost_classes: providerCostClasses }
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
    max_parallel_per_client: numberField(raw, 'max_parallel_per_client', base.max_parallel_per_client),
    metered_personal_caps: mergeCaps(base.metered_personal_caps, raw.metered_personal_caps),
    plan_warning_ratio: numberField(raw, 'plan_warning_ratio', base.plan_warning_ratio),
    summary_max_chars: numberField(raw, 'summary_max_chars', base.summary_max_chars),
  }
}

function parseSettings(raw: JsonObject): Settings {
  const caps = raw.metered_personal_caps
  if (!isObject(caps)) throw invalid('settings.metered_personal_caps must be an object')
  return {
    availability_window_minutes: requireNumber(raw.availability_window_minutes, 'settings.availability_window_minutes'),
    max_parallel_per_client: requireNumber(raw.max_parallel_per_client, 'settings.max_parallel_per_client'),
    metered_personal_caps: {
      per_day_usd: requireNumber(caps.per_day_usd, 'settings.metered_personal_caps.per_day_usd'),
      per_workflow_usd: requireNumber(caps.per_workflow_usd, 'settings.metered_personal_caps.per_workflow_usd'),
    },
    plan_warning_ratio: requireNumber(raw.plan_warning_ratio, 'settings.plan_warning_ratio'),
    summary_max_chars: requireNumber(raw.summary_max_chars, 'settings.summary_max_chars'),
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
